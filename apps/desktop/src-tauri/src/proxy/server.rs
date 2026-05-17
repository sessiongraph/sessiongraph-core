//! Axum proxy server bound to `127.0.0.1:4200`. See spec section 5.2.
//!
//! Provides:
//! - `POST /v1/messages`          — Anthropic Messages API pass-through
//! - `POST /v1/chat/completions`  — OpenAI / OpenRouter / compatible pass-through
//! - `GET  /health`               — health check
//! - `GET  /stats`                — live stats for dashboard
//! - `GET  /sessions/:project_hash/graph` — session graph by project

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::Path;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

use super::intercept::{self, InterceptState};

// ── Handlers ──────────────────────────────────────────────────────────────

/// `POST /v1/messages` — Anthropic Messages API.
async fn anthropic_handler(
    State(state): State<Arc<InterceptState>>,
    headers: HeaderMap,
    body: Json<serde_json::Value>,
) -> Response {
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
            // Parse and return the JSON with proper content-type
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

// ── Server startup ────────────────────────────────────────────────────────

/// Build the Axum router.
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
    let router = build_router(state.clone());

    tracing::info!("Proxy server starting on http://{}", addr);

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind proxy server to {}: {}", addr, e);
            return;
        }
    };

    // Create a channel for restart signals and store it in state
    let (restart_tx, mut restart_rx) = tokio::sync::oneshot::channel();
    {
        let mut tx = state.restart_tx.lock().await;
        *tx = Some(restart_tx);
    }

    // Run the server with graceful shutdown that also handles restart
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = shutdown => {
                    tracing::info!("Proxy server shutting down");
                }
                _ = &mut restart_rx => {
                    tracing::info!("Proxy server received restart signal");
                }
            }
        })
        .await
        .unwrap_or_else(|e| tracing::error!("Proxy server error: {}", e));
}
