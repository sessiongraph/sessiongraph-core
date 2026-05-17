//! Request pipeline: receive → session → inject → compress → forward → log.
//! See spec section 5.3.
//!
//! Each request passes through this pipeline. Internal errors (DB write
//! failures, session tracker issues) are logged but NEVER propagate to the
//! client — the proxy always forwards the request regardless.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::http::HeaderMap;
use axum::response::Response;
use chrono::Utc;
use rusqlite::Connection;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

use super::compress;
use super::forward::{self, ForwardError, Provider};
use super::session::{self, ActiveSession};
use crate::db::queries;
use crate::graph::injector;

/// Shared state between the proxy server and Tauri commands.
pub struct InterceptState {
    /// SQLite connection (blocking — wrapped in Arc<Mutex<>> for Send + Clone)
    pub db: Arc<Mutex<Connection>>,
    /// In-memory active session tracker keyed by project_hash
    pub active_sessions: TokioMutex<Vec<ActiveSession>>,
    /// When the proxy started (for uptime)
    pub start_time: Instant,
    /// Timeout in minutes before a session is considered ended
    pub session_timeout_minutes: i64,
    /// Whether compression is enabled
    pub compression_enabled: bool,
    /// Whether graph injection is enabled
    pub graph_injection_enabled: bool,
    /// Channel to trigger proxy restart
    pub restart_tx: TokioMutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl InterceptState {
    pub fn new(db: Connection) -> Self {
        // Read settings from the database, fall back to defaults if not present
        let session_timeout_minutes = queries::get_setting(&db, "session_timeout_minutes")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let compression_enabled = queries::get_setting(&db, "compression_enabled")
            .ok()
            .flatten()
            .map(|v| v == "true")
            .unwrap_or(true);

        let graph_injection_enabled = queries::get_setting(&db, "graph_injection_enabled")
            .ok()
            .flatten()
            .map(|v| v == "true")
            .unwrap_or(true);

        Self {
            db: Arc::new(Mutex::new(db)),
            active_sessions: TokioMutex::new(Vec::new()),
            start_time: Instant::now(),
            session_timeout_minutes,
            compression_enabled,
            graph_injection_enabled,
            restart_tx: TokioMutex::new(None),
        }
    }

    /// Trigger a proxy restart. The restart happens by sending a signal
    /// that the server monitors. The server will shut down and the caller
    /// is responsible for starting a new instance.
    pub async fn trigger_restart(&self) -> Result<(), String> {
        let mut tx = self.restart_tx.lock().await;
        if let Some(sender) = tx.take() {
            let _ = sender.send(());
            tracing::info!("Proxy restart triggered");
            Ok(())
        } else {
            Err("No restart channel available".to_string())
        }
    }
}

/// The result of running the pipeline for one request.
#[derive(Debug)]
pub struct PipelineLog {
    pub session_id: String,
    pub request_id: String,
    pub sequence: u32,
    pub project_hash: String,
    pub provider: String,
    pub model: String,
    pub tokens_in_raw: u64,
    pub tokens_in_sent: u64,
    pub tokens_out: u64,
    pub compression_ratio: Option<f64>,
    pub graph_injected: bool,
    pub graph_tokens: u32,
    pub latency_ms: u64,
    pub cost_usd_raw: f64,
    pub cost_usd_actual: f64,
    pub is_new_session: bool,
    pub session_ended: bool,
}

/// Run the full request pipeline for an Anthropic-format request.
pub async fn handle_anthropic(
    state: &InterceptState,
    headers: &HeaderMap,
    mut body: serde_json::Value,
) -> Result<Response, ForwardError> {
    let provider = Provider::Anthropic;
    let provider_str = provider.as_str().to_string();
    let start = Instant::now();

    // 1. Session identification
    let api_key = forward::extract_api_key(headers, &provider).unwrap_or_default();
    let system_prompt = extract_anthropic_system(&body);
    let project_hash = session::compute_project_hash(system_prompt, None);
    let tool = forward::detect_tool(headers, &provider);
    let model = forward::extract_model(&body);
    let tokens_in_raw = forward::estimate_tokens(&body);
    let cost_usd_raw = forward::compute_cost(&model, tokens_in_raw, 0);

    // 2. Session lifecycle
    let project_name = session::infer_project_name(system_prompt);
    let (session_id, is_new, sequence, ended_session) = manage_session(
        state,
        &project_hash,
        project_name,
        &provider_str,
        tool.clone(),
        &api_key,
        &body,
        tokens_in_raw,
    )
    .await;

    // If a session just ended, spawn extraction task
    if let Some(ended) = ended_session {
        let db = state.db.clone();
        tokio::spawn(extract_and_store(db, ended));
    }

    // 3. Graph injection (if new session AND injection enabled AND graph exists)
    let mut graph_injected = false;
    let mut graph_tokens: u32 = 0;
    if is_new && state.graph_injection_enabled {
        if let Ok(db) = state.db.lock() {
            let result = injector::inject(&db, body.clone(), &project_hash, &provider_str);
            graph_injected = result.injected;
            graph_tokens = result.graph_tokens;
            body = result.body;
        }
    }

    // 4. Compression (if enabled)
    let mut tokens_in_sent = tokens_in_raw;
    let mut compression_ratio: Option<f64> = None;
    let mut cost_usd_actual = cost_usd_raw;

    if state.compression_enabled {
        if let Some(messages) = body.get("messages").and_then(|v| v.as_array()) {
            let messages_vec: Vec<_> = messages.to_vec();
            if let Some(compressed) = compress::compress(&messages_vec, &model).await {
                body["messages"] = serde_json::Value::Array(compressed.messages);
                tokens_in_sent = compressed.tokens_after;
                compression_ratio = if tokens_in_raw > 0 {
                    Some(tokens_in_sent as f64 / tokens_in_raw as f64)
                } else {
                    None
                };
                cost_usd_actual = forward::compute_cost(&model, tokens_in_sent, 0);

                // Update in-memory session counter for accurate live dashboard
                // After compression, tokens_in_sent is the compressed value
                // We need to add this (not subtract) to the session's running total
                let mut sessions = state.active_sessions.lock().await;
                if let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) {
                    // First remove the raw value that was added in manage_session,
                    // then add the compressed value
                    s.tokens_in_sent = s.tokens_in_sent.saturating_sub(tokens_in_raw);
                    s.tokens_in_sent = s.tokens_in_sent.saturating_add(tokens_in_sent);
                }
            }
        }
    }

    // 5. Forward to upstream
    let forward_result = forward::forward_anthropic(body, &api_key).await?;
    let response = forward_result.response;
    let token_counter = forward_result.token_count;

    // 6. Log with deferred token count
    // The token count is updated in real-time during streaming.
    // We spawn a task to read the final count after a short delay.
    let latency_ms = start.elapsed().as_millis() as u64;
    let request_id = Uuid::new_v4().to_string();
    let log = PipelineLog {
        session_id: session_id.clone(),
        request_id: request_id.clone(),
        sequence,
        project_hash,
        provider: provider_str.clone(),
        model,
        tokens_in_raw,
        tokens_in_sent,
        tokens_out: 0, // Will be updated by background task
        compression_ratio,
        graph_injected,
        graph_tokens,
        latency_ms,
        cost_usd_raw,
        cost_usd_actual,
        is_new_session: is_new,
        session_ended: false,
    };

    let db = state.db.clone();
    let session_id_for_log = session_id.clone();
    let model_for_cost = model.clone();
    tokio::spawn(log_request(db, log));

    // Spawn a task to update the token count after the stream completes
    // The delay gives time for the SSE stream to fully send
    let db_update = state.db.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let bytes_out = token_counter.load(std::sync::atomic::Ordering::Relaxed);
        let tokens_out = forward::bytes_to_tokens(bytes_out);

        // Update the session with output tokens
        if let Ok(db) = db_update.lock() {
            let cost_out = forward::compute_cost(&model_for_cost, 0, tokens_out);
            let _ = crate::db::queries::increment_session(
                &db,
                &session_id_for_log,
                0, // message_count delta
                0, // tokens_in_raw delta
                0, // tokens_in_sent delta
                tokens_out,
                0.0, // cost_raw delta
                cost_out,
            );
        }
    });

    Ok(response)
}

/// Run the full request pipeline for an OpenAI-compatible request.
/// Auto-detects provider (OpenAI, OpenRouter, or custom) from headers.
pub async fn handle_openai_compatible(
    state: &InterceptState,
    headers: &HeaderMap,
    mut body: serde_json::Value,
    base_url_override: Option<&str>,
) -> Result<Response, ForwardError> {
    let provider = if let Some(url) = base_url_override {
        Provider::OpenAICompatible { base_url: url.to_string() }
    } else {
        forward::detect_provider(headers)
    };
    let provider_str = provider.as_str().to_string();
    let start = Instant::now();

    // 1. Session identification
    let api_key = forward::extract_api_key(headers, &provider).unwrap_or_default();
    let system_prompt = extract_openai_system(&body);
    let project_hash = session::compute_project_hash(system_prompt, None);
    let tool = forward::detect_tool(headers, &provider);
    let model = forward::extract_model(&body);
    let tokens_in_raw = forward::estimate_tokens(&body);
    let cost_usd_raw = forward::compute_cost(&model, tokens_in_raw, 0);

    // 2. Session lifecycle
    let project_name = session::infer_project_name(system_prompt);
    let (session_id, is_new, sequence, ended_session) = manage_session(
        state,
        &project_hash,
        project_name,
        &provider_str,
        tool.clone(),
        &api_key,
        &body,
        tokens_in_raw,
    )
    .await;

    // If a session just ended, spawn extraction task
    if let Some(ended) = ended_session {
        let db = state.db.clone();
        tokio::spawn(extract_and_store(db, ended));
    }

    // 3. Graph injection
    let mut graph_injected = false;
    let mut graph_tokens: u32 = 0;
    if is_new && state.graph_injection_enabled {
        if let Ok(db) = state.db.lock() {
            let result = injector::inject(&db, body.clone(), &project_hash, &provider_str);
            graph_injected = result.injected;
            graph_tokens = result.graph_tokens;
            body = result.body;
        }
    }

    // 4. Compression (if enabled)
    let mut tokens_in_sent = tokens_in_raw;
    let mut compression_ratio: Option<f64> = None;
    let mut cost_usd_actual = cost_usd_raw;

    if state.compression_enabled {
        if let Some(messages) = body.get("messages").and_then(|v| v.as_array()) {
            let messages_vec: Vec<_> = messages.to_vec();
            if let Some(compressed) = compress::compress(&messages_vec, &model).await {
                body["messages"] = serde_json::Value::Array(compressed.messages);
                tokens_in_sent = compressed.tokens_after;
                compression_ratio = if tokens_in_raw > 0 {
                    Some(tokens_in_sent as f64 / tokens_in_raw as f64)
                } else {
                    None
                };
                cost_usd_actual = forward::compute_cost(&model, tokens_in_sent, 0);

                // Update in-memory session counter for accurate live dashboard
                let mut sessions = state.active_sessions.lock().await;
                if let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) {
                    s.tokens_in_sent = s.tokens_in_sent.saturating_sub(tokens_in_raw);
                    s.tokens_in_sent = s.tokens_in_sent.saturating_add(tokens_in_sent);
                }
            }
        }
    }

    // 5. Forward to upstream
    let forward_result = forward::forward_openai_compatible(body, &api_key, &provider, headers).await?;
    let response = forward_result.response;
    let token_counter = forward_result.token_count;

    // 6. Log with deferred token count
    let latency_ms = start.elapsed().as_millis() as u64;
    let request_id = Uuid::new_v4().to_string();
    let log = PipelineLog {
        session_id: session_id.clone(),
        request_id: request_id.clone(),
        sequence,
        project_hash,
        provider: provider_str.clone(),
        model,
        tokens_in_raw,
        tokens_in_sent,
        tokens_out: 0,
        compression_ratio,
        graph_injected,
        graph_tokens,
        latency_ms,
        cost_usd_raw,
        cost_usd_actual,
        is_new_session: is_new,
        session_ended: false,
    };

    let db = state.db.clone();
    let session_id_for_log = session_id.clone();
    let model_for_cost = model.clone();
    tokio::spawn(log_request(db, log));

    // Spawn a task to update the token count after the stream completes
    let db_update = state.db.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let bytes_out = token_counter.load(std::sync::atomic::Ordering::Relaxed);
        let tokens_out = forward::bytes_to_tokens(bytes_out);

        if let Ok(db) = db_update.lock() {
            let cost_out = forward::compute_cost(&model_for_cost, 0, tokens_out);
            let _ = crate::db::queries::increment_session(
                &db,
                &session_id_for_log,
                0,
                0,
                0,
                tokens_out,
                0.0,
                cost_out,
            );
        }
    });

    Ok(response)
}

/// Manage session lifecycle:
/// - Find or create an active session for the project
/// - If previous session timed out (30 min gap), end it and create new
/// - Accumulate message bodies for extraction
///
/// Returns (session_id, is_new_session, sequence, optionally_ended_session).
/// Sequence is the request number within the session (1-indexed).
async fn manage_session(
    state: &InterceptState,
    project_hash: &str,
    project_name: Option<String>,
    provider: &str,
    tool: Option<String>,
    api_key: &str,
    body: &serde_json::Value,
    tokens_in_raw: u64,
) -> (String, bool, u32, Option<ActiveSession>) {
    let mut sessions = state.active_sessions.lock().await;
    let timeout = state.session_timeout_minutes;

    // Check if any session has timed out
    let mut ended: Option<ActiveSession> = None;
    if let Some(idx) = sessions.iter().position(|s| {
        s.project_hash == project_hash && s.provider == provider && s.is_timed_out(timeout)
    }) {
        let mut ended_session = sessions.remove(idx);
        ended_session.cost_usd_actual = ended_session.cost_usd_raw; // no compression yet
        ended = Some(ended_session.clone());

        // End the session in the database (best-effort)
        if let Ok(db) = state.db.lock() {
            let _ = queries::end_session(&db, &ended_session.id, &Utc::now().to_rfc3339());
        }
        tracing::info!(
            "Session {} ended (timeout), project={}, provider={}",
            ended_session.id,
            project_hash,
            provider
        );
    }

    // Find active session or create new one
    if let Some(s) = sessions
        .iter_mut()
        .find(|s| s.project_hash == project_hash && s.provider == provider)
    {
        // Existing session — update counters and accumulate body
        s.last_request_at = Utc::now();
        s.message_count += 1;
        let sequence = s.message_count;
        s.tokens_in_raw += tokens_in_raw;
        s.tokens_in_sent += tokens_in_raw; // no compression yet
        s.push_body(body);
        (s.id.clone(), false, sequence, ended)
    } else {
        // New session
        let mut new_session = ActiveSession::new(
            project_hash.to_string(),
            project_name,
            provider.to_string(),
            tool,
            api_key.to_string(),
        );
        new_session.message_count = 1;
        let sequence = 1;
        new_session.tokens_in_raw = tokens_in_raw;
        new_session.tokens_in_sent = tokens_in_raw;
        new_session.push_body(body);

        let id = new_session.id.clone();
        if let Ok(db) = state.db.lock() {
            let _ = queries::insert_session(&db, &new_session);
        }
        sessions.push(new_session);
        tracing::info!(
            "New session created: project={}, provider={}",
            project_hash,
            provider
        );
        (id, true, sequence, ended)
    }
}

/// Extract a session graph from an ended session and store it in the database.
/// Runs as a background task — errors are logged, never surfaced.
async fn extract_and_store(db: Arc<Mutex<Connection>>, session: ActiveSession) {
    let messages_json = session.messages_for_extraction();
    tracing::info!(
        "Extracting graph for session {} (project={}, {} messages)",
        session.id,
        session.project_hash,
        session.recent_bodies.len()
    );

    let graph = match session.provider.as_str() {
        "anthropic" => {
            crate::graph::extractor::extract_anthropic(
                &session.api_key,
                &session.id,
                &session.project_hash,
                &messages_json,
            )
            .await
        }
        _ => {
            crate::graph::extractor::extract_openai(
                &session.api_key,
                &session.id,
                &session.project_hash,
                &messages_json,
            )
            .await
        }
    };

    let Some(graph) = graph else {
        tracing::warn!(
            "Graph extraction failed for session {} — continuing without graph",
            session.id
        );
        return;
    };

    let graph_json = match serde_json::to_string(&graph) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to serialize extracted graph: {}", e);
            return;
        }
    };

    let db = match db.lock() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to lock DB for graph storage: {}", e);
            return;
        }
    };

    let graph_id = Uuid::new_v4().to_string();
    let extraction_model = if session.provider == "anthropic" {
        "claude-3-haiku-20240307"
    } else {
        "gpt-4o-mini"
    };
    let extraction_cost = super::forward::compute_cost(
        extraction_model,
        forward::estimate_tokens(&serde_json::json!({"text": messages_json})),
        graph.token_count as u64,
    );

    if let Err(e) = queries::upsert_session_graph(
        &db,
        &graph_id,
        &session.id,
        &session.project_hash,
        &graph_json,
        graph.token_count,
        extraction_model,
        extraction_cost,
    ) {
        tracing::error!("Failed to store extracted graph: {}", e);
        return;
    }

    // Mark session as 'extracted'
    let _ = db.execute(
        "UPDATE sessions SET status = 'extracted' WHERE id = ?1",
        rusqlite::params![session.id],
    );

    tracing::info!(
        "Graph extracted and stored for session {} ({} tokens, cost ${:.6})",
        session.id,
        graph.token_count,
        extraction_cost
    );
}

/// Log a completed request to the database. Runs in a background task;
/// errors are logged and silently discarded.
async fn log_request(db: Arc<Mutex<Connection>>, log: PipelineLog) {
    let db = match db.lock() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to acquire DB lock for request logging: {}", e);
            return;
        }
    };

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let sequence = log.sequence;

    if let Err(e) = queries::insert_request(
        &db,
        &log.request_id,
        &log.session_id,
        log.sequence,
        &log.provider,
        &log.model,
        log.tokens_in_raw,
        log.tokens_in_sent,
        log.tokens_out,
        log.compression_ratio,
        log.graph_injected,
        log.graph_tokens,
        log.latency_ms,
        log.cost_usd_raw,
        log.cost_usd_actual,
    ) {
        tracing::error!("Failed to log request: {}", e);
        return;
    }

    if let Err(e) = queries::upsert_daily_usage(
        &db,
        &today,
        &log.provider,
        log.tokens_in_raw,
        log.tokens_in_sent,
        log.tokens_out,
        log.cost_usd_raw,
        log.cost_usd_actual,
    ) {
        tracing::error!("Failed to upsert daily usage: {}", e);
    }

    if let Err(e) = queries::increment_session(
        &db,
        &log.session_id,
        1,
        log.tokens_in_raw,
        log.tokens_in_sent,
        log.tokens_out,
        log.cost_usd_raw,
        log.cost_usd_actual,
    ) {
        tracing::error!("Failed to increment session: {}", e);
    }
}

/// Extract the system prompt from an Anthropic request body.
fn extract_anthropic_system(body: &serde_json::Value) -> Option<&str> {
    body.get("system").and_then(|v| {
        if v.is_string() {
            v.as_str()
        } else if v.is_array() {
            v.as_array()
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("text"))
                .and_then(|t| t.as_str())
        } else {
            None
        }
    })
}

/// Extract the system prompt from an OpenAI request body.
fn extract_openai_system(body: &serde_json::Value) -> Option<&str> {
    body.get("messages")
        .and_then(|v| v.as_array())
        .and_then(|msgs| {
            msgs.iter().find_map(|m| {
                if m.get("role")?.as_str()? == "system" {
                    let content = m.get("content")?;
                    if content.is_string() {
                        content.as_str()
                    } else if content.is_array() {
                        content
                            .as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|item| item.get("text"))
                            .and_then(|t| t.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        })
}
