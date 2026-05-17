//! SQLite database layer. See spec section 4.
//!
//! Initializes the database at `~/.sessiongraph/sessiongraph.db`, runs
//! migrations, and exposes the connection for the rest of the app.

pub mod queries;

use anyhow::Context;
use rusqlite::Connection;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Global file for error logging per spec §5.5
static ERROR_LOG: OnceLock<std::sync::Mutex<std::fs::File>> = OnceLock::new();

/// Resolve the SessionGraph data directory (`~/.sessiongraph`).
fn data_dir() -> anyhow::Result<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").context("USERPROFILE not set")?
    } else {
        std::env::var("HOME").context("HOME not set")?
    };
    Ok(PathBuf::from(home).join(".sessiongraph"))
}

/// Get the error log file path.
fn error_log_path() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("logs").join("error.log"))
}

/// Initialize error logging and return a guard for the error log file.
pub fn init_error_log() -> anyhow::Result<()> {
    let log_path = error_log_path()?;

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open error log at {}", log_path.display()))?;

    let _ = ERROR_LOG.set(std::sync::Mutex::new(file));

    tracing::info!("Error log initialized at {}", log_path.display());
    Ok(())
}

/// Log an error to the error log file per spec §5.5.
/// This is called for internal errors that should not propagate to the client.
pub fn log_error(message: &str) {
    if let Some(guard) = ERROR_LOG.get() {
        if let Ok(mut file) = guard.lock() {
            let timestamp = chrono::Utc::now().to_rfc3339();
            let _ = writeln!(file, "[{}] {}", timestamp, message);
        }
    }
}

/// Initialize the SQLite database — create directories, open connection,
/// and run migrations. Returns the ready-to-use connection.
pub fn init_db() -> anyhow::Result<Connection> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).context("failed to create ~/.sessiongraph")?;
    std::fs::create_dir_all(dir.join("logs")).context("failed to create logs directory")?;

    // Initialize error log
    let _ = init_error_log();

    let db_path = dir.join("sessiongraph.db");
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    // Enable WAL mode for better concurrent read performance
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    // Run migration
    let migration_sql = include_str!("migrations/001_init.sql");
    conn.execute_batch(migration_sql).context("failed to run migration 001")?;

    tracing::info!("Database initialized at {}", db_path.display());
    Ok(conn)
}
