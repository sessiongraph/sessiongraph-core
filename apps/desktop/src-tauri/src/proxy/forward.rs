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

/// Result of a forward request including the response and output token count.
/// The token count can be retrieved after the response body is fully consumed.
pub struct ForwardResult {
    pub response: Response,
    pub token_count: Arc<AtomicU64>,
}

/// Estimates token count from byte count (4 chars ≈ 1 token).
pub fn bytes_to_tokens(bytes: u64) -> u64 {
    bytes.div_ceil(4)
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
pub async fn forward_anthropic(
    body: serde_json::Value,
    api_key: &str,
    base_url: Option<&str>,
) -> Result<ForwardResult, ForwardError> {
    let upstream = base_url
        .unwrap_or("https://api.anthropic.com")
        .trim_end_matches('/')
        .to_string()
        + "/v1/messages";
    stream_post(
        &upstream,
        body,
        Some(("x-api-key", api_key)),
        Some(("anthropic-version", "2023-06-01")),
        &[],
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

    // Create a counter to track bytes as they stream through
    // This is updated in real-time as each chunk is sent
    let byte_counter = Arc::new(AtomicU64::new(0));

    let counter_clone = byte_counter.clone();
    let byte_stream: ByteStream =
        Box::pin(upstream_response.bytes_stream().map(move |r| match r {
            Ok(bytes) => {
                let len = bytes.len() as u64;
                counter_clone.fetch_add(len, Ordering::Relaxed);
                Ok(bytes)
            }
            Err(e) => Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
        }));
    let axum_body = Body::from_stream(byte_stream);

    let response = resp
        .body(axum_body)
        .map_err(|e| ForwardError::BuildResponse(e.to_string()))?;

    Ok(ForwardResult {
        response,
        token_count: byte_counter,
    })
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
    } else if ua.contains("opencode") {
        Some("opencode".into())
    } else if ua.contains("cursor") {
        Some("cursor".into())
    } else if ua.contains("windsurf") || ua.contains("codeium") {
        Some("windsurf".into())
    } else if ua.contains("continue") || ua.contains("continue-dev") {
        Some("continue".into())
    } else if ua.contains("aider") {
        Some("aider".into())
    } else if ua.contains("antigravity") || ua.contains("google-gemini-cli") {
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
        } else if m.contains("llama")
            || m.contains("mistral")
            || m.contains("mixtral")
            || m.contains("qwen")
            || m.contains("yi-")
        {
            (0.15, 0.15) // Open-source models on OpenRouter
        }
        // Default
        else {
            (2.50, 10.0)
        }
    };

    (tokens_in as f64 / 1_000_000.0) * price_in_per_1m
        + (tokens_out as f64 / 1_000_000.0) * price_out_per_1m
}
