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
            "# Run this in PowerShell as Administrator:\n\
             [System.Environment]::SetEnvironmentVariable('ANTHROPIC_BASE_URL','http://localhost:{port}','User')\n\
             [System.Environment]::SetEnvironmentVariable('OPENAI_BASE_URL','http://localhost:{port}/v1','User')\n\
             \n# Restart your terminal for changes to take effect.\n"
        )
    } else {
        format!(
            "# Run this in your terminal:\n\
             echo 'export ANTHROPIC_BASE_URL=http://localhost:{port}' >> ~/.zshrc\n\
             echo 'export OPENAI_BASE_URL=http://localhost:{port}/v1' >> ~/.zshrc\n\
             source ~/.zshrc\n\
             \n# Or substitute ~/.bashrc if you use bash.\n"
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
