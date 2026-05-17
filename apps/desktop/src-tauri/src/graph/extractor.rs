//! Session graph extraction. Runs at session end via a low-cost LLM call.
//! See spec section 2.5.
//!
//! The extractor takes the last N messages from a completed session, sends
//! them to the cheapest available model, and asks it to populate the
//! canonical SessionGraph JSON schema.

use anyhow::Context;
use serde_json::Value;

use super::schema::SessionGraph;

/// System prompt for the extraction model. Asks for a structured JSON output.
const EXTRACTION_SYSTEM: &str = r#"You are a session state extractor. Given a conversation between a developer and an AI coding assistant, extract a structured session graph in the exact JSON format provided below. Be maximally concise — every field must be under 20 words. Focus on what would be most useful for resuming this exact work in a future session. Return only valid JSON, no other text."#;

/// Maximum input tokens to send to the extraction model (≈ 8000 tokens of
/// conversation context, or roughly 32k characters).
const MAX_INPUT_CHARS: usize = 32_000;

/// The cheapest model names per provider (used for extraction to keep cost
/// under $0.001 per session).
fn cheapest_model(provider: &str) -> &str {
    match provider {
        "anthropic" => "claude-3-haiku-20240307",
        _ => "gpt-4o-mini",
    }
}

/// The system prompt we send to the extraction model, including the
/// SessionGraph JSON schema as a template.
fn extraction_system_prompt() -> String {
    let schema_json = serde_json::json!({
        "sg_version": "1.0",
        "session_id": "<session-id>",
        "project_hash": "<project-hash>",
        "project": {
            "name": "<inferred project name>",
            "stack": ["<tech>"],
            "entry_points": ["<file>"],
            "package_manager": "<pkg>"
        },
        "state": {
            "current_task": "<brief>",
            "progress": "<what was completed>",
            "next_steps": ["<step>"],
            "blockers": ["<blocker>"]
        },
        "decisions": [
            { "topic": "<topic>", "decision": "<what>", "rationale": "<why>" }
        ],
        "conventions": {
            "naming": "<naming conventions>",
            "structure": "<file/folder conventions>",
            "patterns": ["<pattern>"]
        },
        "files": {
            "active": ["<file>"],
            "read": ["<file>"],
            "created": ["<file>"]
        },
        "errors": [
            { "file": "<file>", "description": "<error>", "resolution": "<how fixed or null>" }
        ]
    });

    format!(
        "{}\n\nJSON schema to populate:\n{}",
        EXTRACTION_SYSTEM,
        serde_json::to_string_pretty(&schema_json).unwrap_or_default()
    )
}

/// Run extraction for an Anthropic session. Returns the parsed SessionGraph
/// on success, or `None` if extraction fails (graceful degradation).
pub async fn extract_anthropic(
    api_key: &str,
    session_id: &str,
    project_hash: &str,
    messages_json: &str,
) -> Option<SessionGraph> {
    let model = cheapest_model("anthropic");

    // Build the extraction body
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "system": extraction_system_prompt(),
        "messages": [
            {
                "role": "user",
                "content": format!(
                    "Conversation to extract from:\n{}",
                    truncate_text(messages_json, MAX_INPUT_CHARS)
                )
            }
        ]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .ok()?;

    let json: Value = response.json().await.ok()?;

    // Extract the text content from Anthropic's response format
    let text = json
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| blocks.first())
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())?;

    // Parse the JSON from the model's response
    parse_and_validate_graph(text, session_id, project_hash)
}

/// Run extraction for an OpenAI-compatible session.
pub async fn extract_openai(
    api_key: &str,
    session_id: &str,
    project_hash: &str,
    messages_json: &str,
) -> Option<SessionGraph> {
    let model = cheapest_model("openai");

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [
            {
                "role": "system",
                "content": extraction_system_prompt()
            },
            {
                "role": "user",
                "content": format!(
                    "Conversation to extract from:\n{}",
                    truncate_text(messages_json, MAX_INPUT_CHARS)
                )
            }
        ]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .ok()?;

    let json: Value = response.json().await.ok()?;

    // Extract from OpenAI's response format
    let text = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|choices| choices.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())?;

    parse_and_validate_graph(text, session_id, project_hash)
}

/// Parse the LLM's output as SessionGraph JSON and sanitize required fields.
fn parse_and_validate_graph(
    text: &str,
    session_id: &str,
    project_hash: &str,
) -> Option<SessionGraph> {
    // Strip markdown code fences if the model wraps the JSON
    let cleaned = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s))
        .unwrap_or(text)
        .trim();

    let mut graph: SessionGraph =
        serde_json::from_str(cleaned).with_context(|| "failed to parse extraction JSON").ok()?;

    // Override fields we own — the model may hallucinate these
    graph.session_id = session_id.to_string();
    graph.project_hash = project_hash.to_string();
    graph.sg_version = "1.0".into();
    graph.created_at = chrono::Utc::now().to_rfc3339();
    graph.last_updated = chrono::Utc::now().to_rfc3339();

    // Count tokens in the resulting graph JSON
    let json_str = serde_json::to_string(&graph).unwrap_or_default();
    graph.token_count = (json_str.chars().count() as u32).div_ceil(4);

    Some(graph)
}

/// Truncate text to roughly `max_chars` characters, keeping the last portion
/// (most recent messages are at the end).
fn truncate_text(text: &str, max_chars: usize) -> &str {
    if text.len() <= max_chars {
        return text;
    }
    // Keep the trailing portion (most recent conversation)
    let start = text.len() - max_chars;
    // Walk back to a UTF-8 char boundary
    let mut idx = start;
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    &text[idx..]
}
