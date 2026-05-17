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
    /// In-memory active session tracker keyed by (project_hash, provider, api_key_hash)
    pub active_sessions: TokioMutex<Vec<ActiveSession>>,
    /// When the proxy started (for uptime)
    pub start_time: Instant,
    /// Timeout in minutes before a session is considered ended
    /// The port the proxy is listening on
    pub proxy_port: u16,
    /// Timeout in minutes before a session is considered ended
    pub session_timeout_minutes: i64,
    /// Whether compression is enabled
    pub compression_enabled: bool,
    /// Whether graph injection is enabled
    pub graph_injection_enabled: bool,
    /// Custom Anthropic-compatible base URL (None = default api.anthropic.com)
    pub anthropic_base_url: Option<String>,
    /// Custom OpenAI-compatible base URL (None = default api.openai.com)
    pub openai_base_url: Option<String>,
    /// Channel to trigger proxy restart
    pub restart_tx: TokioMutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl InterceptState {
    pub fn new(db: Connection, proxy_port: u16) -> Self {
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

        let anthropic_base_url = queries::get_setting(&db, "anthropic_base_url")
            .ok()
            .flatten()
            .filter(|v| !v.is_empty() && v != "https://api.anthropic.com");

        let openai_base_url = queries::get_setting(&db, "openai_base_url")
            .ok()
            .flatten()
            .filter(|v| !v.is_empty() && v != "https://api.openai.com");

        Self {
            db: Arc::new(Mutex::new(db)),
            active_sessions: TokioMutex::new(Vec::new()),
            start_time: Instant::now(),
            proxy_port,
            session_timeout_minutes,
            compression_enabled,
            graph_injection_enabled,
            anthropic_base_url,
            openai_base_url,
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

    /// End all active sessions and write them to the database.
    /// Called on app shutdown so sessions don't remain 'active' forever.
    pub async fn end_all_sessions(&self) {
        let mut sessions = self.active_sessions.lock().await;
        let now = Utc::now().to_rfc3339();

        for s in sessions.drain(..) {
            {
                let db = self.db.clone();
                let sid = s.id.clone();
                let n = now.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let conn = db.lock().ok()?;
                    queries::end_session(&conn, &sid, &n).ok()
                })
                .await;
            }
            tracing::info!("Session {} ended (app shutdown)", s.id);

            // Spawn extraction for each ended session
            let db_clone = self.db.clone();
            tokio::spawn(extract_and_store(db_clone, s));
        }
    }
}

/// Run a synchronous database operation in a blocking thread (spawn_blocking)
/// so the Tokio runtime is not blocked. Returns None on failure (logged
/// internally by the caller) or the operation's result.
pub async fn with_db<F, R>(db: Arc<Mutex<Connection>>, f: F) -> Option<R>
where
    F: FnOnce(&Connection) -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().ok()?;
        Some(f(&conn))
    })
    .await
    .ok()?
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
    let api_key_hash = session::hash_api_key(&api_key);
    let system_prompt = extract_anthropic_system(&body);
    let project_hash = session::compute_project_hash(system_prompt, None);
    let tool = forward::detect_tool(headers, &provider);
    let model = forward::extract_model(&body);
    let tokens_in_raw = forward::estimate_tokens(&body);
    let cost_usd_raw = forward::compute_cost(&model, tokens_in_raw, 0);

    // 2. Session lifecycle (no token DB writes — that happens after the request)
    let project_name = session::infer_project_name(system_prompt);
    let (session_id, is_new, sequence, ended_session) = manage_session(
        state,
        &project_hash,
        &api_key_hash,
        project_name,
        &provider_str,
        tool.clone(),
        &api_key,
        &body,
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
        let body_clone = body.clone();
        let ph = project_hash.clone();
        let ps = provider_str.clone();
        let result = with_db(state.db.clone(), move |db| {
            let gmt: u32 = queries::get_setting(db, "graph_max_tokens")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(500);
            injector::inject(db, body_clone, &ph, &ps, gmt)
        })
        .await;
        if let Some(r) = result {
            graph_injected = r.injected;
            graph_tokens = r.graph_tokens;
            body = r.body;
        }
    }

    // 4. Compression (if enabled)
    let mut tokens_in_sent = tokens_in_raw;
    let mut compression_ratio: Option<f64> = None;

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
            }
        }
    }

    // Update in-memory session counters for live dashboard
    {
        let mut sessions = state.active_sessions.lock().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) {
            s.tokens_in_raw += tokens_in_raw;
            s.tokens_in_sent += tokens_in_sent;
        }
    }

    // 5. Forward to upstream
    let forward_result =
        forward::forward_anthropic(body, &api_key, state.anthropic_base_url.as_deref()).await?;
    let response = forward_result.response;
    let token_counter = forward_result.token_count;

    // 6. Log everything in a single background task (waits for output tokens)
    let latency_ms = start.elapsed().as_millis() as u64;
    let request_id = Uuid::new_v4().to_string();
    let db = state.db.clone();
    let session_id_clone = session_id.clone();
    let provider_clone = provider_str.clone();
    let model_clone = model.clone();

    tokio::spawn(async move {
        // Wait for the byte stream to flush — poll counter until stable
        let tokens_out = wait_for_output_tokens(&token_counter).await;

        let cost_usd_actual_with_out =
            forward::compute_cost(&model_clone, tokens_in_sent, tokens_out);

        // Update in-memory session tokens_out
        // (best-effort — sessions lock might be held; fine if skipped, dashboard updates next poll)

        // All DB writes in one spawn_blocking call (single TX-equivalent batch)
        let _ = with_db(db, move |conn| {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let _ = queries::insert_request(
                conn,
                &request_id,
                &session_id_clone,
                sequence,
                &provider_clone,
                &model_clone,
                tokens_in_raw,
                tokens_in_sent,
                tokens_out,
                compression_ratio,
                graph_injected,
                graph_tokens,
                latency_ms,
                cost_usd_raw,
                cost_usd_actual_with_out,
            );
            let _ = queries::upsert_daily_usage(
                conn,
                &today,
                &provider_clone,
                tokens_in_raw,
                tokens_in_sent,
                tokens_out,
                cost_usd_raw,
                cost_usd_actual_with_out,
            );
            let _ = queries::increment_session(
                conn,
                &session_id_clone,
                1,
                tokens_in_raw,
                tokens_in_sent,
                tokens_out,
                cost_usd_raw,
                cost_usd_actual_with_out,
            );
        })
        .await;
    });

    Ok(response)
}

/// Run the full request pipeline for an OpenAI-compatible request.
pub async fn handle_openai_compatible(
    state: &InterceptState,
    headers: &HeaderMap,
    mut body: serde_json::Value,
    base_url_override: Option<&str>,
) -> Result<Response, ForwardError> {
    let provider = if let Some(url) = base_url_override {
        // Explicit header override always wins
        Provider::OpenAICompatible {
            base_url: url.to_string(),
        }
    } else {
        let detected = forward::detect_provider(headers);
        // Only override generic OpenAI to the configured base URL.
        // OpenRouter (sk-or-v1-*) and Anthropic (sk-ant-*) keys still route
        // to their correct upstreams regardless of the openai_base_url setting.
        match &detected {
            Provider::OpenAI => match state.openai_base_url.as_deref() {
                Some(url) => Provider::OpenAICompatible {
                    base_url: url.to_string(),
                },
                None => detected,
            },
            _ => detected, // OpenRouter, Anthropic — don't touch
        }
    };
    let provider_str = provider.as_str().to_string();
    let start = Instant::now();

    // 1. Session identification
    let api_key = forward::extract_api_key(headers, &provider).unwrap_or_default();
    let api_key_hash = session::hash_api_key(&api_key);
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
        &api_key_hash,
        project_name,
        &provider_str,
        tool.clone(),
        &api_key,
        &body,
    )
    .await;

    if let Some(ended) = ended_session {
        let db = state.db.clone();
        tokio::spawn(extract_and_store(db, ended));
    }

    // 3. Graph injection
    let mut graph_injected = false;
    let mut graph_tokens: u32 = 0;
    if is_new && state.graph_injection_enabled {
        let body_clone = body.clone();
        let ph = project_hash.clone();
        let ps = provider_str.clone();
        let result = with_db(state.db.clone(), move |db| {
            let gmt: u32 = queries::get_setting(db, "graph_max_tokens")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(500);
            injector::inject(db, body_clone, &ph, &ps, gmt)
        })
        .await;
        if let Some(r) = result {
            graph_injected = r.injected;
            graph_tokens = r.graph_tokens;
            body = r.body;
        }
    }

    // 4. Compression (if enabled)
    let mut tokens_in_sent = tokens_in_raw;
    let mut compression_ratio: Option<f64> = None;

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
            }
        }
    }

    // Update in-memory session counters
    {
        let mut sessions = state.active_sessions.lock().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) {
            s.tokens_in_raw += tokens_in_raw;
            s.tokens_in_sent += tokens_in_sent;
        }
    }

    // 5. Forward to upstream
    let forward_result =
        forward::forward_openai_compatible(body, &api_key, &provider, headers).await?;
    let response = forward_result.response;
    let token_counter = forward_result.token_count;

    // 6. Log in single background task
    let latency_ms = start.elapsed().as_millis() as u64;
    let request_id = Uuid::new_v4().to_string();
    let db = state.db.clone();
    let session_id_clone = session_id.clone();
    let provider_clone = provider_str.clone();
    let model_clone = model.clone();

    tokio::spawn(async move {
        let tokens_out = wait_for_output_tokens(&token_counter).await;
        let cost_usd_actual_with_out =
            forward::compute_cost(&model_clone, tokens_in_sent, tokens_out);

        let _ = with_db(db, move |conn| {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let _ = queries::insert_request(
                conn,
                &request_id,
                &session_id_clone,
                sequence,
                &provider_clone,
                &model_clone,
                tokens_in_raw,
                tokens_in_sent,
                tokens_out,
                compression_ratio,
                graph_injected,
                graph_tokens,
                latency_ms,
                cost_usd_raw,
                cost_usd_actual_with_out,
            );
            let _ = queries::upsert_daily_usage(
                conn,
                &today,
                &provider_clone,
                tokens_in_raw,
                tokens_in_sent,
                tokens_out,
                cost_usd_raw,
                cost_usd_actual_with_out,
            );
            let _ = queries::increment_session(
                conn,
                &session_id_clone,
                1,
                tokens_in_raw,
                tokens_in_sent,
                tokens_out,
                cost_usd_raw,
                cost_usd_actual_with_out,
            );
        })
        .await;
    });

    Ok(response)
}

/// Wait for the output token counter to stabilise.
/// Polls the atomic counter with exponential backoff up to 5 seconds total.
async fn wait_for_output_tokens(counter: &std::sync::atomic::AtomicU64) -> u64 {
    use std::sync::atomic::Ordering;

    let mut last = counter.load(Ordering::Relaxed);
    let mut stable_ms = 0u64;

    for delay_ms in [50, 100, 200, 400, 800, 800, 800, 800, 800] {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        let current = counter.load(Ordering::Relaxed);
        if current == last {
            stable_ms += delay_ms;
            if stable_ms >= 800 {
                break;
            }
        } else {
            stable_ms = 0;
            last = current;
        }
    }

    forward::bytes_to_tokens(last)
}

/// Manage session lifecycle:
/// - Find or create an active session for (project_hash, provider, api_key_hash)
/// - If previous session timed out (30 min gap), end it and create new
/// - Accumulate message bodies for extraction
///
/// Returns (session_id, is_new_session, sequence, optionally_ended_session).
/// Sequence is the request number within the session (1-indexed).
///
/// NOTE: This function handles lifecycle ONLY. Token/cost DB writes happen
/// in the post-request logging task to avoid double-counting.
#[allow(clippy::too_many_arguments)]
async fn manage_session(
    state: &InterceptState,
    project_hash: &str,
    api_key_hash: &str,
    project_name: Option<String>,
    provider: &str,
    tool: Option<String>,
    api_key: &str,
    body: &serde_json::Value,
) -> (String, bool, u32, Option<ActiveSession>) {
    let mut sessions = state.active_sessions.lock().await;
    let timeout = state.session_timeout_minutes;

    // Check if any session has timed out
    let mut ended: Option<ActiveSession> = None;
    let lookup_key = (
        project_hash.to_string(),
        provider.to_string(),
        api_key_hash.to_string(),
    );

    if let Some(idx) = sessions.iter().position(|s| {
        s.project_hash == lookup_key.0
            && s.provider == lookup_key.1
            && s.api_key_hash == lookup_key.2
            && s.is_timed_out(timeout)
    }) {
        let mut ended_session = sessions.remove(idx);
        ended_session.cost_usd_actual = ended_session.cost_usd_raw;
        ended = Some(ended_session.clone());

        // End the session in the database (best-effort)
        let ended_id = ended_session.id.clone();
        let _ = with_db(state.db.clone(), move |conn| {
            let _ = queries::end_session(conn, &ended_id, &Utc::now().to_rfc3339());
        })
        .await;
        tracing::info!(
            "Session {} ended (timeout), project={}, provider={}",
            ended_session.id,
            project_hash,
            provider
        );
    }

    // Find active session or create new one
    if let Some(s) = sessions.iter_mut().find(|s| {
        s.project_hash == lookup_key.0
            && s.provider == lookup_key.1
            && s.api_key_hash == lookup_key.2
    }) {
        // Existing session — bump message count and accumulate body
        s.last_request_at = Utc::now();
        s.message_count += 1;
        let sequence = s.message_count as u32;
        s.push_body(body);
        (s.id.clone(), false, sequence, ended)
    } else {
        // New session — insert row with zero token/cost counters
        let mut new_session = ActiveSession::new(
            project_hash.to_string(),
            api_key_hash.to_string(),
            project_name,
            provider.to_string(),
            tool,
        );
        new_session.api_key = api_key.to_string();
        new_session.message_count = 1;
        new_session.push_body(body);

        let id = new_session.id.clone();
        let session_for_db = new_session.clone();
        let _ = with_db(state.db.clone(), move |conn| {
            let _ = queries::insert_session(conn, &session_for_db);
        })
        .await;
        sessions.push(new_session);
        tracing::info!(
            "New session created: project={}, provider={}",
            project_hash,
            provider
        );
        (id, true, 1, ended)
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

    // If API key is empty, skip extraction — we can't call the provider
    if session.api_key.is_empty() {
        tracing::warn!(
            "No API key available for session {} — skipping graph extraction",
            session.id
        );
        crate::db::log_error(&format!(
            "Graph extraction skipped: no API key for session {}",
            session.id
        ));
        return;
    }

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
        crate::db::log_error(&format!(
            "Graph extraction failed for session {}",
            session.id
        ));
        return;
    };

    let graph_json = match serde_json::to_string(&graph) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to serialize extracted graph: {}", e);
            crate::db::log_error(&format!(
                "Failed to serialize graph for session {}: {}",
                session.id, e
            ));
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

    let sid = session.id.clone();
    let sid_for_log = sid.clone();
    let ph = session.project_hash.clone();
    let em = extraction_model.to_string();
    let tc = graph.token_count;
    let sj = graph_json.clone();
    if with_db(db, move |conn| {
        let _ = queries::upsert_session_graph(
            conn,
            &graph_id,
            &sid,
            &ph,
            &sj,
            tc,
            &em,
            extraction_cost,
        );
        let _ = conn.execute(
            "UPDATE sessions SET status = 'extracted' WHERE id = ?1",
            rusqlite::params![&sid],
        );
    })
    .await
    .is_none()
    {
        tracing::error!("Failed to store graph for session {}", sid_for_log);
        crate::db::log_error(&format!(
            "Failed to store graph for session {}",
            sid_for_log
        ));
        return;
    }

    tracing::info!(
        "Graph extracted and stored for session {} ({} tokens, cost ${:.6})",
        session.id,
        graph.token_count,
        extraction_cost
    );
}

/// Extract the system prompt from an Anthropic request body.
pub fn extract_anthropic_system(body: &serde_json::Value) -> Option<&str> {
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
pub fn extract_openai_system(body: &serde_json::Value) -> Option<&str> {
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
