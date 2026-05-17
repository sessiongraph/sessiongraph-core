//! Sessions IPC commands. See spec section 7.

use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

use crate::db::queries;
use crate::proxy::InterceptState;

#[derive(Debug, Serialize, Clone)]
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
pub fn list_sessions(
    state: tauri::State<'_, Arc<InterceptState>>,
    page: u32,
    per_page: u32,
) -> SessionPage {
    let db = match state.db.lock() {
        Ok(d) => d,
        Err(_) => {
            return SessionPage {
                items: Vec::new(),
                page,
                per_page,
                total: 0,
            }
        }
    };

    match queries::list_sessions_paginated(&db, page, per_page) {
        Ok((items, total)) => SessionPage {
            items,
            page,
            per_page,
            total,
        },
        Err(e) => {
            tracing::error!("list_sessions failed: {}", e);
            SessionPage {
                items: Vec::new(),
                page,
                per_page,
                total: 0,
            }
        }
    }
}

#[tauri::command]
pub fn get_session(
    state: tauri::State<'_, Arc<InterceptState>>,
    id: String,
) -> Option<SessionSummary> {
    let db = state.db.lock().ok()?;
    queries::get_session_by_id(&db, &id).ok().flatten()
}

#[tauri::command]
pub fn get_session_graph(
    state: tauri::State<'_, Arc<InterceptState>>,
    project_hash: String,
) -> Option<Value> {
    let db = state.db.lock().ok()?;
    let mut stmt = db
        .prepare("SELECT graph_json FROM session_graphs WHERE project_hash = ?1")
        .ok()?;
    let json_str: String = stmt
        .query_row(rusqlite::params![project_hash], |row| row.get(0))
        .ok()?;
    serde_json::from_str(&json_str).ok()
}

#[tauri::command]
pub fn delete_session_graph(state: tauri::State<'_, Arc<InterceptState>>, project_hash: String) {
    if let Ok(db) = state.db.lock() {
        let _ = db.execute(
            "DELETE FROM session_graphs WHERE project_hash = ?1",
            rusqlite::params![project_hash],
        );
    }
}
