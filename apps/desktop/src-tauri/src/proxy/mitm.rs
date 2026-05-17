use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::ServerConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;

use super::forward::detect_provider;
use super::intercept::{handle_anthropic, handle_openai_compatible, InterceptState};

type CertEntry = (Vec<u8>, Vec<u8>);

pub struct MitmState {
    ca_cert: rcgen::Certificate,
    ca_key_pem: String,
    cert_cache: Mutex<HashMap<String, CertEntry>>,
}

fn sessiongraph_dir() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()?
    } else {
        std::env::var("HOME").ok()?
    };
    Some(PathBuf::from(home).join(".sessiongraph"))
}

pub fn init_mitm() -> Result<Arc<MitmState>, String> {
    let dir = sessiongraph_dir().ok_or("Cannot determine home directory")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create dir: {e}"))?;

    let ca_cert_path = dir.join("mitm-ca.crt");
    let ca_key_path = dir.join("mitm-ca.key");

    let (ca_cert, ca_key_pem) = if ca_cert_path.exists() && ca_key_path.exists() {
        let _ca_pem =
            fs::read_to_string(&ca_cert_path).map_err(|e| format!("Read CA cert: {e}"))?;
        let ca_key_pem =
            fs::read_to_string(&ca_key_path).map_err(|e| format!("Read CA key: {e}"))?;
        let ca_key = KeyPair::from_pem(&ca_key_pem).map_err(|e| format!("Parse CA key: {e}"))?;
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_cert = params.self_signed(&ca_key).map_err(|e| e.to_string())?;
        (ca_cert, ca_key_pem)
    } else {
        let ca_key = KeyPair::generate().map_err(|e| format!("Gen CA key: {e}"))?;
        let mut params =
            CertificateParams::new(Vec::<String>::new()).map_err(|e| format!("CA params: {e}"))?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::OrganizationName, "SessionGraph");
        params
            .distinguished_name
            .push(DnType::CommonName, "SessionGraph MITM CA");
        let ca_cert = params
            .self_signed(&ca_key)
            .map_err(|e| format!("CA self-sign: {e}"))?;
        let ca_pem = ca_cert.pem();
        let ca_key_pem = ca_key.serialize_pem();
        fs::write(&ca_cert_path, &ca_pem).map_err(|e| format!("Write CA cert: {e}"))?;
        fs::write(&ca_key_path, &ca_key_pem).map_err(|e| format!("Write CA key: {e}"))?;
        tracing::info!("MITM CA generated at {}", ca_cert_path.display());
        (ca_cert, ca_key_pem)
    };

    if cfg!(windows) {
        install_ca_windows(&ca_cert_path);
    }

    Ok(Arc::new(MitmState {
        ca_cert,
        ca_key_pem,
        cert_cache: Mutex::new(HashMap::new()),
    }))
}

fn install_ca_windows(ca_cert_path: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let path_str = ca_cert_path.to_string_lossy();
    let cmd = format!(
        "Import-Certificate -FilePath '{}' -CertStoreLocation Cert:\\CurrentUser\\Root",
        path_str.replace('\'', "''")
    );
    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) if o.status.success() => tracing::info!("MITM CA installed"),
        Ok(o) => tracing::warn!("CA install exit code {}", o.status.code().unwrap_or(-1)),
        Err(e) => tracing::warn!("CA install failed: {e}"),
    }
}

async fn get_or_create_cert(state: &MitmState, host: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut cache = state.cert_cache.lock().await;
    if let Some(cached) = cache.get(host) {
        return Ok(cached.clone());
    }
    let ca_key = KeyPair::from_pem(&state.ca_key_pem).map_err(|e| format!("CA key: {e}"))?;
    let server_key = KeyPair::generate().map_err(|e| format!("Server key: {e}"))?;
    let mut params =
        CertificateParams::new(vec![host.to_string()]).map_err(|e| format!("Params: {e}"))?;
    params.is_ca = IsCa::ExplicitNoCa;
    params.distinguished_name.push(DnType::CommonName, host);
    let server_cert = params
        .signed_by(&server_key, &state.ca_cert, &ca_key)
        .map_err(|e| format!("Sign: {e}"))?;
    let cert_der = server_cert.der().to_vec();
    let key_der = server_key.serialize_der();
    cache.insert(host.to_string(), (cert_der.clone(), key_der.clone()));
    Ok((cert_der, key_der))
}

async fn tls_config(state: &MitmState, host: &str) -> Result<Arc<ServerConfig>, String> {
    let (cert_der, key_der) = get_or_create_cert(state, host).await?;
    let certs = vec![CertificateDer::from(cert_der)];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map(Arc::new)
        .map_err(|e| format!("TLS config: {e}"))
}

pub async fn handle_connect(
    upgraded: hyper::upgrade::Upgraded,
    host: &str,
    port: u16,
    intercept: Arc<InterceptState>,
    mitm: Arc<MitmState>,
) {
    let tls_cfg = match tls_config(&mitm, host).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("TLS config fail for {host}: {e}");
            return;
        }
    };

    let upstream = match TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Upstream connect fail {host}:{port}: {e}");
            return;
        }
    };

    let io = hyper_util::rt::TokioIo::new(upgraded);
    let acceptor = TlsAcceptor::from(tls_cfg);
    let mut client_tls = match acceptor.accept(io).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("TLS accept fail for {host}: {e}");
            return;
        }
    };

    if let Err(e) = handle_http(&mut client_tls, intercept, host, port, upstream).await {
        tracing::debug!("MITM error: {e}");
    }
}

/// Read one HTTP/1.1 request, route through pipeline, write response back.
async fn handle_http(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin + Send),
    intercept: Arc<InterceptState>,
    _host: &str,
    _port: u16,
    _upstream: TcpStream,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request_line = read_line(stream).await?;
    if request_line.is_empty() {
        return Ok(());
    }

    let parts: Vec<&str> = request_line.split(' ').collect();
    if parts.len() < 3 {
        let empty_headers = HeaderMap::new();
        write_response(stream, StatusCode::BAD_REQUEST, &empty_headers, &[]).await?;
        return Ok(());
    }
    let method: Method = parts[0].parse().unwrap_or(Method::GET);
    let _path = parts[1];

    let mut headers = HeaderMap::new();
    let mut content_length: Option<usize> = None;
    loop {
        let line = read_line(stream).await?;
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = v.trim();
            if key == "content-length" {
                content_length = val.parse::<usize>().ok();
            }
            if let (Ok(k), Ok(v)) = (key.parse::<HeaderName>(), val.parse::<HeaderValue>()) {
                headers.insert(k, v);
            }
        }
    }

    let body_bytes = if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;
        buf
    } else {
        Vec::new()
    };

    let json_body: serde_json::Value = if body_bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null)
    };

    let provider = detect_provider(&headers);
    let response = match (method, _path, &provider) {
        (Method::POST, "/v1/messages", _) => handle_anthropic(&intercept, &headers, json_body)
            .await
            .unwrap_or_else(|e| error_response(&format!("proxy: {:?}", e))),
        (Method::POST, "/v1/chat/completions", _) => {
            handle_openai_compatible(&intercept, &headers, json_body, None)
                .await
                .unwrap_or_else(|e| error_response(&format!("proxy: {:?}", e)))
        }
        _ => {
            let mut r = Response::new(Body::from("Not Found"));
            *r.status_mut() = StatusCode::NOT_FOUND;
            r
        }
    };

    let (parts, body) = response.into_parts();
    let body_bytes = body
        .collect()
        .await
        .map(|c| c.to_bytes().to_vec())
        .unwrap_or_default();
    write_response(stream, parts.status, &parts.headers, &body_bytes).await?;
    Ok(())
}

fn error_response(msg: &str) -> Response {
    let mut r = Response::new(Body::from(serde_json::json!({"error": msg}).to_string()));
    *r.status_mut() = StatusCode::BAD_GATEWAY;
    r
}

/// Read a line (up to \n) from a stream, byte by byte.
async fn read_line(
    stream: &mut (impl AsyncReadExt + Unpin),
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = String::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte).await? {
            0 => return Ok(buf),
            _ => {
                if byte[0] == b'\n' {
                    return Ok(buf.trim_end().to_string());
                }
                buf.push(byte[0] as char);
            }
        }
    }
}

/// Write an HTTP/1.1 response to a stream.
async fn write_response(
    stream: &mut (impl AsyncWriteExt + Unpin + Send),
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let reason = status.canonical_reason().unwrap_or("Unknown");
    stream
        .write_all(format!("HTTP/1.1 {} {}\r\n", status.as_u16(), reason).as_bytes())
        .await?;
    for (name, value) in headers.iter() {
        if name.as_str().eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        if let Ok(v) = value.to_str() {
            stream
                .write_all(format!("{}: {}\r\n", name, v).as_bytes())
                .await?;
        }
    }
    stream
        .write_all(format!("Content-Length: {}\r\n", body.len()).as_bytes())
        .await?;
    stream.write_all(b"\r\n").await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}
