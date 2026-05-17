//! Headroom compression subprocess caller. See spec section 5.4.
//!
//! Spawns `~/.sessiongraph/venv/Scripts/headroom-compress.py` (or the
//! Unix equivalent) as a subprocess, pipes the request messages via stdin,
//! and reads the compressed result from stdout.
//!
//! If the subprocess fails for ANY reason (crashes, times out, returns
//! malformed output), the original uncompressed messages are returned
//! unchanged.  Compression failure MUST never break the proxy.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    messages: &'a Vec<serde_json::Value>,
    model: &'a str,
}

/// Try to compress messages via the Headroom Python subprocess.
///
/// Returns `Some(CompressOutput)` with the compressed messages and metrics on
/// success, or `None` if compression fails (graceful fallback to original).
pub async fn compress(
    messages: &[serde_json::Value],
    model: &str,
) -> Option<CompressOutput> {
    let script_path = compress_script_path()?;

    let input = CompressInput {
        messages: &messages.to_vec(),
        model,
    };

    let input_json = serde_json::to_string(&input).ok()?;

    let result = tokio::process::Command::new("python")
        .arg(&script_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    let mut child = result;

    // Write JSON to stdin
    use tokio::io::AsyncWriteExt;
    if let Some(ref mut stdin) = child.stdin {
        if stdin.write_all(input_json.as_bytes()).await.is_err() {
            tracing::warn!("Compression: failed to write to subprocess stdin");
            let _ = child.kill().await;
            return None;
        }
    }
    // (stdin handle dropped here — signals EOF to Python)

    // Read output with a 15-second timeout
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        child.wait_with_output(),
    )
    .await;

    let output = match output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            tracing::warn!("Compression: subprocess error: {}", e);
            return None;
        }
        Err(_elapsed) => {
            tracing::warn!("Compression: subprocess timed out after 15s");
            // Try to kill the hung process
            // (child is moved into wait_with_output, so we can't easily kill it here)
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
            None
        }
    }
}

/// Resolve the path to the compression Python script.
fn compress_script_path() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()?
    } else {
        std::env::var("HOME").ok()?
    };

    let script = if cfg!(windows) {
        PathBuf::from(&home)
            .join(".sessiongraph")
            .join("venv")
            .join("Scripts")
            .join("headroom-compress.py")
    } else {
        PathBuf::from(&home)
            .join(".sessiongraph")
            .join("venv")
            .join("bin")
            .join("headroom-compress.py")
    };

    if script.exists() {
        Some(script)
    } else {
        tracing::warn!("Compression script not found at {}", script.display());
        None
    }
}
