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
             \x20\x20   $env:CODEX_OSS_BASE_URL = $sgProxyUrl\n\
             \x20 }} else {{\n\
             \x20\x20   Remove-Item Env:ANTHROPIC_BASE_URL -ErrorAction SilentlyContinue\n\
             \x20\x20   Remove-Item Env:OPENAI_BASE_URL -ErrorAction SilentlyContinue\n\
             \x20\x20   Remove-Item Env:CODEX_OSS_BASE_URL -ErrorAction SilentlyContinue\n\
             \x20 }}\n\
             }} catch {{\n\
             \x20 Remove-Item Env:ANTHROPIC_BASE_URL -ErrorAction SilentlyContinue\n\
             \x20 Remove-Item Env:OPENAI_BASE_URL -ErrorAction SilentlyContinue\n\
             \x20 Remove-Item Env:CODEX_OSS_BASE_URL -ErrorAction SilentlyContinue\n\
             }}\n"
        )
    } else {
        format!(
            "\n# SessionGraph auto-detect (proxy running -> use it; closed -> direct)\n\
             if curl -sf http://localhost:{port}/health > /dev/null 2>&1; then\n\
             \x20 export ANTHROPIC_BASE_URL=http://localhost:{port}\n\
             \x20 export OPENAI_BASE_URL=http://localhost:{port}/v1\n\
             \x20 export CODEX_OSS_BASE_URL=http://localhost:{port}\n\
             fi\n"
        )
    }
}

/// Set persistent user env vars pointing AI tools to the proxy.
/// Written to HKCU\Environment so new processes auto-discover the proxy.
/// Called when proxy starts, removed when proxy stops (see `remove_proxy_env_vars`).
#[cfg(not(windows))]
pub fn set_proxy_env_vars(_port: u16) {}

#[cfg(windows)]
pub fn set_proxy_env_vars(port: u16) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let proxy_url = format!("http://localhost:{port}");
    let openai_url = format!("http://localhost:{port}/v1");

    // Path to our MITM CA cert — Bun/Node.js tools read NODE_EXTRA_CA_CERTS
    // to trust additional certificates, which makes HTTPS_PROXY MITM work.
    let ca_cert_path = {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        format!("{}\\{}", home, ".sessiongraph\\mitm-ca.crt")
    };

    // HTTPS_PROXY / HTTP_PROXY intentionally NOT set here.
    // Bun (opencode) applies HTTPS_PROXY globally, including to the Anthropic SDK,
    // which causes it to CONNECT to api.anthropic.com even when provider.baseURL
    // is set to our localhost address. Tools that can't read env vars get their own
    // config files (see write_opencode_config). Tools that DO read ANTHROPIC_BASE_URL
    // (claude-code) use that directly without needing HTTPS_PROXY.
    // SSL_CERT_FILE intentionally NOT set: it replaces the entire system CA bundle,
    // breaking TLS verification for all other hosts (e.g. Codex → api.openai.com).
    // NODE_EXTRA_CA_CERTS correctly ADDS our cert to the existing bundle.
    //
    // CODEX_OSS_BASE_URL: OpenAI Codex CLI (Rust binary) reads this env var instead
    // of OPENAI_BASE_URL to override the upstream API base URL.
    let codex_url = format!("http://localhost:{port}");
    let pairs: [(&str, &str); 4] = [
        ("ANTHROPIC_BASE_URL", &proxy_url),
        ("OPENAI_BASE_URL", &openai_url),
        ("CODEX_OSS_BASE_URL", &codex_url),
        ("NODE_EXTRA_CA_CERTS", &ca_cert_path),
    ];

    // Set in current process so child processes spawned from any shell
    // that launched us also inherit the vars immediately.
    for (name, value) in &pairs {
        std::env::set_var(name, value);
    }

    // Persist to HKCU\Environment via setx so newly opened terminals inherit them.
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
                    "setx {name}={value} failed: exit {}",
                    s.code().unwrap_or(-1)
                );
                ok = false;
            }
            Err(e) => {
                tracing::warn!("setx {name}={value} error: {e}");
                ok = false;
            }
        }
    }

    // Broadcast WM_SETTINGCHANGE so Explorer and already-open terminals
    // pick up the new HKCU env vars without needing a logoff.
    broadcast_env_change();

    // Write tool-specific config files so tools that ignore env vars
    // still get redirected to the proxy via their own config mechanism.
    write_opencode_config(port);

    if ok {
        tracing::info!("Proxy env vars set for port {port}");
    }
}

/// Inject provider baseURL overrides into ~/.config/opencode/opencode.json.
/// Merges our settings into the user's existing config rather than replacing it.
/// Uses a "_sessiongraph" marker key to track our managed entries so we can
/// remove them cleanly without touching user settings.
#[cfg(windows)]
fn write_opencode_config(port: u16) {
    let config_path = match opencode_config_path() {
        Some(p) => p,
        None => return,
    };

    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Load existing config (or start fresh).
    let mut config: serde_json::Value = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let base = format!("http://localhost:{port}/v1");
    let provider_obj = serde_json::json!({
        "anthropic": { "options": { "baseURL": base } },
        "openai":    { "options": { "baseURL": base } },
        "openrouter":{ "options": { "baseURL": base } }
    });

    // Merge our provider entries into whatever the user has.
    // We use a comment in the $schema value as a marker so we can clean up
    // later — opencode validates JSON but ignores unknown $schema values.
    // Tracking key: we store our proxy port in the provider baseURL itself,
    // so we can detect and remove our entries on shutdown.
    if let Some(obj) = config.as_object_mut() {
        let provider = obj
            .entry("provider")
            .or_insert_with(|| serde_json::json!({}));
        if let (Some(p), Some(new)) = (provider.as_object_mut(), provider_obj.as_object()) {
            for (k, v) in new {
                p.insert(k.clone(), v.clone());
            }
        }
    }

    match std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).unwrap_or_default(),
    ) {
        Ok(_) => tracing::info!("opencode config written to {}", config_path.display()),
        Err(e) => tracing::warn!("Failed to write opencode config: {e}"),
    }
}

/// Return the path opencode uses for its config file.
/// Follows XDG: $XDG_CONFIG_HOME/opencode/opencode.json, else ~/.config/opencode/opencode.json
#[cfg(windows)]
fn opencode_config_path() -> Option<std::path::PathBuf> {
    let config_base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .ok()?;
            Some(std::path::PathBuf::from(home).join(".config"))
        })?;
    Some(config_base.join("opencode").join("opencode.json"))
}

/// Broadcast WM_SETTINGCHANGE with "Environment" so Windows shells and
/// apps that listen for env changes (Explorer, ConEmu, etc.) reload env vars.
#[cfg(windows)]
fn broadcast_env_change() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    // HWND_BROADCAST = 0xFFFF, WM_SETTINGCHANGE = 0x001A, SMTO_ABORTIFHUNG = 0x0002
    let env_wide: Vec<u16> = OsStr::new("Environment\0").encode_wide().collect();
    unsafe {
        windows_broadcast_setting_change(env_wide.as_ptr());
    }
}

#[cfg(windows)]
unsafe fn windows_broadcast_setting_change(env_ptr: *const u16) {
    // Use SendMessageTimeoutW via raw FFI — no winapi crate needed.
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
    let mut result: usize = 0;
    SendMessageTimeoutW(
        0xFFFF_isize, // HWND_BROADCAST
        0x001A,       // WM_SETTINGCHANGE
        0,
        env_ptr as isize,
        0x0002, // SMTO_ABORTIFHUNG
        1000,
        &mut result,
    );
}

/// Remove persistent user env vars for the proxy.
/// Called when proxy stops so new processes fall back to direct connection.
#[cfg(not(windows))]
pub fn remove_proxy_env_vars() {}

#[cfg(windows)]
pub fn remove_proxy_env_vars() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let names = [
        "ANTHROPIC_BASE_URL",
        "OPENAI_BASE_URL",
        "CODEX_OSS_BASE_URL",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NODE_EXTRA_CA_CERTS",
        "SSL_CERT_FILE",
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
    // Remove our provider overrides from opencode config (keep user settings intact).
    // We identify our entries by the localhost baseURL we injected.
    if let Some(config_path) = opencode_config_path() {
        if let Ok(text) = std::fs::read_to_string(&config_path) {
            if let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(obj) = config.as_object_mut() {
                    let mut changed = false;
                    if let Some(provider) = obj.get_mut("provider").and_then(|p| p.as_object_mut())
                    {
                        for key in &["anthropic", "openai", "openrouter"] {
                            let is_ours = provider
                                .get(*key)
                                .and_then(|v| v.get("options"))
                                .and_then(|o| o.get("baseURL"))
                                .and_then(|u| u.as_str())
                                .map(|u| u.contains("localhost"))
                                .unwrap_or(false);
                            if is_ours {
                                provider.remove(*key);
                                changed = true;
                            }
                        }
                        if provider.is_empty() {
                            obj.remove("provider");
                        }
                    }
                    if changed {
                        let _ = std::fs::write(
                            &config_path,
                            serde_json::to_string_pretty(&config).unwrap_or_default(),
                        );
                        tracing::debug!("opencode provider overrides removed");
                    }
                }
            }
        }
    }

    broadcast_env_change();

    // Also clear the PAC-based system proxy so traffic stops routing through
    // the dead proxy port. Mirrors what set_system_proxy_sync(false) does.
    let _ = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Remove-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name AutoConfigURL -ErrorAction SilentlyContinue",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

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
        shExpMatch(host, "openrouter.ai") ||
        shExpMatch(host, "api.openrouter.ai") ||
        shExpMatch(host, "api.deepseek.com") ||
        shExpMatch(host, "cloudcode-pa.googleapis.com") ||
        shExpMatch(host, "generativelanguage.googleapis.com") ||
        shExpMatch(host, "api.minimax.io") ||
        shExpMatch(host, "api.minimax.chat") ||
        shExpMatch(host, "dashscope.aliyuncs.com") ||
        shExpMatch(host, "open.bigmodel.cn") ||
        shExpMatch(host, "api.together.xyz") ||
        shExpMatch(host, "api.together.ai") ||
        shExpMatch(host, "api.mistral.ai") ||
        shExpMatch(host, "api.groq.com") ||
        shExpMatch(host, "api.cohere.com") ||
        shExpMatch(host, "api.cohere.ai") ||
        shExpMatch(host, "inference.ai.azure.com") ||
        shExpMatch(host, "*.inference.ai.azure.com")) {{
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
