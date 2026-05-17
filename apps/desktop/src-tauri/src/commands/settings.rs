//! Settings, proxy control, and onboarding IPC commands. See spec section 7.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;

use crate::db::queries;
use crate::proxy;
use crate::proxy::InterceptState;

#[derive(Debug, Serialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub status: &'static str,
    pub proxy_version: &'static str,
    pub uptime_seconds: u64,
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, Arc<InterceptState>>) -> HashMap<String, String> {
    let db = match state.db.lock() {
        Ok(d) => d,
        Err(_) => {
            // Fall back to spec defaults
            let mut s = HashMap::new();
            s.insert("proxy_port".into(), "4200".into());
            s.insert("session_timeout_minutes".into(), "30".into());
            s.insert("compression_enabled".into(), "true".into());
            s.insert("graph_injection_enabled".into(), "true".into());
            s.insert("graph_max_tokens".into(), "500".into());
            s.insert(
                "anthropic_base_url".into(),
                "https://api.anthropic.com".into(),
            );
            s.insert("openai_base_url".into(), "https://api.openai.com".into());
            s.insert("tier".into(), "free".into());
            s.insert("sessions_saved_this_month".into(), "0".into());
            s.insert("onboarding_complete".into(), "false".into());
            return s;
        }
    };

    let default_keys = [
        ("proxy_port", "4200"),
        ("session_timeout_minutes", "30"),
        ("compression_enabled", "true"),
        ("graph_injection_enabled", "true"),
        ("graph_max_tokens", "500"),
        ("anthropic_base_url", "https://api.anthropic.com"),
        ("openai_base_url", "https://api.openai.com"),
        ("tier", "free"),
        ("sessions_saved_this_month", "0"),
        ("onboarding_complete", "false"),
    ];

    let mut map = HashMap::new();
    for (key, default) in &default_keys {
        let value = queries::get_setting(&db, key)
            .ok()
            .flatten()
            .unwrap_or_else(|| default.to_string());
        map.insert(key.to_string(), value);
    }
    map
}

#[tauri::command]
pub fn update_setting(state: tauri::State<'_, Arc<InterceptState>>, key: String, value: String) {
    if let Ok(db) = state.db.lock() {
        let _ = queries::set_setting(&db, &key, &value);
    }
}

#[tauri::command]
pub fn get_proxy_status(state: tauri::State<'_, Arc<InterceptState>>) -> ProxyStatus {
    ProxyStatus {
        running: true, // proxy is always running while the app is open
        port: state.proxy_port,
        uptime_seconds: state.start_time.elapsed().as_secs(),
    }
}

#[tauri::command]
pub async fn restart_proxy(
    state: tauri::State<'_, Arc<proxy::InterceptState>>,
) -> Result<String, String> {
    tracing::info!("Restarting proxy server...");
    state.trigger_restart().await?;
    // Note: The actual server restart requires the app to recreate the server.
    // This implementation triggers the shutdown; the app would need to restart it.
    // For now, we return success and log that the user should restart the app.
    Ok("Proxy restart signal sent. Please restart the app to complete restart.".to_string())
}

#[tauri::command]
pub fn get_setup_script(state: tauri::State<'_, Arc<InterceptState>>) -> String {
    let port = state.proxy_port;
    if cfg!(windows) {
        format!(
            "# Add this to your PowerShell profile ($PROFILE):\n\
             $sgProxyUrl = 'http://localhost:{port}'\n\
             try {{\n\
             \x20 $sgResponse = Invoke-WebRequest -Uri \"$sgProxyUrl/health\" -TimeoutSec 1 -ErrorAction Stop\n\
             \x20 if ($sgResponse.StatusCode -eq 200) {{\n\
             \x20\x20   $env:ANTHROPIC_BASE_URL = $sgProxyUrl\n\
             \x20\x20   $env:OPENAI_BASE_URL = \"$sgProxyUrl/v1\"\n\
             \x20 }}\n\
             }} catch {{ }}\n\
             \n\
             # What this does:\n\
             # - Proxy running → env vars set → CLI tools use proxy\n\
             # - Proxy closed → vars not set → tools connect directly\n\
             # Open a new terminal (or restart your shell) after adding.\n"
        )
    } else {
        format!(
            "# Add this to ~/.zshrc (or ~/.bashrc):\n\
             if curl -sf http://localhost:{port}/health > /dev/null 2>&1; then\n\
             \x20 export ANTHROPIC_BASE_URL=http://localhost:{port}\n\
             \x20 export OPENAI_BASE_URL=http://localhost:{port}/v1\n\
             fi\n\
             \n\
             # What this does:\n\
             # - Proxy running → env vars set → CLI tools use proxy\n\
             # - Proxy closed → vars not set → tools connect directly\n\
             # Then: source ~/.zshrc (or open a new terminal).\n"
        )
    }
}

#[tauri::command]
pub async fn check_proxy_health(
    state: tauri::State<'_, Arc<InterceptState>>,
) -> Result<HealthStatus, String> {
    // Verify the proxy is actually reachable by making an HTTP request
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;

    let port = state.proxy_port;
    match client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Ok(HealthStatus {
            status: "healthy",
            proxy_version: env!("CARGO_PKG_VERSION"),
            uptime_seconds: state.start_time.elapsed().as_secs(),
        }),
        Ok(response) => {
            tracing::warn!("Proxy health check returned status: {}", response.status());
            Err(format!("Proxy returned status: {}", response.status()))
        }
        Err(e) => {
            tracing::error!("Proxy health check failed: {}", e);
            Err(format!("Proxy not reachable: {}", e))
        }
    }
}

#[derive(serde::Serialize)]
pub struct VenvStatus {
    pub ready: bool,
    pub python_path: Option<String>,
}

/// Check the status of the Python venv for compression.
#[tauri::command]
pub async fn check_venv_status() -> Result<VenvStatus, String> {
    let ready = crate::venv::venv_ready().await;
    let python_path = crate::venv::python_executable().map(|p| p.to_string_lossy().to_string());

    Ok(VenvStatus { ready, python_path })
}

/// Set up the Python venv with Headroom compression.
/// This is called during onboarding to prepare the compression environment.
#[tauri::command]
pub async fn setup_venv() -> Result<String, String> {
    // Check if already ready
    if crate::venv::venv_ready().await {
        return Ok("Headroom compression environment already ready".to_string());
    }

    tracing::info!("Setting up Python venv for Headroom compression");
    crate::venv::setup_venv().await
}

/// Delete all session data from the database and clear in-memory state.
#[tauri::command]
pub async fn delete_all_data(state: tauri::State<'_, Arc<InterceptState>>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        crate::db::queries::delete_all_data(&conn).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Also clear in-memory active sessions
    state.active_sessions.lock().await.clear();
    tracing::info!("All session data deleted");
    Ok(())
}

/// Return the app version string.
#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ── PAC file & system proxy ──────────────────────────────────────────────

/// Resolve the sessiongraph data directory.
fn sessiongraph_dir() -> Option<std::path::PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()?
    } else {
        std::env::var("HOME").ok()?
    };
    Some(std::path::PathBuf::from(home).join(".sessiongraph"))
}

/// Write the PAC (Proxy Auto-Config) file to disk.
/// Returns the file path on success.
pub fn write_pac_file(port: u16) -> Result<std::path::PathBuf, String> {
    let dir = sessiongraph_dir().ok_or("Cannot determine home directory")?;
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("proxy.pac");

    let content = format!(
        r#"function FindProxyForURL(url, host) {{
    // Route AI API traffic through SessionGraph proxy when running.
    // Falls back to DIRECT if the proxy is unavailable (app closed).
    if (shExpMatch(host, "api.anthropic.com") ||
        shExpMatch(host, "api.openai.com") ||
        shExpMatch(host, "*.openai.com") ||
        shExpMatch(host, "openrouter.ai") ||
        shExpMatch(host, "*.openrouter.ai")) {{
        return "PROXY 127.0.0.1:{port}; DIRECT";
    }}
    return "DIRECT";
}}
"#,
    );
    std::fs::write(&path, &content)
        .map_err(|e| format!("Cannot write PAC file: {e}"))?;
    tracing::info!("PAC file written to {}", path.display());
    Ok(path)
}

#[derive(Debug, Serialize)]
pub struct SystemProxyStatus {
    pub enabled: bool,
    pub pac_file_path: String,
}

/// Get the current system proxy status.
#[tauri::command]
pub fn get_system_proxy_status() -> SystemProxyStatus {
    let dir = sessiongraph_dir();
    let pac_file_path = dir
        .as_ref()
        .map(|d| d.join("proxy.pac"))
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let enabled = if cfg!(windows) {
        // Check registry for AutoConfigURL pointing to our PAC file
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "try {{ $v = Get-ItemPropertyValue -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name AutoConfigURL -ErrorAction Stop; if ($v -eq 'file:///{escaped}') {{ 'true' }} else {{ 'false' }} }} catch {{ 'false' }}",
                    escaped = pac_file_path.replace('\'', "''")
                ),
            ])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "true")
            .unwrap_or(false)
    } else {
        false
    };

    SystemProxyStatus {
        enabled,
        pac_file_path,
    }
}

/// Enable or disable the system proxy (PAC file).
#[tauri::command]
pub async fn set_system_proxy(enabled: bool) -> Result<(), String> {
    let dir = sessiongraph_dir().ok_or("Cannot determine home directory")?;
    let pac_path = dir.join("proxy.pac");

    if !pac_path.exists() {
        return Err("PAC file not found. Restart SessionGraph to create it.".to_string());
    }

    let pac_url = format!("file:///{}", pac_path.to_string_lossy());

    if cfg!(windows) {
        let action = if enabled { "Set" } else { "Remove" };
        let cmd = if enabled {
            format!(
                "Set-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name AutoConfigURL -Value '{url}'",
                url = pac_url
            )
        } else {
            "Remove-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name AutoConfigURL -ErrorAction SilentlyContinue".to_string()
        };

        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &cmd])
            .output()
            .map_err(|e| format!("Failed to run PowerShell: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to {action} system proxy: {stderr}"));
        }

        tracing::info!("System proxy {}", if enabled { "enabled" } else { "disabled" });
        Ok(())
    } else {
        // macOS/Linux proxy configuration via networksetup / gsettings
        // Not yet implemented — PRs welcome.
        tracing::warn!("set_system_proxy not implemented on this platform");
        Err("System proxy configuration is only supported on Windows".to_string())
    }
}
