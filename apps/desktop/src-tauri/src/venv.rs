//! Python virtual environment setup for Headroom compression.
//! See spec section 5.4.
//!
//! Sets up an isolated Python venv at ~/.sessiongraph/venv/ and installs
//! headroom-ai if not already present.

use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

/// Resolve the SessionGraph venv directory.
pub fn venv_dir() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()?
    } else {
        std::env::var("HOME").ok()?
    };
    Some(PathBuf::from(home).join(".sessiongraph").join("venv"))
}

/// Check if the venv already exists and has headroom installed.
pub async fn venv_ready() -> bool {
    let Some(venv) = venv_dir() else {
        return false;
    };

    let python = if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    };

    if !python.exists() {
        return false;
    }

    // Check if headroom is importable
    let output = Command::new(&python)
        .args(["-c", "import headroom"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await;

    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

/// Set up the Python venv and install headroom-ai.
/// Returns Ok on success, Err with message on failure.
pub async fn setup_venv() -> Result<String, String> {
    let Some(base_dir) = venv_dir() else {
        return Err("Could not determine home directory".to_string());
    };

    let venv_path = base_dir.to_string_lossy().to_string();

    // Create .sessiongraph directory if it doesn't exist
    if let Some(parent) = base_dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create sessiongraph directory: {}", e))?;
    }

    tracing::info!("Setting up Python venv at {}", venv_path);

    // Check if Python is available
    let python_check = Command::new("python")
        .args(["--version"])
        .output()
        .await
        .map_err(|e| format!("Python not found: {}", e))?;

    if !python_check.status.success() {
        return Err("Python not installed or not in PATH".to_string());
    }

    // Create virtual environment
    let create_output = Command::new("python")
        .args(["-m", "venv", &venv_path])
        .output()
        .await
        .map_err(|e| format!("Failed to create venv: {}", e))?;

    if !create_output.status.success() {
        let stderr = String::from_utf8_lossy(&create_output.stderr);
        return Err(format!("venv creation failed: {}", stderr));
    }

    // Install headroom-ai
    let pip_executable = if cfg!(windows) {
        base_dir.join("Scripts").join("pip.exe")
    } else {
        base_dir.join("bin").join("pip")
    };

    let pip_path = pip_executable.to_string_lossy().to_string();

    let pip_output = Command::new(&pip_path)
        .args(["install", "headroom-ai"])
        .output()
        .await
        .map_err(|e| format!("Failed to install headroom: {}", e))?;

    if !pip_output.status.success() {
        let stderr = String::from_utf8_lossy(&pip_output.stderr);
        return Err(format!("headroom installation failed: {}", stderr));
    }

    tracing::info!("Python venv setup complete with headroom-ai");
    Ok("Python environment ready with Headroom compression".to_string())
}

/// Get the path to the headroom compression script.
/// Returns the path to headroom-compress.py or equivalent.
pub fn compression_script() -> Option<PathBuf> {
    let venv = venv_dir()?;

    let script = if cfg!(windows) {
        venv.join("Scripts").join("headroom-compress.py")
    } else {
        venv.join("bin").join("headroom-compress.py")
    };

    if script.exists() {
        Some(script)
    } else {
        // Fallback: try pip-installed headroom module
        None
    }
}

/// Returns the Python executable path in the venv.
pub fn python_executable() -> Option<PathBuf> {
    let venv = venv_dir()?;

    let python = if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    };

    if python.exists() {
        Some(python)
    } else {
        None
    }
}