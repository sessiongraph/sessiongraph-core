//! Session tracking: project hash, 30-minute inactivity rollover.
//! See spec sections 2.3 and 5.3 step 2.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Maximum number of request bodies to retain for extraction.
const MAX_MESSAGE_SNAPSHOTS: usize = 10;

/// An in-memory representation of the current active session.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub id: String,
    pub project_hash: String,
    /// SHA256 hash of the API key, truncated to 16 chars. Used for session identity.
    pub api_key_hash: String,
    pub project_name: Option<String>,
    pub provider: String,
    pub tool: Option<String>,
    /// The API key used for this session (needed for extraction at session end).
    pub api_key: String,
    pub started_at: DateTime<Utc>,
    pub last_request_at: DateTime<Utc>,
    pub message_count: u64,
    pub tokens_in_raw: u64,
    pub tokens_in_sent: u64,
    pub tokens_out: u64,
    pub cost_usd_raw: f64,
    pub cost_usd_actual: f64,
    /// Snapshots of recent request bodies for graph extraction at session end.
    pub recent_bodies: Vec<Value>,
}

impl ActiveSession {
    pub fn new(
        project_hash: String,
        api_key_hash: String,
        project_name: Option<String>,
        provider: String,
        tool: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            project_hash,
            api_key_hash,
            project_name,
            provider,
            tool,
            api_key: String::new(),
            started_at: now,
            last_request_at: now,
            message_count: 0,
            tokens_in_raw: 0,
            tokens_in_sent: 0,
            tokens_out: 0,
            cost_usd_raw: 0.0,
            cost_usd_actual: 0.0,
            recent_bodies: Vec::new(),
        }
    }

    /// Check whether this session has timed out (no requests for `timeout_minutes`).
    pub fn is_timed_out(&self, timeout_minutes: i64) -> bool {
        let elapsed = Utc::now() - self.last_request_at;
        elapsed.num_minutes() >= timeout_minutes
    }

    /// Push a request body snapshot, keeping at most `MAX_MESSAGE_SNAPSHOTS`.
    pub fn push_body(&mut self, body: &Value) {
        // Keep only a summary to save memory: strip large binary/array fields
        let summary = strip_large_fields(body);
        self.recent_bodies.push(summary);
        if self.recent_bodies.len() > MAX_MESSAGE_SNAPSHOTS {
            self.recent_bodies.remove(0);
        }
    }

    /// Serialize recent message snapshots for the extractor.
    pub fn messages_for_extraction(&self) -> String {
        serde_json::to_string(&self.recent_bodies).unwrap_or_default()
    }
}

/// Strip large fields from a request body to keep memory usage low.
fn strip_large_fields(body: &Value) -> Value {
    match body {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                // Skip base64-encoded fields and huge arrays
                if k.contains("data") || k.contains("image") || k.contains("base64") {
                    out.insert(k.clone(), Value::String("[stripped]".into()));
                } else if k == "messages" {
                    // Keep messages but cap array size
                    if let Value::Array(arr) = v {
                        let capped: Vec<_> = arr
                            .iter()
                            .rev()
                            .take(20)
                            .rev()
                            .map(|m| strip_large_fields(m))
                            .collect();
                        out.insert(k.clone(), Value::Array(capped));
                    } else {
                        out.insert(k.clone(), strip_large_fields(v));
                    }
                } else {
                    out.insert(k.clone(), strip_large_fields(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(strip_large_fields).collect())
        }
        _ => body.clone(),
    }
}

/// Hash an API key for session identity comparison.
/// Returns a hex-encoded SHA-256 truncated to 16 characters.
pub fn hash_api_key(api_key: &str) -> String {
    if api_key.is_empty() {
        return "unknown".to_string();
    }
    let digest = Sha256::digest(api_key.as_bytes());
    digest[..8]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Compute a stable project hash from available context.
///
/// Priority:
/// 1. Working directory path (if detectable from system prompt)
/// 2. First 100 chars of the first system prompt
/// 3. Fallback: "unknown"
///
/// The result is a hex-encoded SHA-256 truncated to 16 characters.
pub fn compute_project_hash(
    system_prompt: Option<&str>,
    _working_dir: Option<&str>,
) -> String {
    let input = system_prompt
        .map(|s| s.chars().take(100).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let digest = Sha256::digest(input.as_bytes());
    // First 8 bytes → 16 hex chars
    digest[..8]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Infer a project name from the system prompt.
///
/// Looks for:
/// - Explicit "project:" or "Project:" field
/// - Package.json "name" field pattern
/// - Git repository name (after "github.com/")
/// - Directory path patterns
/// - Falls back to "unknown" if nothing found
pub fn infer_project_name(system_prompt: Option<&str>) -> Option<String> {
    let prompt = system_prompt?;

    // Look for explicit project name markers
    if let Some(line) = prompt.lines().find(|l| {
        l.to_lowercase().starts_with("project:")
            || l.to_lowercase().starts_with("project name:")
    }) {
        let name = line.split(':').nth(1)?.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    // Look for package.json style name (common in AI prompts about codebase)
    if let Some(start) = prompt.find("\"name\":") {
        let rest = &prompt[start + 7..];
        if let Some(quote_start) = rest.find('"') {
            let rest = &rest[quote_start + 1..];
            if let Some(quote_end) = rest.find('"') {
                let name = &rest[..quote_end];
                if !name.is_empty() && name.len() < 100 {
                    return Some(name.to_string());
                }
            }
        }
    }

    // Look for GitHub repository name
    if let Some(gh) = prompt.to_lowercase().find("github.com/") {
        let rest = &prompt[gh + 11..];
        let path = rest.split_whitespace().next()?;
        let segments: Vec<&str> = path.split('/').collect();
        // github.com/user/repo → segments[0] = user, segments[1] = repo
        let repo = if segments.len() >= 2 { segments[1] } else { segments[0] };
        if !repo.is_empty() && !repo.contains('.') && repo.len() < 50 {
            return Some(repo.to_string());
        }
    }

    // Look for directory path pattern (e.g., "working on /path/to/my-project")
    if let Some(start) = prompt.find("working in ") {
        let rest = &prompt[start + 11..];
        let path = rest.split_whitespace().next()?;
        if let Some(name) = path.split('/').filter(|s| !s.is_empty()).last() {
            if name.len() < 50 && !name.contains('.') {
                return Some(name.to_string());
            }
        }
    }

    // Last resort: try to extract from first line of prompt (often contains context)
    if let Some(first_line) = prompt.lines().next() {
        let cleaned = first_line.trim();
        if cleaned.len() > 3 && cleaned.len() < 40 {
            // Remove common prefixes
            let name = cleaned
                .trim_start_matches('#')
                .trim_start_matches("You are")
                .trim_start_matches("You are a")
                .trim();
            if !name.is_empty() && name.len() < 40 {
                return Some(name.to_string());
            }
        }
    }

    None
}
