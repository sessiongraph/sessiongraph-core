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

/// Generate the CLI health-check snippet for a given port.
fn cli_snippet(port: u16) -> String {
    if cfg!(windows) {
        format!(
            "\n# SessionGraph auto-detect (proxy running -> use it; closed -> direct)\n\
             $sgProxyUrl = 'http://localhost:{port}'\n\
             try {{\n\
             \x20 $sgResponse = Invoke-WebRequest -Uri \"$sgProxyUrl/health\" -TimeoutSec 1 -ErrorAction Stop\n\
             \x20 if ($sgResponse.StatusCode -eq 200) {{\n\
             \x20\x20   $env:ANTHROPIC_BASE_URL = $sgProxyUrl\n\
             \x20\x20   $env:OPENAI_BASE_URL = \"$sgProxyUrl/v1\"\n\
             \x20 }}\n\
             }} catch {{ }}\n"
        )
    } else {
        format!(
            "\n# SessionGraph auto-detect (proxy running -> use it; closed -> direct)\n\
             if curl -sf http://localhost:{port}/health > /dev/null 2>&1; then\n\
             \x20 export ANTHROPIC_BASE_URL=http://localhost:{port}\n\
             \x20 export OPENAI_BASE_URL=http://localhost:{port}/v1\n\
             fi\n"
        )
    }
}

/// Set persistent user env vars pointing AI tools to the proxy.
/// Written to HKCU\Environment so new processes auto-discover the proxy.
/// Called when proxy starts, removed when proxy stops (see `remove_proxy_env_vars`).
pub fn set_proxy_env_vars(port: u16) {
    if !cfg!(windows) {
        return;
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let proxy_url = format!("http://localhost:{port}");
    let proxy_str = proxy_url.as_str();
    let openai_url = format!("http://localhost:{port}/v1");
    let openai_str = openai_url.as_str();
    let pairs: [(&str, &str); 4] = [
        ("ANTHROPIC_BASE_URL", proxy_str),
        ("OPENAI_BASE_URL", openai_str),
        ("HTTPS_PROXY", proxy_str),
        ("HTTP_PROXY", proxy_str),
    ];
    let mut ok = true;
    for (name, value) in &pairs {
        let status = std::process::Command::new("setx")
            .args([name, value])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                tracing::warn!(
                    "setx {name}={value} failed with exit code {}",
                    s.code().unwrap_or(-1)
                );
                ok = false;
            }
            Err(e) => {
                tracing::warn!("setx {name}={value} could not be launched: {e}");
                ok = false;
            }
        }
    }
    if ok {
        tracing::info!("Proxy env vars set for port {port}");
    }
}

/// Remove persistent user env vars for the proxy.
/// Called when proxy stops so new processes fall back to direct connection.
pub fn remove_proxy_env_vars() {
    if !cfg!(windows) {
        return;
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let names = [
        "ANTHROPIC_BASE_URL",
        "OPENAI_BASE_URL",
        "HTTPS_PROXY",
        "HTTP_PROXY",
    ];
    for name in &names {
        let status = std::process::Command::new("reg")
            .args(["delete", "HKCU\\Environment", "/v", name, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                tracing::warn!(
                    "reg delete HKCU\\Environment /v {name} failed with exit code {}",
                    s.code().unwrap_or(-1)
                );
            }
            Err(e) => {
                tracing::warn!("reg delete HKCU\\Environment /v {name} could not be launched: {e}");
            }
        }
    }
    tracing::info!("Proxy env vars removed");
}

/// Detect the shell profile path.
fn profile_path() -> Option<std::path::PathBuf> {
    if cfg!(windows) {
        let user = std::env::var("USERPROFILE").ok()?;
        let docs = std::path::PathBuf::from(&user).join("Documents");

        // PowerShell 7 (modern): ~/Documents/PowerShell/Microsoft.PowerShell_profile.ps1
        let ps7 = docs
            .join("PowerShell")
            .join("Microsoft.PowerShell_profile.ps1");
        if ps7.exists() {
            return Some(ps7);
        }
        // Windows PowerShell (legacy): ~/Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1
        let ps5 = docs
            .join("WindowsPowerShell")
            .join("Microsoft.PowerShell_profile.ps1");
        if ps5.exists() {
            return Some(ps5);
        }
        // Neither exists yet — default to PowerShell 7 path (will be created)
        Some(ps7)
    } else {
        let home = std::env::var("HOME").ok()?;
        let zsh = std::path::PathBuf::from(&home).join(".zshrc");
        if zsh.exists() {
            Some(zsh)
        } else {
            Some(std::path::PathBuf::from(&home).join(".bashrc"))
        }
    }
}

/// Check if the CLI profile already has the SessionGraph snippet.
#[tauri::command]
pub fn get_cli_profile_status() -> serde_json::Value {
    let snippet_marker = "# SessionGraph auto-detect";
    let installed = profile_path()
        .map(|p| {
            std::fs::read_to_string(&p)
                .map(|content| content.contains(snippet_marker))
                .unwrap_or(false)
        })
        .unwrap_or(false);

    serde_json::json!({
        "installed": installed,
        "profile_path": profile_path().map(|p| p.to_string_lossy().to_string()),
    })
}

/// Automatically add the CLI health-check snippet to the user's shell profile.
/// Returns the profile path that was modified.
#[tauri::command]
pub fn add_cli_profile(state: tauri::State<'_, Arc<InterceptState>>) -> Result<String, String> {
    let port = state.proxy_port;
    let snippet = cli_snippet(port);
    let profile = profile_path().ok_or("Could not determine shell profile path")?;

    // Read existing content
    let existing = std::fs::read_to_string(&profile).unwrap_or_default();
    if existing.contains("# SessionGraph auto-detect") {
        return Err("CLI auto-detect is already installed".to_string());
    }

    // Ensure parent directory exists (PowerShell profile dir may not exist yet)
    if let Some(parent) = profile.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
    }

    // Append the snippet
    let mut content = existing;
    content.push_str(&snippet);
    std::fs::write(&profile, &content)
        .map_err(|e| format!("Failed to write to {}: {}", profile.display(), e))?;

    // Also set persistent registry env vars for all new processes
    set_proxy_env_vars(port);

    tracing::info!("CLI auto-detect snippet added to {}", profile.display());
    Ok(profile.to_string_lossy().to_string())
}

/// Remove the SessionGraph snippet from the user's shell profile.
#[tauri::command]
pub fn remove_cli_profile() -> Result<String, String> {
    let profile = profile_path().ok_or("Could not determine shell profile path")?;
    let content = std::fs::read_to_string(&profile).map_err(|e| e.to_string())?;

    // Remove lines between the marker comment and the next blank line
    let marker = "# SessionGraph auto-detect";
    let mut lines: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in content.lines() {
        if line.trim() == marker {
            skipping = true;
        } else if skipping && line.trim().is_empty() {
            skipping = false;
            continue;
        } else if !skipping {
            lines.push(line);
        }
    }
    let new_content = lines.join("\n");
    std::fs::write(&profile, &new_content).map_err(|e| e.to_string())?;

    // Also remove persistent registry env vars
    remove_proxy_env_vars();

    tracing::info!("CLI auto-detect snippet removed from {}", profile.display());
    Ok(profile.to_string_lossy().to_string())
}

/// Get the setup script text for display in onboarding (informational only).
#[tauri::command]
pub fn get_setup_script(state: tauri::State<'_, Arc<InterceptState>>) -> String {
    let port = state.proxy_port;
    cli_snippet(port)
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
        shExpMatch(host, "*.openrouter.ai") ||
        shExpMatch(host, "api.deepseek.com") ||
        shExpMatch(host, "cloudcode-pa.googleapis.com") ||
        shExpMatch(host, "generativelanguage.googleapis.com")) {{
        return "PROXY 127.0.0.1:{port}; DIRECT";
    }}
    return "DIRECT";
}}
"#,
    );
    std::fs::write(&path, &content).map_err(|e| format!("Cannot write PAC file: {e}"))?;
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

/// Enable or disable the system proxy (PAC file). Callable from IPC or directly.
pub fn set_system_proxy_sync(enabled: bool) -> Result<(), String> {
    let dir = sessiongraph_dir().ok_or("Cannot determine home directory")?;
    let pac_path = dir.join("proxy.pac");

    if !pac_path.exists() {
        return Err("PAC file not found. Restart SessionGraph to create it.".to_string());
    }

    let pac_url = format!("file:///{}", pac_path.to_string_lossy());

    if cfg!(windows) {
        let cmd = if enabled {
            format!(
                "Set-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name AutoConfigURL -Value '{url}'",
                url = pac_url
            )
        } else {
            "Remove-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name AutoConfigURL -ErrorAction SilentlyContinue".to_string()
        };

        let output = {
            let mut c = std::process::Command::new("powershell");
            c.args(["-NoProfile", "-Command", &cmd]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            c.output()
                .map_err(|e| format!("Failed to run PowerShell: {e}"))?
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let action = if enabled { "enable" } else { "disable" };
            return Err(format!("Failed to {action} system proxy: {stderr}"));
        }

        tracing::info!(
            "System proxy {}",
            if enabled { "enabled" } else { "disabled" }
        );
        Ok(())
    } else {
        tracing::warn!("set_system_proxy not implemented on this platform");
        Err("System proxy configuration is only supported on Windows".to_string())
    }
}

/// Enable or disable the system proxy (PAC file). IPC entry point.
#[tauri::command]
pub async fn set_system_proxy(enabled: bool) -> Result<(), String> {
    set_system_proxy_sync(enabled)
}
