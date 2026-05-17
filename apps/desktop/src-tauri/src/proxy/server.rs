//! Axum proxy server bound to `127.0.0.1:4200`. See spec section 5.2.
//!
//! Provides:
//! - `POST /v1/messages`          — Anthropic Messages API pass-through
//! - `POST /v1/chat/completions`  — OpenAI / OpenRouter / compatible pass-through
//! - `CONNECT *`                  — TCP tunneling (HTTPS proxy support)
//! - `GET  /health`               — health check
//! - `GET  /stats`                — live stats for dashboard
//! - `GET  /sessions/:project_hash/graph` — session graph by project

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::net::TcpListener;
use tower::Service;
use tower_http::cors::CorsLayer;

use super::intercept::{self, InterceptState};
use super::mitm;

// ── Handlers ──────────────────────────────────────────────────────────────

/// `POST /v1/messages` — Anthropic Messages API.
async fn anthropic_handler(
    State(state): State<Arc<InterceptState>>,
    headers: HeaderMap,
    body: Json<serde_json::Value>,
) -> Response {
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("-");
    tracing::info!("→ POST /v1/messages  ua={}", ua);
    match intercept::handle_anthropic(&state, &headers, body.0).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /v1/chat/completions` — OpenAI / OpenRouter / compatible API.
/// Provider is auto-detected from the API key prefix.
async fn openai_handler(
    State(state): State<Arc<InterceptState>>,
    headers: HeaderMap,
    body: Json<serde_json::Value>,
) -> Response {
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("-");
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("-");
    tracing::info!("→ POST /v1/chat/completions  ua={}  model={}", ua, model);

    // Allow overriding the upstream base URL via header (for local models, etc.)
    let base_url = headers
        .get("x-upstream-base-url")
        .and_then(|v| v.to_str().ok());

    match intercept::handle_openai_compatible(&state, &headers, body.0, base_url).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    proxy_version: &'static str,
    uptime_seconds: u64,
}

/// `GET /health` — health check.
async fn health_handler(State(state): State<Arc<InterceptState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        proxy_version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.start_time.elapsed().as_secs(),
    })
}

#[derive(Serialize)]
struct StatsResponse {
    today: TodayStatsResponse,
    total: TotalStatsResponse,
    current_session: Option<CurrentSessionResponse>,
    active_sessions: Vec<CurrentSessionResponse>,
}

#[derive(Serialize)]
struct TodayStatsResponse {
    tokens_saved: u64,
    cost_saved_usd: f64,
    requests: u64,
    sessions: u64,
}

#[derive(Serialize)]
struct TotalStatsResponse {
    tokens_saved: u64,
    cost_saved_usd: f64,
    sessions: u64,
}

#[derive(Serialize, Clone)]
struct CurrentSessionResponse {
    id: String,
    active: bool,
    tokens_in_raw: u64,
    tokens_in_sent: u64,
    compression_ratio: f64,
    provider: String,
    project_name: Option<String>,
}

/// `GET /stats` — live dashboard stats.
async fn stats_handler(State(state): State<Arc<InterceptState>>) -> Json<StatsResponse> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let db = state.db.clone();
    let (today_stats, total_stats) = super::intercept::with_db(db, move |conn| {
        (
            crate::db::queries::get_today_stats(conn, &today).ok(),
            crate::db::queries::get_total_stats(conn).ok(),
        )
    })
    .await
    .unwrap_or((None, None));

    let (current_session, active_sessions) = {
        let sessions = state.active_sessions.lock().await;
        let all: Vec<CurrentSessionResponse> = sessions
            .iter()
            .map(|s| CurrentSessionResponse {
                id: s.id.clone(),
                active: true,
                tokens_in_raw: s.tokens_in_raw,
                tokens_in_sent: s.tokens_in_sent,
                compression_ratio: if s.tokens_in_raw > 0 {
                    s.tokens_in_sent as f64 / s.tokens_in_raw as f64
                } else {
                    0.0
                },
                provider: s.provider.clone(),
                project_name: s.project_name.clone(),
            })
            .collect();
        let first = all.first().cloned();
        (first, all)
    };

    Json(StatsResponse {
        today: TodayStatsResponse {
            tokens_saved: today_stats.as_ref().map_or(0, |t| t.tokens_saved),
            cost_saved_usd: today_stats.as_ref().map_or(0.0, |t| t.cost_saved_usd),
            requests: today_stats.as_ref().map_or(0, |t| t.requests),
            sessions: today_stats.as_ref().map_or(0, |t| t.sessions),
        },
        total: TotalStatsResponse {
            tokens_saved: total_stats.as_ref().map_or(0, |t| t.tokens_saved),
            cost_saved_usd: total_stats.as_ref().map_or(0.0, |t| t.cost_saved_usd),
            sessions: total_stats.as_ref().map_or(0, |t| t.sessions),
        },
        current_session,
        active_sessions,
    })
}

/// `GET /sessions` — returns a paginated list of past sessions.
async fn sessions_handler(
    State(state): State<Arc<InterceptState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let page: u32 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
    let per_page: u32 = params
        .get("per_page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let db = state.db.clone();
    let result = super::intercept::with_db(db, move |conn| {
        crate::db::queries::list_sessions_paginated(conn, page, per_page)
    })
    .await;

    let result = match result {
        Some(r) => r,
        None => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to access database",
            )
                .into_response();
        }
    };

    match result {
        Ok((items, total)) => {
            #[derive(Serialize)]
            struct SessionsResponse {
                items: Vec<crate::commands::sessions::SessionSummary>,
                page: u32,
                per_page: u32,
                total: u64,
            }
            Json(SessionsResponse {
                items,
                page,
                per_page,
                total,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list sessions: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to list sessions",
            )
                .into_response()
        }
    }
}

/// `GET /sessions/:project_hash/graph` — returns the session graph JSON.
async fn session_graph_handler(
    State(state): State<Arc<InterceptState>>,
    Path(project_hash): Path<String>,
) -> Response {
    let db = state.db.clone();
    let result = super::intercept::with_db(db, move |conn| {
        crate::db::queries::get_latest_graph_json(conn, &project_hash)
    })
    .await;

    let result = match result {
        Some(r) => r,
        None => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to access database",
            )
                .into_response();
        }
    };

    match result {
        Ok(Some(graph_json)) => {
            match serde_json::from_str::<serde_json::Value>(&graph_json) {
                Ok(parsed) => Json(parsed).into_response(),
                Err(e) => {
                    tracing::error!("Failed to parse graph JSON: {}", e);
                    (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to parse session graph",
                    )
                        .into_response()
                }
            }
        }
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            "No session graph found for this project",
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to get session graph: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to retrieve session graph",
            )
                .into_response()
        }
    }
}

// ── CONNECT proxy service ─────────────────────────────────────────────────
//
// CONNECT must be handled at the hyper Service level, not inside an axum
// handler.  axum wraps the request body which can lose the OnUpgrade extension
// that hyper sets on incoming requests.  By intercepting CONNECT before the
// request reaches the axum Router we guarantee the upgrade future is intact.

/// Hyper service that intercepts CONNECT before forwarding to the axum Router.
#[derive(Clone)]
struct ProxyService {
    state: Arc<InterceptState>,
    inner: axum::routing::RouterIntoService<axum::body::Body>,
}

impl hyper::service::Service<Request<hyper::body::Incoming>> for ProxyService {
    type Response = Response<axum::body::Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<hyper::body::Incoming>) -> Self::Future {
        if req.method() == Method::CONNECT {
            let state = self.state.clone();
            Box::pin(async move { Ok(handle_connect_hyper(req, state).await) })
        } else {
            let (parts, body) = req.into_parts();
            let body = axum::body::Body::new(body);
            let req = Request::from_parts(parts, body);
            let mut inner = self.inner.clone();
            Box::pin(async move { inner.call(req).await })
        }
    }
}

/// Handle a CONNECT request at the hyper service level so the OnUpgrade
/// extension is guaranteed to be present (not lost through axum rewrapping).
async fn handle_connect_hyper(
    req: Request<hyper::body::Incoming>,
    state: Arc<InterceptState>,
) -> Response<axum::body::Body> {
    let host_port = req.uri().to_string();
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(port) => (h.to_string(), port),
            Err(_) => {
                tracing::warn!("CONNECT invalid port: {}", p);
                return axum::http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(axum::body::Body::from("Invalid port"))
                    .unwrap();
            }
        },
        None => (host_port.clone(), 443),
    };

    tracing::debug!("CONNECT to {host}:{port}");

    let upgrade = hyper::upgrade::on(req);
    let mitm = state.mitm.clone();

    tokio::spawn(async move {
        let upgraded = match upgrade.await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("CONNECT upgrade to {host}:{port} failed: {e}");
                return;
            }
        };

        if let Some(mitm) = mitm {
            if is_intercept_host(&host) {
                tracing::debug!("CONNECT: MITM intercept for {host}:{port}");
                mitm::handle_connect(upgraded, &host, port, state, mitm).await;
                return;
            }
        }
        // All other hosts get a transparent TCP tunnel.
        tracing::debug!("CONNECT: tunnel passthrough for {host}:{port}");
        if let Err(e) = tunnel(upgraded, &host, port).await {
            tracing::debug!("Tunnel to {host}:{port} closed: {e}");
        }
    });

    // 200 Connection Established — no body, no content-length
    axum::http::Response::builder()
        .status(StatusCode::OK)
        .body(axum::body::Body::empty())
        .unwrap()
}

/// Returns true if this host should be MITM-intercepted (TLS termination).
///
/// Only MITM hosts where tools cannot set a plain-HTTP base URL and MUST
/// go through HTTPS_PROXY. Tools that support ANTHROPIC_BASE_URL /
/// OPENAI_BASE_URL already send plain HTTP to port 4200 — no MITM needed.
///
/// We MITM api.anthropic.com because claude-code uses it via HTTPS_PROXY
/// when ANTHROPIC_BASE_URL is not set, and its Node.js TLS stack accepts
/// our installed CA cert. We do NOT MITM openrouter.ai, api.openai.com,
/// or any other host because Cursor/Windsurf/opencode use plain HTTP via
/// the OPENAI_BASE_URL/ANTHROPIC_BASE_URL env vars — CONNECT to those
/// hosts comes from other background traffic that we should not intercept.
fn is_intercept_host(host: &str) -> bool {
    let h = host.trim_start_matches("www.");
    // Only intercept the Anthropic API — everything else uses plain HTTP
    // via the base URL env vars, so CONNECT to those hosts is background
    // traffic (updates, analytics) that must not be MITMed.
    h == "api.anthropic.com" || h.ends_with(".anthropic.com")
}

/// Relay bytes bidirectionally between upgraded connection and upstream TCP.
/// Used as fallback when MITM is disabled.
async fn tunnel(
    upgraded: hyper::upgrade::Upgraded,
    host: &str,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut upgraded = hyper_util::rt::TokioIo::new(upgraded);
    let mut upstream = tokio::net::TcpStream::connect((host, port)).await?;
    tokio::io::copy_bidirectional(&mut upgraded, &mut upstream).await?;
    Ok(())
}

// ── Server startup ────────────────────────────────────────────────────────

/// Build the Axum router (handles everything except CONNECT).
fn build_router(state: Arc<InterceptState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/sessions", get(sessions_handler))
        .route("/v1/messages", post(anthropic_handler))
        .route("/v1/chat/completions", post(openai_handler))
        .route("/sessions/:project_hash/graph", get(session_graph_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Start the proxy server on the given port. Blocks until the shutdown
/// signal is received (via the oneshot receiver).
pub async fn start(
    state: Arc<InterceptState>,
    port: u16,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();

    tracing::info!("Proxy server starting on http://{}", addr);

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind proxy server to {}: {}", addr, e);
            return;
        }
    };

    // Create a channel for restart signals and store it in state
    let (restart_tx, restart_rx) = tokio::sync::oneshot::channel();
    {
        let mut tx = state.restart_tx.lock().await;
        *tx = Some(restart_tx);
    }

    let shutdown_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_flag2 = shutdown_flag.clone();

    tokio::spawn(async move {
        tokio::select! {
            _ = shutdown => {
                tracing::info!("Proxy server shutting down");
            }
            _ = restart_rx => {
                tracing::info!("Proxy server received restart signal");
            }
        }
        shutdown_flag2.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    // Manual accept loop — required for HTTP upgrade (CONNECT) support.
    // We use HTTP/1.1 only; CONNECT tunneling is an HTTP/1.1 mechanism.
    loop {
        if shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let accept = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            listener.accept(),
        )
        .await;

        let (stream, _peer) = match accept {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                tracing::error!("Accept error: {}", e);
                continue;
            }
            Err(_elapsed) => continue,
        };

        let router = build_router(state.clone());
        let service = ProxyService {
            state: state.clone(),
            inner: router.into_service(),
        };

        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                let msg = e.to_string().to_lowercase();
                if !msg.contains("reset")
                    && !msg.contains("broken pipe")
                    && !msg.contains("connection closed")
                {
                    tracing::debug!("Connection error: {}", e);
                }
            }
        });
    }
}
