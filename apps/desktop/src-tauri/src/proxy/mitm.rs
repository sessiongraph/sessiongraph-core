use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures::StreamExt as FuturesStreamExt;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::ServerConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

    let io = hyper_util::rt::TokioIo::new(upgraded);
    let acceptor = TlsAcceptor::from(tls_cfg);
    let mut client_tls = match acceptor.accept(io).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("TLS accept fail for {host}: {e}");
            return;
        }
    };

    // Loop: serve all requests on this persistent connection.
    // HTTP/1.1 keep-alive means many requests can arrive on a single CONNECT tunnel.
    // The upstream host/port are passed so unknown paths can be forwarded transparently.
    loop {
        match handle_one_request(&mut client_tls, intercept.clone(), host, port).await {
            Ok(keep_alive) => {
                if !keep_alive {
                    break;
                }
            }
            Err(e) => {
                // EOF or connection reset are normal — don't log as errors
                let msg = e.to_string().to_lowercase();
                if !msg.contains("eof")
                    && !msg.contains("reset")
                    && !msg.contains("broken pipe")
                    && !msg.contains("connection reset")
                {
                    tracing::debug!("MITM connection closed: {e}");
                }
                break;
            }
        }
    }
}

/// Read and handle one HTTP/1.1 request from the MITM stream.
/// Returns Ok(true) to continue reading (keep-alive), Ok(false) to close.
async fn handle_one_request(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin + Send),
    intercept: Arc<InterceptState>,
    upstream_host: &str,
    upstream_port: u16,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let request_line = read_line(stream).await?;
    if request_line.is_empty() {
        return Ok(false);
    }

    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        let empty_headers = HeaderMap::new();
        write_response(
            stream,
            StatusCode::BAD_REQUEST,
            &empty_headers,
            b"Bad Request",
        )
        .await?;
        return Ok(false);
    }
    let method: Method = parts[0].parse().unwrap_or(Method::GET);
    let raw_path = parts[1];
    let path = raw_path.split('?').next().unwrap_or(raw_path);

    tracing::debug!("MITM request: {} {} (host={})", method, path, upstream_host);

    let mut headers = HeaderMap::new();
    let mut content_length: Option<usize> = None;
    let mut is_chunked = false;
    let mut connection_close = false;

    loop {
        let line = read_line(stream).await?;
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = v.trim();
            match key.as_str() {
                "content-length" => {
                    content_length = val.parse::<usize>().ok();
                }
                "transfer-encoding" => {
                    is_chunked = val.to_lowercase().contains("chunked");
                }
                "connection" => {
                    connection_close = val.to_lowercase().contains("close");
                }
                _ => {}
            }
            if let (Ok(hk), Ok(hv)) = (key.parse::<HeaderName>(), val.parse::<HeaderValue>()) {
                headers.insert(hk, hv);
            }
        }
    }

    // Read request body — handles both Content-Length and chunked encoding
    let body_bytes = if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;
        buf
    } else if is_chunked {
        read_chunked_body(stream).await?
    } else {
        Vec::new()
    };

    let json_body: serde_json::Value = if body_bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null)
    };

    let _provider = detect_provider(&headers);
    let response = match (method.clone(), path) {
        (Method::POST, "/v1/messages") => handle_anthropic(&intercept, &headers, json_body)
            .await
            .unwrap_or_else(|e| error_response(&format!("proxy: {:?}", e))),
        (Method::POST, "/v1/chat/completions") => {
            handle_openai_compatible(&intercept, &headers, json_body, None)
                .await
                .unwrap_or_else(|e| error_response(&format!("proxy: {:?}", e)))
        }
        _ => {
            // Unknown path — forward transparently to the upstream over TLS.
            // This handles Gemini, Vertex AI, and any other API that goes through
            // the PAC file but isn't intercepted (we observe but don't modify).
            tracing::debug!(
                "MITM transparent forward: {} {}://{}:{}{}",
                method,
                if upstream_port == 443 {
                    "https"
                } else {
                    "http"
                },
                upstream_host,
                upstream_port,
                raw_path,
            );
            return forward_to_upstream(
                stream,
                upstream_host,
                upstream_port,
                &request_line,
                &headers,
                &body_bytes,
                connection_close,
            )
            .await;
        }
    };

    // Check if the response itself requests connection close
    let resp_connection_close = response
        .headers()
        .get("connection")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_lowercase().contains("close"))
        .unwrap_or(false);

    // Stream the response body — do NOT buffer it all before writing.
    // LLM APIs return SSE streams that may take 30–120s to complete.
    // Buffering the whole body would cause the client to time out.
    let (resp_parts, resp_body) = response.into_parts();
    stream_response(stream, resp_parts.status, &resp_parts.headers, resp_body).await?;

    // Continue serving requests unless either side requested close
    let keep_alive = !connection_close && !resp_connection_close;
    Ok(keep_alive)
}

fn error_response(msg: &str) -> Response {
    let mut r = Response::new(Body::from(serde_json::json!({"error": msg}).to_string()));
    *r.status_mut() = StatusCode::BAD_GATEWAY;
    r
}

/// Forward a request to the upstream over HTTPS using reqwest and relay the response.
/// Used for paths the proxy doesn't intercept (e.g. Gemini, Vertex AI).
/// This is observe-only — we don't modify the request or response.
/// Returns Ok(false) to close the connection after forwarding.
#[allow(clippy::too_many_arguments)]
async fn forward_to_upstream(
    client: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin + Send),
    host: &str,
    port: u16,
    request_line: &str,
    headers: &HeaderMap,
    body: &[u8],
    _connection_close: bool,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Reconstruct the full URL from the request line (e.g. "POST /v1beta/models/...:generate HTTP/1.1")
    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    let path = parts.get(1).copied().unwrap_or("/");
    let scheme = if port == 443 { "https" } else { "http" };
    let url = format!("{}://{}:{}{}", scheme, host, port, path);

    let method_str = parts.first().copied().unwrap_or("POST");
    let method =
        reqwest::Method::from_bytes(method_str.as_bytes()).unwrap_or(reqwest::Method::POST);

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .no_proxy()
        .build()?;

    let mut req = http_client.request(method, &url);
    // Forward original headers (skip hop-by-hop)
    let skip = [
        "host",
        "connection",
        "keep-alive",
        "transfer-encoding",
        "te",
        "upgrade",
    ];
    for (name, value) in headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if !skip.contains(&name_lower.as_str()) {
            req = req.header(name.as_str(), value.as_bytes());
        }
    }
    if !body.is_empty() {
        req = req.body(body.to_vec());
    }

    let upstream_response = req.send().await?;
    let status = upstream_response.status().as_u16();
    let reason = upstream_response
        .status()
        .canonical_reason()
        .unwrap_or("Unknown");

    // Write status line and headers
    client
        .write_all(format!("HTTP/1.1 {} {}\r\n", status, reason).as_bytes())
        .await?;

    let upstream_headers = upstream_response.headers().clone();
    let skip_resp = [
        "transfer-encoding",
        "content-length",
        "connection",
        "keep-alive",
    ];
    for (name, value) in upstream_headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if skip_resp.contains(&name_lower.as_str()) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            client
                .write_all(format!("{}: {}\r\n", name, v).as_bytes())
                .await?;
        }
    }
    // Use chunked encoding so we can stream without buffering
    client
        .write_all(b"Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
        .await?;

    // Stream response body chunks to the client
    let mut stream = upstream_response.bytes_stream();
    while let Some(chunk_result) = FuturesStreamExt::next(&mut stream).await {
        match chunk_result {
            Ok(chunk) if !chunk.is_empty() => {
                client
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await?;
                client.write_all(&chunk).await?;
                client.write_all(b"\r\n").await?;
                client.flush().await?;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!("Upstream stream error in forward: {e}");
                break;
            }
        }
    }
    client.write_all(b"0\r\n\r\n").await?;
    client.flush().await?;

    Ok(false) // close after transparent forward — connection state is undefined
}

/// Read a chunked transfer-encoding body from the stream.
async fn read_chunked_body(
    stream: &mut (impl AsyncReadExt + Unpin),
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut body = Vec::new();
    loop {
        // Read chunk size line (hex)
        let size_line = read_line(stream).await?;
        // Strip chunk extensions (e.g. "1a;ext=val" → "1a")
        let size_str = size_line.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(size_str, 16).unwrap_or(0);
        if chunk_size == 0 {
            // Final chunk — consume trailing CRLF
            let _ = read_line(stream).await;
            break;
        }
        let mut chunk = vec![0u8; chunk_size];
        stream.read_exact(&mut chunk).await?;
        body.extend_from_slice(&chunk);
        // Consume trailing CRLF after chunk data
        let _ = read_line(stream).await;
    }
    Ok(body)
}

/// Write an HTTP/1.1 response, streaming the body chunk-by-chunk using
/// chunked transfer encoding so the client sees tokens as they arrive.
/// This is critical for SSE/streaming LLM responses — buffering the whole
/// body causes client timeouts on long generations.
async fn stream_response(
    stream: &mut (impl AsyncWriteExt + Unpin + Send),
    status: StatusCode,
    headers: &HeaderMap,
    body: Body,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let reason = status.canonical_reason().unwrap_or("Unknown");
    stream
        .write_all(format!("HTTP/1.1 {} {}\r\n", status.as_u16(), reason).as_bytes())
        .await?;

    // Forward headers, skipping hop-by-hop and rewriting transfer encoding
    // to chunked so we can stream without knowing the total length upfront.
    let skip = [
        "transfer-encoding",
        "content-length",
        "connection",
        "keep-alive",
    ];
    for (name, value) in headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if skip.contains(&name_lower.as_str()) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            stream
                .write_all(format!("{}: {}\r\n", name, v).as_bytes())
                .await?;
        }
    }
    // Use chunked TE + keep-alive so the connection can be reused
    stream
        .write_all(b"Transfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n")
        .await?;

    // Stream body chunks as they arrive from the upstream
    let mut body_stream = body.into_data_stream();
    while let Some(chunk_result) = FuturesStreamExt::next(&mut body_stream).await {
        match chunk_result {
            Ok(chunk) if !chunk.is_empty() => {
                // Write chunk size in hex, then chunk data, then CRLF
                stream
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await?;
                stream.write_all(&chunk).await?;
                stream.write_all(b"\r\n").await?;
                stream.flush().await?;
            }
            Ok(_) => {} // empty chunk — skip
            Err(e) => {
                tracing::debug!("Stream chunk error: {e}");
                break;
            }
        }
    }

    // Final zero-length chunk signals end of chunked body
    stream.write_all(b"0\r\n\r\n").await?;
    stream.flush().await?;
    Ok(())
}

/// Read a line (up to \n) from a stream, byte by byte.
async fn read_line(
    stream: &mut (impl AsyncReadExt + Unpin),
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte).await? {
            0 => {
                if buf.is_empty() {
                    return Err("EOF".into());
                }
                return Ok(String::from_utf8_lossy(&buf)
                    .trim_end_matches('\r')
                    .to_string());
            }
            _ => {
                if byte[0] == b'\n' {
                    // Strip trailing CR if present (CRLF line endings)
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                    return Ok(String::from_utf8_lossy(&buf).to_string());
                }
                buf.push(byte[0]);
            }
        }
    }
}

/// Write an HTTP/1.1 response with a fully-known body (for error responses).
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
