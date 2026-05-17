//! SessionGraph desktop library — Tauri command registration and proxy lifecycle.
//!
//! On startup:
//! 1. Initialize the SQLite database at `~/.sessiongraph/sessiongraph.db`
//! 2. Start the Axum proxy server on `127.0.0.1:4200`
//! 3. Register all Tauri IPC commands

pub mod commands;
pub mod db;
pub mod graph;
pub mod proxy;
pub mod venv;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SESSIONGRAPH_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Initialize the database
    let conn = db::init_db().expect("Failed to initialize SessionGraph database");

    // Build the shared application state
    let state = Arc::new(proxy::InterceptState::new(conn));

    // Set up the proxy shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Start the proxy server in the background
    let proxy_state = state.clone();
    tauri::async_runtime::spawn(async move {
        proxy::server::start(proxy_state, 4200, shutdown_rx).await;
    });

    tracing::info!("SessionGraph v{} starting", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state.clone())
        // Store the shutdown sender so we can cleanly stop the proxy on exit
        .setup(move |app| {
            app.manage(ProxyShutdown {
                tx: Some(shutdown_tx),
            });
            Ok(())
        })
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
            commands::settings::check_venv_status,
            commands::settings::setup_venv,
        ])
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                tracing::info!("Window destroyed, proxy will shut down with app");
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Holds the oneshot sender for clean proxy shutdown.
#[allow(dead_code)]
struct ProxyShutdown {
    tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for ProxyShutdown {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
            tracing::info!("Proxy shutdown signal sent");
        }
    }
}
