//! Upstream forwarding with streaming response (SSE) pass-through.
//! See spec section 5.3 step 5.
//!
//! Supported providers: Anthropic, OpenAI, OpenRouter, and any
//! OpenAI-compatible endpoint. Provider is auto-detected from the API key
//! prefix and headers — no user configuration needed.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

type ByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, Box<dyn std::error::Error + Send + Sync>>> + Send>>;

/// Result of a forward request including the response and real token counts.
/// The counts are populated asynchronously by a background SSE parser task
/// and can be read after the response body is fully consumed.
pub struct ForwardResult {
    pub response: Response,
    pub input_token_count: Arc<AtomicU64>,
    pub output_token_count: Arc<AtomicU64>,
}

// ── Provider detection ────────────────────────────────────────────────────

/// Identified upstream provider.
#[derive(Debug, Clone, PartialEq)]
pub enum Provider {
    Anthropic,
    OpenAI,
    OpenRouter,
    /// Catch-all for any OpenAI-compatible endpoint.
    OpenAICompatible {
        base_url: String,
    },
}

impl Provider {
    pub fn as_str(&self) -> &str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
            Provider::OpenRouter => "openrouter",
            Provider::OpenAICompatible { .. } => "openai-compatible",
        }
    }
}

/// Detect the provider from the Authorization / API key headers.
///
/// Detection rules:
/// - `x-api-key` header present → Anthropic (native Anthropic key format)
/// - `Authorization: Bearer sk-or-v1-...` → OpenRouter
/// - `Authorization: Bearer sk-ant-api03-...` → Anthropic (via Bearer)
/// - `Authorization: Bearer sk-...` → OpenAI
/// - Otherwise → OpenAI (default)
pub fn detect_provider(headers: &HeaderMap) -> Provider {
    // Check for Anthropic native header first
    if headers.contains_key("x-api-key") {
        return Provider::Anthropic;
    }

    // Check Authorization Bearer token
    if let Some(key) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        if key.starts_with("sk-or-v1-") {
            return Provider::OpenRouter;
        }
        if key.starts_with("sk-ant-") {
            return Provider::Anthropic;
        }
        return Provider::OpenAI;
    }

    Provider::OpenAI
}

// ── Forwarding ────────────────────────────────────────────────────────────

/// Forward a request to Anthropic's Messages API.
/// `base_url` overrides the default `https://api.anthropic.com`.
/// `client_headers` are passed through so version and beta headers
/// set by the calling SDK (e.g. Claude Code) reach the real API unchanged.
pub async fn forward_anthropic(
    body: serde_json::Value,
    api_key: &str,
    base_url: Option<&str>,
    client_headers: &HeaderMap,
) -> Result<ForwardResult, ForwardError> {
    let upstream = base_url
        .unwrap_or("https://api.anthropic.com")
        .trim_end_matches('/')
        .to_string()
        + "/v1/messages";

    // Honour the API version the caller requested; fall back to a modern
    // default that accepts all current features (incl. context_management).
    let version = client_headers
        .get("anthropic-version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("2023-06-01");

    let beta = client_headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok());

    let mut extra: Vec<(&str, &str)> = Vec::new();
    if let Some(b) = beta {
        extra.push(("anthropic-beta", b));
    }

    stream_post(
        &upstream,
        body,
        Some(("x-api-key", api_key)),
        Some(("anthropic-version", version)),
        &extra,
    )
    .await
}

/// Forward an OpenAI-format request to the appropriate upstream.
/// Auto-detects the provider from the headers and routes accordingly.
pub async fn forward_openai_compatible(
    body: serde_json::Value,
    api_key: &str,
    provider: &Provider,
    headers: &HeaderMap,
) -> Result<ForwardResult, ForwardError> {
    let upstream_url = resolve_upstream_url(provider);

    let extra_headers: Vec<(&str, &str)> = match provider {
        Provider::OpenRouter => {
            // OpenRouter requires these optional headers for ranking/analytics
            let mut h: Vec<(&str, &str)> = Vec::new();
            if let Some(referer) = headers
                .get("HTTP-Referer")
                .or_else(|| headers.get("http-referer"))
                .and_then(|v| v.to_str().ok())
            {
                h.push(("HTTP-Referer", referer));
            }
            if let Some(title) = headers
                .get("X-Title")
                .or_else(|| headers.get("x-title"))
                .and_then(|v| v.to_str().ok())
            {
                h.push(("X-Title", title));
            }
            // SessionGraph identifies itself
            if !h.iter().any(|(k, _)| *k == "X-Title") {
                h.push(("X-Title", "SessionGraph"));
            }
            h
        }
        _ => Vec::new(),
    };

    let auth_value = format!("Bearer {}", api_key);
    stream_post(
        &upstream_url,
        body,
        Some(("Authorization", &auth_value)),
        None,
        &extra_headers,
    )
    .await
}

/// Low-level streaming POST to an upstream API.
/// Creates a wrapper stream that counts bytes as they pass through to enable
/// accurate output token counting.
///
/// The token count is updated in real-time as bytes stream through. It can be
/// read after the response is fully consumed using the returned token_count.
/// For accurate counts, spawn a background task to read the counter after the
/// stream completes.
async fn stream_post(
    url: &str,
    body: serde_json::Value,
    auth_header: Option<(&str, &str)>,
    extra_single: Option<(&str, &str)>,
    extra_headers: &[(&str, &str)],
) -> Result<ForwardResult, ForwardError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .no_proxy() // never route through system proxy — would loop back to ourselves
        .build()
        .map_err(|e| ForwardError::BuildClient(e.to_string()))?;

    let mut req = client.post(url).json(&body);

    if let Some((k, v)) = auth_header {
        req = req.header(k, v);
    }
    if let Some((k, v)) = extra_single {
        req = req.header(k, v);
    }
    for &(k, v) in extra_headers {
        req = req.header(k, v);
    }

    let upstream_response = req
        .send()
        .await
        .map_err(|e| ForwardError::Upstream(e.to_string()))?;

    let status = upstream_response.status();
    let upstream_headers = upstream_response.headers().clone();

    // Compute hop-by-hop headers per RFC 7230 §6.1
    let static_hop_by_hop: &[&str] = &[
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "transfer-encoding",
        "upgrade",
        "connection",
    ];
    let mut extra_hop_by_hop: Vec<String> = Vec::new();
    // Any header named in the Connection header value must also be stripped
    if let Some(conn_value) = upstream_headers
        .get("connection")
        .and_then(|v| v.to_str().ok())
    {
        for part in conn_value.split(',').map(|s| s.trim()) {
            let lower = part.to_lowercase();
            if !static_hop_by_hop.contains(&lower.as_str()) && !extra_hop_by_hop.contains(&lower) {
                extra_hop_by_hop.push(lower);
            }
        }
    }

    let mut resp = Response::builder().status(status);

    for (key, value) in upstream_headers.iter() {
        let key_lower = key.as_str().to_lowercase();
        if static_hop_by_hop.contains(&key_lower.as_str())
            || extra_hop_by_hop.iter().any(|e| e == &key_lower)
        {
            continue;
        }
        resp = resp.header(key.as_str(), value.as_bytes());
    }

    // Tee the SSE stream: each chunk goes to the client immediately AND to a
    // background channel so we can parse real usage counts without blocking.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();

    let tee_stream: ByteStream = Box::pin(upstream_response.bytes_stream().map(move |r| match r {
        Ok(bytes) => {
            let _ = tx.send(bytes.clone());
            Ok(bytes)
        }
        Err(e) => Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
    }));
    let axum_body = Body::from_stream(tee_stream);

    let input_tokens = Arc::new(AtomicU64::new(0));
    let output_tokens = Arc::new(AtomicU64::new(0));
    let input_clone = input_tokens.clone();
    let output_clone = output_tokens.clone();

    // Background task: drain the tee channel, concatenate all chunks, then
    // parse the real input/output token counts from the SSE events.
    tokio::spawn(async move {
        let mut buf = Vec::new();
        while let Some(chunk) = rx.recv().await {
            buf.extend_from_slice(&chunk);
        }
        let (inp, out) = parse_sse_usage(&buf);
        if inp > 0 {
            input_clone.store(inp, Ordering::Relaxed);
        }
        if out > 0 {
            output_clone.store(out, Ordering::Relaxed);
        }
    });

    let response = resp
        .body(axum_body)
        .map_err(|e| ForwardError::BuildResponse(e.to_string()))?;

    Ok(ForwardResult {
        response,
        input_token_count: input_tokens,
        output_token_count: output_tokens,
    })
}

/// Parse real input/output token counts from a complete SSE response body.
///
/// Handles both Anthropic and OpenAI/compatible SSE formats:
/// - Anthropic: `data:` lines contain JSON with `usage.input_tokens` /
///   `usage.output_tokens`. The final `message_delta` event carries
///   cumulative output_tokens; the `message_start` event carries input_tokens.
/// - OpenAI: the last non-`[DONE]` `data:` line before `data: [DONE]` contains
///   `usage.prompt_tokens` (input) and `usage.completion_tokens` (output).
///
/// The function returns the last non-zero values found so that the final
/// (most accurate) usage event always wins.
pub fn parse_sse_usage(buf: &[u8]) -> (u64, u64) {
    let text = String::from_utf8_lossy(buf);
    let mut best_input: u64 = 0;
    let mut best_output: u64 = 0;

    for line in text.lines() {
        let line = line.trim();
        let json_str = if let Some(rest) = line.strip_prefix("data:") {
            rest.trim()
        } else {
            continue;
        };

        // Skip the OpenAI stream terminator
        if json_str == "[DONE]" {
            continue;
        }

        let obj: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let usage = match obj.get("usage") {
            Some(u) if u.is_object() => u,
            _ => continue,
        };

        // OpenAI / OpenAI-compatible: prompt_tokens + completion_tokens
        if let (Some(inp), Some(out)) = (
            usage.get("prompt_tokens").and_then(|v| v.as_u64()),
            usage.get("completion_tokens").and_then(|v| v.as_u64()),
        ) {
            if inp > 0 {
                best_input = inp;
            }
            if out > 0 {
                best_output = out;
            }
            continue;
        }

        // Anthropic: input_tokens (message_start) or output_tokens (message_delta/stop)
        if let Some(inp) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
            if inp > 0 {
                best_input = inp;
            }
        }
        if let Some(out) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
            if out > 0 {
                best_output = out;
            }
        }
    }

    (best_input, best_output)
}

/// Resolve the upstream URL for a detected provider.
fn resolve_upstream_url(provider: &Provider) -> String {
    match provider {
        Provider::OpenRouter => "https://openrouter.ai/api/v1/chat/completions".into(),
        Provider::OpenAICompatible { base_url } => {
            format!("{}/v1/chat/completions", base_url.trim_end_matches('/'))
        }
        _ => "https://api.openai.com/v1/chat/completions".into(),
    }
}

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ForwardError {
    BuildClient(String),
    Upstream(String),
    BuildResponse(String),
}

impl IntoResponse for ForwardError {
    fn into_response(self) -> Response {
        tracing::error!("Forward error: {:?}", self);
        let (status, msg) = match &self {
            ForwardError::BuildClient(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "proxy: failed to build HTTP client",
            ),
            ForwardError::Upstream(_) => (
                StatusCode::BAD_GATEWAY,
                "proxy: upstream provider unreachable",
            ),
            ForwardError::BuildResponse(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "proxy: failed to build response",
            ),
        };
        let body = serde_json::json!({"error": msg});
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap_or_default()))
            .unwrap()
    }
}

// ── Token estimation ──────────────────────────────────────────────────────

/// Approximate token count from a request body (4 chars ≈ 1 token).
pub fn estimate_tokens(body: &serde_json::Value) -> u64 {
    let json_str = serde_json::to_string(body).unwrap_or_default();
    (json_str.chars().count() as u64).div_ceil(4)
}

// ── API key extraction ────────────────────────────────────────────────────

/// Extract the API key from request headers for the given provider.
pub fn extract_api_key(headers: &HeaderMap, provider: &Provider) -> Option<String> {
    match provider {
        Provider::Anthropic => headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .or_else(|| {
                // Anthropic keys can also come via Bearer (e.g., when using OpenRouter-style tools)
                headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .map(String::from)
            })
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()),
        Provider::OpenRouter | Provider::OpenAI | Provider::OpenAICompatible { .. } => headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(String::from)
            .or_else(|| {
                if matches!(provider, Provider::OpenRouter) {
                    std::env::var("OPENROUTER_API_KEY").ok()
                } else {
                    None
                }
            })
            .or_else(|| std::env::var("OPENAI_API_KEY").ok()),
    }
}

// ── Model extraction ──────────────────────────────────────────────────────

/// Extract the model name from the request body.
pub fn extract_model(body: &serde_json::Value) -> String {
    body.get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

// ── Tool detection ────────────────────────────────────────────────────────

/// Detect the AI coding tool from the User-Agent header and provider.
pub fn detect_tool(headers: &HeaderMap, provider: &Provider) -> Option<String> {
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    if ua.contains("claude-code") || ua.contains("claudecli") {
        Some("claude-code".into())
    } else if ua.contains("opencode") || ua.contains("opecode") {
        Some("opencode".into())
    } else if ua.contains("cursor") {
        Some("cursor".into())
    } else if ua.contains("windsurf") || ua.contains("codeium") {
        Some("windsurf".into())
    } else if ua.contains("continue") || ua.contains("continue-dev") {
        Some("continue".into())
    } else if ua.contains("aider") {
        Some("aider".into())
    } else if ua.contains("antigravity")
        || ua.contains("google-gemini-cli")
        || ua.contains("gemini-cli")
        || ua.contains("google-cloud-sdk")
    {
        Some("antigravity".into())
    } else if ua.contains("cline") {
        Some("cline".into())
    } else if ua.is_empty() && *provider == Provider::Anthropic {
        Some("claude-code".into())
    } else if *provider == Provider::OpenRouter {
        Some("openrouter".into())
    } else {
        None
    }
}

// ── Cost estimation ───────────────────────────────────────────────────────

/// Compute approximate cost in USD from token counts and model.
/// Prices are approximate; refresh before launch.
pub fn compute_cost(model: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    let (price_in_per_1m, price_out_per_1m): (f64, f64) = {
        let m = model.to_lowercase();

        // Anthropic models
        if m.contains("claude-sonnet-4")
            || m.contains("claude-3-5-sonnet")
            || m.contains("claude-3.5-sonnet")
        {
            (3.0, 15.0)
        } else if m.contains("claude-3-5-haiku") || m.contains("claude-3.5-haiku") {
            (0.80, 4.0)
        } else if m.contains("claude-3-opus") || m.contains("claude-opus") {
            (15.0, 75.0)
        } else if m.contains("claude-3-haiku") || m.contains("claude-haiku") {
            (0.25, 1.25)
        } else if m.contains("claude") {
            (3.0, 15.0)
        }
        // OpenAI models
        else if m.contains("gpt-4o") {
            (2.50, 10.0)
        } else if m.contains("gpt-4-turbo") {
            (10.0, 30.0)
        } else if m.contains("gpt-4") {
            (30.0, 60.0)
        } else if m.contains("gpt-3.5") {
            (0.50, 1.50)
        } else if m.contains("o3") || m.contains("o1") {
            (15.0, 60.0)
        }
        // DeepSeek models (direct or via OpenRouter)
        else if m.contains("deepseek") && (m.contains("reasoner") || m.contains("r1")) {
            (0.55, 2.19) // DeepSeek-R1
        } else if m.contains("deepseek") {
            (0.27, 1.10) // DeepSeek-V3 / deepseek-chat
        } else if m.contains("gemini") && (m.contains("flash") || m.contains("1.5")) {
            (0.075, 0.30) // Gemini Flash via OpenRouter
        } else if m.contains("gemini") {
            (1.25, 5.0) // Gemini Pro
                        // MiniMax models
        } else if m.contains("abab6.5s") {
            (0.10, 0.10)
        } else if m.contains("abab5.5") {
            (0.15, 0.15)
        }
        // Qwen (Alibaba) and GLM (Zhipu AI) models — same pricing tier
        else if m.contains("qwen") || m.contains("glm") {
            (0.50, 1.50)
        }
        // Open-source models on OpenRouter / together.ai / etc.
        else if m.contains("llama")
            || m.contains("mistral")
            || m.contains("mixtral")
            || m.contains("yi-")
        {
            (0.15, 0.15)
        }
        // Default
        else {
            (2.50, 10.0)
        }
    };

    (tokens_in as f64 / 1_000_000.0) * price_in_per_1m
        + (tokens_out as f64 / 1_000_000.0) * price_out_per_1m
}
