//! SessionGraph desktop library — Tauri command registration and proxy lifecycle.
//!
//! Week 1 / Task 1: scaffold only. The proxy server, session graph, database,
//! and IPC commands are stubbed out and will be wired up in subsequent tasks.

pub mod commands;
pub mod db;
pub mod graph;
pub mod proxy;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SESSIONGRAPH_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::stats::get_dashboard_stats,
            commands::stats::get_current_session,
            commands::sessions::list_sessions,
            commands::sessions::get_session,
            commands::sessions::get_session_graph,
            commands::sessions::delete_session_graph,
            commands::settings::get_settings,
            commands::settings::update_setting,
            commands::settings::get_proxy_status,
            commands::settings::restart_proxy,
            commands::settings::get_setup_script,
            commands::settings::check_proxy_health,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
