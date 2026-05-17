//! Stats IPC commands. See spec section 7.
//!
//! Stub — returns zeroed/empty data. Real wiring lands in Week 1 Task 10.

use serde::Serialize;

#[derive(Debug, Serialize, Default)]
pub struct TodayStats {
    pub tokens_saved: u64,
    pub cost_saved_usd: f64,
    pub requests: u64,
    pub sessions: u64,
}

#[derive(Debug, Serialize, Default)]
pub struct TotalStats {
    pub tokens_saved: u64,
    pub cost_saved_usd: f64,
    pub sessions: u64,
}

#[derive(Debug, Serialize)]
pub struct CurrentSession {
    pub id: String,
    pub active: bool,
    pub tokens_in_raw: u64,
    pub tokens_in_sent: u64,
    pub compression_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct DashboardStats {
    pub today: TodayStats,
    pub total: TotalStats,
    pub current_session: Option<CurrentSession>,
}

#[tauri::command]
pub fn get_dashboard_stats() -> DashboardStats {
    DashboardStats {
        today: TodayStats::default(),
        total: TotalStats::default(),
        current_session: None,
    }
}

#[tauri::command]
pub fn get_current_session() -> Option<CurrentSession> {
    None
}
