//! Settings, proxy control, and onboarding IPC commands. See spec section 7.
//!
//! Stub — returns spec defaults. Real wiring lands across Weeks 1-4.

use std::collections::HashMap;

use serde::Serialize;

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
pub fn get_settings() -> HashMap<String, String> {
    let mut s = HashMap::new();
    s.insert("proxy_port".into(), "4200".into());
    s.insert("session_timeout_minutes".into(), "30".into());
    s.insert("compression_enabled".into(), "true".into());
    s.insert("graph_injection_enabled".into(), "true".into());
    s.insert("graph_max_tokens".into(), "500".into());
    s.insert("tier".into(), "free".into());
    s.insert("sessions_saved_this_month".into(), "0".into());
    s.insert("onboarding_complete".into(), "false".into());
    s
}

#[tauri::command]
pub fn update_setting(key: String, value: String) {
    let _ = (key, value);
}

#[tauri::command]
pub fn get_proxy_status() -> ProxyStatus {
    ProxyStatus {
        running: false,
        port: 4200,
        uptime_seconds: 0,
    }
}

#[tauri::command]
pub fn restart_proxy() {}

#[tauri::command]
pub fn get_setup_script() -> String {
    // Real OS-specific generation lands in Week 4 Task 2.
    "export ANTHROPIC_BASE_URL=http://localhost:4200\n\
     export OPENAI_BASE_URL=http://localhost:4200/v1\n"
        .to_string()
}

#[tauri::command]
pub fn check_proxy_health() -> HealthStatus {
    HealthStatus {
        status: "unhealthy",
        proxy_version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: 0,
    }
}
