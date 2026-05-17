//! Session graph injection. Prepends the prior graph block to the system
//! prompt on the first request of a new session for a known project.
//! See spec section 2.6.
//!
//! If no prior graph exists for this project, or injection is disabled,
//! the request body is returned unchanged.

use rusqlite::Connection;
use serde_json::Value;

use crate::db::queries;

/// The context block marker prepended to the system prompt.
const GRAPH_PREFIX: &str = "[SESSIONGRAPH CONTEXT — Previous session state]\n";
const GRAPH_SUFFIX: &str = "\n[END SESSIONGRAPH CONTEXT]";

/// Result of an injection attempt.
#[derive(Debug)]
pub struct InjectionResult {
    /// The modified request body (or original if no graph found).
    pub body: Value,
    /// Whether a graph was actually injected.
    pub injected: bool,
    /// Token count of the injected graph text.
    pub graph_tokens: u32,
}

/// Try to inject a prior session graph into the request body.
///
/// For Anthropic format: prepends to `body["system"]`.
/// For OpenAI format: prepends to the first `system` role message content.
///
/// `provider` should be `"anthropic"` or `"openai"` to handle the different
/// system prompt locations.
/// `max_tokens` is the token budget — the graph JSON is truncated if it
/// exceeds this budget.
pub fn inject(
    db: &Connection,
    mut body: Value,
    project_hash: &str,
    provider: &str,
    max_tokens: u32,
) -> InjectionResult {
    // Look up the latest graph for this project
    let graph_json: Option<String> = match queries::get_latest_graph_json(db, project_hash) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!("Failed to look up graph for injection: {}", e);
            None
        }
    };

    let Some(graph_json) = graph_json else {
        return InjectionResult {
            body,
            injected: false,
            graph_tokens: 0,
        };
    };

    // Enforce token budget: if the graph exceeds max_tokens, truncate by
    // removing low-priority fields per spec §2.4:
    //   state > decisions > conventions > files > errors > project
    let truncated_json = enforce_graph_budget(&graph_json, max_tokens);

    let context_block = format!("{}{}{}", GRAPH_PREFIX, truncated_json, GRAPH_SUFFIX);
    let graph_tokens = (context_block.chars().count() as u32).div_ceil(4);

    match provider {
        "anthropic" => {
            // Anthropic: system prompt is `"system"` key (string or array)
            let existing_system = body.get("system").cloned();
            body["system"] = build_anthropic_system(existing_system, &context_block);
            InjectionResult {
                body,
                injected: true,
                graph_tokens,
            }
        }
        _ => {
            // OpenAI: system prompt is first message with role="system"
            if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
                if let Some(system_msg) = messages
                    .iter_mut()
                    .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
                {
                    let existing_content = system_msg.get("content").cloned();
                    system_msg["content"] = build_openai_content(existing_content, &context_block);
                } else {
                    // No system message — insert one at the beginning
                    let sys_msg = serde_json::json!({
                        "role": "system",
                        "content": context_block
                    });
                    messages.insert(0, sys_msg);
                }
            }
            InjectionResult {
                body,
                injected: true,
                graph_tokens,
            }
        }
    }
}

/// Build a new Anthropic system prompt by prepending the context block.
/// Handles both string and content-block-array formats.
fn build_anthropic_system(existing: Option<Value>, context_block: &str) -> Value {
    match existing {
        Some(Value::String(s)) => Value::String(format!("{}\n\n{}", context_block, s)),
        Some(Value::Array(blocks)) => {
            // Content block array — prepend a text block
            let mut new_blocks = vec![serde_json::json!({
                "type": "text",
                "text": context_block
            })];
            new_blocks.extend(blocks);
            Value::Array(new_blocks)
        }
        Some(other) => {
            // Unexpected type — stringify and prepend
            Value::String(format!(
                "{}\n\n{}",
                context_block,
                other.as_str().unwrap_or("")
            ))
        }
        None => Value::String(context_block.to_string()),
    }
}

/// Build a new OpenAI system message content by prepending the context block.
/// Handles both string and content-part-array formats.
fn build_openai_content(existing: Option<Value>, context_block: &str) -> Value {
    match existing {
        Some(Value::String(s)) => Value::String(format!("{}\n\n{}", context_block, s)),
        Some(Value::Array(parts)) => {
            let mut new_parts = vec![serde_json::json!({
                "type": "text",
                "text": context_block
            })];
            new_parts.extend(parts);
            Value::Array(new_parts)
        }
        Some(other) => Value::String(format!(
            "{}\n\n{}",
            context_block,
            other.as_str().unwrap_or("")
        )),
        None => Value::String(context_block.to_string()),
    }
}

/// Truncate the session graph JSON to fit within `max_tokens` by removing
/// low-priority fields in order: project, errors, files, conventions, decisions, state.
/// See spec §2.4 priority order: state > decisions > conventions > files > errors > project.
fn enforce_graph_budget(graph_json: &str, max_tokens: u32) -> String {
    let max_chars = max_tokens as usize * 4; // 4 chars ≈ 1 token

    if graph_json.chars().count() <= max_chars {
        return graph_json.to_string();
    }

    // Parse and strip fields
    let mut graph: Value = match serde_json::from_str(graph_json) {
        Ok(v) => v,
        Err(_) => {
            // Can't parse — truncate at nearest char boundary (may break JSON but better than bloating)
            let truncated: String = graph_json.chars().take(max_chars).collect();
            return truncated;
        }
    };

    // Remove fields in reverse priority order until we fit
    let strippable = ["project", "errors", "files", "conventions", "decisions"];
    for field in &strippable {
        if graph.as_object().map_or(true, |o| !o.contains_key(*field)) {
            continue;
        }
        let serialized = serde_json::to_string(&graph).unwrap_or_default();
        if serialized.chars().count() <= max_chars {
            break;
        }
        // Replace with minimal placeholder
        if let Some(obj) = graph.as_object_mut() {
            let replacement = match *field {
                "project" => serde_json::json!({"name": "…"}),
                "decisions" | "errors" => Value::Array(vec![]),
                _ => Value::Object(serde_json::Map::new()),
            };
            obj.insert(field.to_string(), replacement);
        }
    }

    // Final fallback: if still too large after stripping all, hard-truncate
    let result = serde_json::to_string(&graph).unwrap_or_default();
    if result.chars().count() > max_chars {
        result.chars().take(max_chars).collect()
    } else {
        result
    }
}
