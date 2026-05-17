//! Headroom compression subprocess caller. See spec section 5.4.
//!
//! Invokes `~/.sessiongraph/venv/bin/python -m headroom.compress` (or the
//! Windows equivalent) as a subprocess, pipes the request messages via stdin,
//! and reads the compressed result from stdout.
//!
//! If the subprocess fails for ANY reason (crashes, times out, returns
//! malformed output), the original uncompressed messages are returned
//! unchanged.  Compression failure MUST never break the proxy.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Output from the Python compression subprocess.
#[derive(Debug, Clone, Deserialize)]
pub struct CompressOutput {
    pub messages: Vec<serde_json::Value>,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub tokens_saved: u64,
    pub compression_ratio: f64,
}

/// Input sent to the Python compression subprocess.
#[derive(Debug, Serialize)]
struct CompressInput<'a> {
    messages: &'a [serde_json::Value],
    model: &'a str,
}

/// Try to compress messages via the Headroom Python subprocess.
///
/// Returns `Some(CompressOutput)` with the compressed messages and metrics on
/// success, or `None` if compression fails (graceful fallback to original).
pub async fn compress(messages: &[serde_json::Value], model: &str) -> Option<CompressOutput> {
    let python_path = venv_python_path()?;

    let input = CompressInput { messages, model };

    let input_json = serde_json::to_string(&input).ok()?;

    // Write input JSON to a temp file — avoids leaking request data via
    // command-line arguments (ps aux / Process Explorer).
    let temp_dir = std::env::temp_dir().join("sessiongraph-compress");
    let _ = std::fs::create_dir_all(&temp_dir);
    let temp_file = temp_dir.join(format!("{}.json", Uuid::new_v4()));
    if std::fs::write(&temp_file, &input_json).is_err() {
        tracing::warn!("Compression: failed to write temp input file");
        return None;
    }

    // Ensure the Python wrapper script exists
    if let Err(e) = ensure_wrapper_exists() {
        tracing::warn!("Compression: failed to create wrapper script: {}", e);
        let _ = std::fs::remove_file(&temp_file);
        return None;
    }

    let result = tokio::process::Command::new(&python_path)
        .args([
            wrapper_script_path()?.to_string_lossy().as_ref(),
            &temp_file.to_string_lossy(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    let child = result;

    // Read output with a 15-second timeout
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(15), child.wait_with_output()).await;

    // Clean up temp file regardless of outcome
    let _ = std::fs::remove_file(&temp_file);

    let output = match output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            tracing::warn!("Compression: subprocess error: {}", e);
            crate::db::log_error(&format!("Compression subprocess error: {}", e));
            return None;
        }
        Err(_elapsed) => {
            tracing::warn!("Compression: subprocess timed out after 15s");
            crate::db::log_error("Compression subprocess timed out after 15s");
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            "Compression: subprocess exited with {}: {}",
            output.status,
            stderr.trim()
        );
        crate::db::log_error(&format!(
            "Compression exited with {}: {}",
            output.status,
            stderr.trim()
        ));
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<CompressOutput>(&stdout) {
        Ok(compressed) => {
            tracing::debug!(
                "Compression: {} → {} tokens (saved {}, {:.1}%)",
                compressed.tokens_before,
                compressed.tokens_after,
                compressed.tokens_saved,
                compressed.compression_ratio * 100.0,
            );
            Some(compressed)
        }
        Err(e) => {
            tracing::warn!("Compression: failed to parse output: {}", e);
            crate::db::log_error(&format!("Compression output parse error: {}", e));
            None
        }
    }
}

/// Return the path to the Python wrapper script used for compression.
fn wrapper_script_path() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()?
    } else {
        std::env::var("HOME").ok()?
    };
    Some(
        PathBuf::from(home)
            .join(".sessiongraph")
            .join("compress_wrapper.py"),
    )
}

/// Ensure the Python wrapper script exists, creating it if necessary.
/// The wrapper reads the input JSON from a file (first CLI arg) and relays
/// it to headroom.compress, keeping the payload data out of process argv.
fn ensure_wrapper_exists() -> Result<(), String> {
    let path = wrapper_script_path().ok_or("Cannot determine home directory")?;
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create dir: {e}"))?;
    }
    std::fs::write(
        &path,
        r#"import json, sys, pathlib

# Read the input JSON from a temp file (first CLI arg).
input_file = pathlib.Path(sys.argv[1])
payload = json.loads(input_file.read_text(encoding='utf-8'))

messages = payload["messages"]
model = payload.get("model", "claude-sonnet-4-5-20250929")

from headroom import compress
result = compress(messages, model=model)

out = {
    "messages": result.messages,
    "tokens_before": result.tokens_before,
    "tokens_after": result.tokens_after,
    "tokens_saved": result.tokens_saved,
    "compression_ratio": result.compression_ratio,
}
print(json.dumps(out))
"#,
    )
    .map_err(|e| format!("Cannot write wrapper: {e}"))?;
    Ok(())
}

/// Resolve the path to the Python executable inside the SessionGraph venv.
fn venv_python_path() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()?
    } else {
        std::env::var("HOME").ok()?
    };

    let python = if cfg!(windows) {
        PathBuf::from(&home)
            .join(".sessiongraph")
            .join("venv")
            .join("Scripts")
            .join("python.exe")
    } else {
        PathBuf::from(&home)
            .join(".sessiongraph")
            .join("venv")
            .join("bin")
            .join("python")
    };

    if python.exists() {
        Some(python)
    } else {
        tracing::warn!("Compression: Python venv not found at {}", python.display());
        None
    }
}
