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
    // Install rustls crypto provider process-wide before any TLS work.
    // Required by tokio-rustls; panics without it when multiple crates
    // pull in rustls with no default provider selected.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Initialize logging — write to both stderr and a file so we can tail it.
    let log_path = {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(home).join(".sessiongraph").join("proxy.log")
    };
    let _ = std::fs::create_dir_all(log_path.parent().unwrap());
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();

    let filter = tracing_subscriber::EnvFilter::try_from_env("SESSIONGRAPH_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("sessiongraph_desktop_lib=debug"));

    use tracing_subscriber::prelude::*;
    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    if let Some(file) = log_file {
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file));
        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .init();
    }
    eprintln!("Proxy log: {}", log_path.display());

    // Initialize the database
    let conn = db::init_db().expect("Failed to initialize SessionGraph database");

    // Read proxy port from settings (before connection is consumed by InterceptState)
    let proxy_port: u16 = crate::db::queries::get_setting(&conn, "proxy_port")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4200);

    // Write the PAC file for automatic proxy discovery
    if let Err(e) = crate::commands::settings::write_pac_file(proxy_port) {
        tracing::warn!("Failed to write PAC file: {}", e);
    }

    // Clean up stale env vars from previous crash, then set fresh ones
    crate::commands::settings::remove_proxy_env_vars();
    crate::commands::settings::set_proxy_env_vars(proxy_port);

    // Auto-enable system proxy (PAC) so GUI apps auto-discover the proxy.
    // The PAC file has ;DIRECT fallback so tools work fine when app is closed.
    if let Err(e) = crate::commands::settings::set_system_proxy_sync(true) {
        tracing::warn!("Failed to enable system proxy: {}", e);
    }

    // Initialize MITM TLS interception (best-effort; failure means no MITM)
    let mitm_state = proxy::mitm::init_mitm().ok();
    if mitm_state.is_some() {
        tracing::info!("MITM TLS interception enabled");
    } else {
        tracing::warn!("MITM TLS interception not available — using plain tunnel for HTTPS");
    }

    // Build the shared application state
    let mut state = proxy::InterceptState::new(conn, proxy_port);
    state.mitm = mitm_state;
    let state = Arc::new(state);

    // Set up the proxy shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Start the proxy server in the background
    let proxy_state = state.clone();
    let proxy_port_server = proxy_port;
    tauri::async_runtime::spawn(async move {
        proxy::server::start(proxy_state, proxy_port_server, shutdown_rx).await;
    });

    tracing::info!("SessionGraph v{} starting", env!("CARGO_PKG_VERSION"));

    let shutdown_state = state.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state.clone())
        // Store the shutdown sender so we can cleanly stop the proxy on exit
        .setup(move |app| {
            app.manage(ProxyShutdown {
                tx: Some(shutdown_tx),
                state: shutdown_state,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::stats::get_dashboard_stats,
            commands::stats::get_current_session,
            commands::stats::get_token_usage_chart,
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
            commands::settings::delete_all_data,
            commands::settings::get_app_version,
            commands::settings::get_system_proxy_status,
            commands::settings::set_system_proxy,
            commands::settings::get_cli_profile_status,
            commands::settings::add_cli_profile,
            commands::settings::remove_cli_profile,
        ])
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                tracing::info!("Window destroyed, proxy will shut down with app");
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Holds the oneshot sender for clean proxy shutdown, and a reference
/// to the intercept state so we can end active sessions on drop.
struct ProxyShutdown {
    tx: Option<tokio::sync::oneshot::Sender<()>>,
    state: Arc<proxy::InterceptState>,
}

impl Drop for ProxyShutdown {
    fn drop(&mut self) {
        // End sessions first, while the runtime is still alive
        let state = self.state.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                tokio::task::block_in_place(|| {
                    handle.block_on(async {
                        state.end_all_sessions().await;
                    })
                });
            }
            Err(_) => {
                tracing::warn!("No tokio runtime — skipping session end-on-drop");
            }
        }

        // Send proxy shutdown signal after sessions are ended
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
            tracing::info!("Proxy shutdown signal sent");
        }

        // Remove persistent env vars so new processes fall back to direct
        crate::commands::settings::remove_proxy_env_vars();
        // Disable system proxy (PAC) so no impact when app is closed
        let _ = crate::commands::settings::set_system_proxy_sync(false);
    }
}
