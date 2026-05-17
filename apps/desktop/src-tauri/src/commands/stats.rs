//! Stats IPC commands. See spec section 7.

use chrono::Utc;
use serde::Serialize;
use std::sync::Arc;

use crate::db::queries;
use crate::proxy::InterceptState;

#[derive(Debug, Serialize, Default, Clone)]
pub struct TodayStats {
    pub tokens_saved: u64,
    pub cost_saved_usd: f64,
    pub requests: u64,
    pub sessions: u64,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct TotalStats {
    pub tokens_saved: u64,
    pub cost_saved_usd: f64,
    pub sessions: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct CurrentSession {
    pub id: String,
    pub active: bool,
    pub tokens_in_raw: u64,
    pub tokens_in_sent: u64,
    pub compression_ratio: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DashboardStats {
    pub today: TodayStats,
    pub total: TotalStats,
    pub current_session: Option<CurrentSession>,
}

#[tauri::command]
pub async fn get_dashboard_stats(
    state: tauri::State<'_, Arc<InterceptState>>,
) -> Result<DashboardStats, String> {
    let today = Utc::now().format("%Y-%m-%d").to_string();

    let (today_stats, total_stats) = {
        match state.db.lock() {
            Ok(db) => (
                queries::get_today_stats(&db, &today).ok(),
                queries::get_total_stats(&db).ok(),
            ),
            Err(_) => (None, None),
        }
    };

    let current_session = {
        let sessions = state.active_sessions.lock().await;
        sessions.first().map(|s| CurrentSession {
            id: s.id.clone(),
            active: true,
            tokens_in_raw: s.tokens_in_raw,
            tokens_in_sent: s.tokens_in_sent,
            compression_ratio: if s.tokens_in_raw > 0 {
                s.tokens_in_sent as f64 / s.tokens_in_raw as f64
            } else {
                0.0
            },
        })
    };

    Ok(DashboardStats {
        today: today_stats.unwrap_or_default(),
        total: total_stats.unwrap_or_default(),
        current_session,
    })
}

#[tauri::command]
pub async fn get_current_session(
    state: tauri::State<'_, Arc<InterceptState>>,
) -> Result<Option<CurrentSession>, String> {
    let sessions = state.active_sessions.lock().await;
    Ok(sessions.first().map(|s| CurrentSession {
        id: s.id.clone(),
        active: true,
        tokens_in_raw: s.tokens_in_raw,
        tokens_in_sent: s.tokens_in_sent,
        compression_ratio: if s.tokens_in_raw > 0 {
            s.tokens_in_sent as f64 / s.tokens_in_raw as f64
        } else {
            0.0
        },
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct DailyTokenUsage {
    pub date: String,
    pub tokens_raw: u64,
    pub tokens_sent: u64,
}

#[tauri::command]
pub fn get_token_usage_chart(
    state: tauri::State<'_, Arc<InterceptState>>,
    days: u32,
) -> Result<Vec<DailyTokenUsage>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let rows = crate::db::queries::get_token_usage_last_n_days(&db, days)
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(date, tokens_raw, tokens_sent)| DailyTokenUsage {
            date,
            tokens_raw,
            tokens_sent,
        })
        .collect())
}