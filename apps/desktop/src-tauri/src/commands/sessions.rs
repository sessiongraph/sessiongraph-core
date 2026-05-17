//! Sessions IPC commands. See spec section 7.
//!
//! Stub — returns empty lists. Real wiring lands in Week 2.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub project_hash: String,
    pub project_name: Option<String>,
    pub provider: String,
    pub tool: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub tokens_in_raw: u64,
    pub tokens_in_sent: u64,
    pub cost_usd_raw: f64,
    pub cost_usd_actual: f64,
    pub has_graph: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionPage {
    pub items: Vec<SessionSummary>,
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
}

#[tauri::command]
pub fn list_sessions(page: u32, per_page: u32) -> SessionPage {
    SessionPage {
        items: Vec::new(),
        page,
        per_page,
        total: 0,
    }
}

#[tauri::command]
pub fn get_session(id: String) -> Option<SessionSummary> {
    let _ = id;
    None
}

#[tauri::command]
pub fn get_session_graph(project_hash: String) -> Option<Value> {
    let _ = project_hash;
    None
}

#[tauri::command]
pub fn delete_session_graph(project_hash: String) {
    let _ = project_hash;
}
