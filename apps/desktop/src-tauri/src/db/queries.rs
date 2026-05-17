//! Database query functions for `sessions`, `requests`, `session_graphs`,
//! `token_usage_daily`, and `settings`. See spec section 4.

use rusqlite::{params, Connection};

use crate::commands::sessions::SessionSummary;
use crate::commands::stats::{CurrentSession, DashboardStats, TodayStats, TotalStats};
use crate::proxy::session::ActiveSession;

// ── Sessions ──────────────────────────────────────────────────────────────

/// Create a new session row and return the assigned UUID.
pub fn insert_session(conn: &Connection, s: &ActiveSession) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO sessions (id, project_hash, project_name, provider, tool, started_at,
         status, message_count, tokens_in_raw, tokens_in_sent, tokens_out, cost_usd_raw, cost_usd_actual)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            s.id,
            s.project_hash,
            s.project_name,
            s.provider,
            s.tool,
            s.started_at.to_rfc3339(),
            s.message_count,
            s.tokens_in_raw,
            s.tokens_in_sent,
            s.tokens_out,
            s.cost_usd_raw,
            s.cost_usd_actual,
        ],
    )?;
    Ok(())
}

/// Mark a session as ended with its final stats.
pub fn end_session(conn: &Connection, id: &str, ended_at: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE sessions SET status = 'ended', ended_at = ?1 WHERE id = ?2",
        params![ended_at, id],
    )?;
    Ok(())
}

/// Increment a session's counters after each request.
#[allow(clippy::too_many_arguments)]
pub fn increment_session(
    conn: &Connection,
    id: &str,
    msg_count_delta: u32,
    tokens_in_raw_delta: u64,
    tokens_in_sent_delta: u64,
    tokens_out_delta: u64,
    cost_raw_delta: f64,
    cost_actual_delta: f64,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE sessions SET
            message_count = message_count + ?2,
            tokens_in_raw = tokens_in_raw + ?3,
            tokens_in_sent = tokens_in_sent + ?4,
            tokens_out   = tokens_out   + ?5,
            cost_usd_raw   = cost_usd_raw   + ?6,
            cost_usd_actual = cost_usd_actual + ?7
         WHERE id = ?1",
        params![
            id,
            msg_count_delta,
            tokens_in_raw_delta,
            tokens_in_sent_delta,
            tokens_out_delta,
            cost_raw_delta,
            cost_actual_delta,
        ],
    )?;
    Ok(())
}

// ── Requests ──────────────────────────────────────────────────────────────

/// Log a single proxied request to the requests table.
#[allow(clippy::too_many_arguments)]
pub fn insert_request(
    conn: &Connection,
    id: &str,
    session_id: &str,
    sequence: u32,
    provider: &str,
    model: &str,
    tokens_in_raw: u64,
    tokens_in_sent: u64,
    tokens_out: u64,
    compression_ratio: Option<f64>,
    graph_injected: bool,
    graph_tokens: u32,
    latency_ms: u64,
    cost_usd_raw: f64,
    cost_usd_actual: f64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO requests
         (id, session_id, sequence, provider, model, tokens_in_raw, tokens_in_sent,
          tokens_out, compression_ratio, graph_injected, graph_tokens, latency_ms,
          cost_usd_raw, cost_usd_actual)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            id,
            session_id,
            sequence,
            provider,
            model,
            tokens_in_raw,
            tokens_in_sent,
            tokens_out,
            compression_ratio,
            graph_injected as i32,
            graph_tokens,
            latency_ms,
            cost_usd_raw,
            cost_usd_actual,
        ],
    )?;
    Ok(())
}

// ── Daily usage ───────────────────────────────────────────────────────────

/// Upsert today's token usage row for a given provider.
#[allow(clippy::too_many_arguments)]
pub fn upsert_daily_usage(
    conn: &Connection,
    date: &str,
    provider: &str,
    tokens_in_raw: u64,
    tokens_in_sent: u64,
    tokens_out: u64,
    cost_raw: f64,
    cost_actual: f64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO token_usage_daily (date, provider, tokens_in_raw, tokens_in_sent,
         tokens_out, cost_usd_raw, cost_usd_actual, savings_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(date, provider) DO UPDATE SET
           tokens_in_raw  = tokens_in_raw  + ?3,
           tokens_in_sent = tokens_in_sent + ?4,
           tokens_out     = tokens_out     + ?5,
           cost_usd_raw   = cost_usd_raw   + ?6,
           cost_usd_actual = cost_usd_actual + ?7,
           savings_usd    = savings_usd    + ?8",
        params![
            date,
            provider,
            tokens_in_raw,
            tokens_in_sent,
            tokens_out,
            cost_raw,
            cost_actual,
            cost_raw - cost_actual, // savings
        ],
    )?;
    Ok(())
}

// ── Settings ──────────────────────────────────────────────────────────────

/// Read a single setting value.
pub fn get_setting(conn: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(Ok(v)) => Ok(Some(v)),
        _ => Ok(None),
    }
}

/// Write a setting value (upsert).
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = datetime('now')",
        params![key, value],
    )?;
    Ok(())
}

// ── Dashboard stats ───────────────────────────────────────────────────────

/// Return today's aggregated stats for the dashboard.
pub fn get_today_stats(conn: &Connection, today: &str) -> anyhow::Result<TodayStats> {
    let row = conn.query_row(
        "SELECT COALESCE(SUM(tokens_in_raw) - SUM(tokens_in_sent), 0),
                COALESCE(SUM(cost_usd_raw) - SUM(cost_usd_actual), 0.0),
                COUNT(*),
                COUNT(DISTINCT session_id)
         FROM requests WHERE date(created_at) = ?1",
        params![today],
        |r| {
            Ok(TodayStats {
                tokens_saved: r.get::<_, i64>(0)?.unsigned_abs(),
                cost_saved_usd: r.get(1)?,
                requests: r.get::<_, i64>(2)?.unsigned_abs(),
                sessions: r.get::<_, i64>(3)?.unsigned_abs(),
            })
        },
    )?;
    Ok(row)
}

/// Return all-time aggregated stats.
pub fn get_total_stats(conn: &Connection) -> anyhow::Result<TotalStats> {
    let row = conn.query_row(
        "SELECT COALESCE(SUM(tokens_in_raw) - SUM(tokens_in_sent), 0),
                COALESCE(SUM(cost_usd_raw) - SUM(cost_usd_actual), 0.0),
                COUNT(DISTINCT session_id)
         FROM requests",
        [],
        |r| {
            Ok(TotalStats {
                tokens_saved: r.get::<_, i64>(0)?.unsigned_abs(),
                cost_saved_usd: r.get(1)?,
                sessions: r.get::<_, i64>(2)?.unsigned_abs(),
                graphs_saved: 0, // filled below
            })
        },
    )?;
    let graphs_saved: u64 = conn
        .query_row("SELECT COUNT(*) FROM session_graphs", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0)
        .unsigned_abs();
    Ok(TotalStats {
        graphs_saved,
        ..row
    })
}

/// Build a full DashboardStats struct for the given date.
pub fn get_dashboard_stats_for_date(
    conn: &Connection,
    today: &str,
    current_session: Option<CurrentSession>,
) -> anyhow::Result<DashboardStats> {
    let today_stats = get_today_stats(conn, today)?;
    let total_stats = get_total_stats(conn)?;
    Ok(DashboardStats {
        today: today_stats,
        total: total_stats,
        current_session,
        active_sessions: vec![],
    })
}

// ── Session listing ───────────────────────────────────────────────────────

/// Store (or replace) a session graph. Uses ON CONFLICT REPLACE per the
/// UNIQUE(project_hash) constraint — only the latest graph per project is kept.
#[allow(clippy::too_many_arguments)]
pub fn upsert_session_graph(
    conn: &Connection,
    id: &str,
    session_id: &str,
    project_hash: &str,
    graph_json: &str,
    token_count: u32,
    extraction_model: &str,
    extraction_cost_usd: f64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO session_graphs
         (id, session_id, project_hash, graph_json, token_count, extraction_model, extraction_cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(project_hash) DO UPDATE SET
           id = ?1,
           session_id = ?2,
           graph_json = ?4,
           token_count = ?5,
           extraction_model = ?6,
           extraction_cost_usd = ?7,
           created_at = datetime('now')",
        rusqlite::params![
            id,
            session_id,
            project_hash,
            graph_json,
            token_count,
            extraction_model,
            extraction_cost_usd,
        ],
    )?;
    Ok(())
}

/// Get the latest session graph JSON string for a project.
pub fn get_latest_graph_json(
    conn: &Connection,
    project_hash: &str,
) -> anyhow::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT graph_json FROM session_graphs WHERE project_hash = ?1")?;
    let mut rows = stmt.query_map(rusqlite::params![project_hash], |row| {
        row.get::<_, String>(0)
    })?;
    match rows.next() {
        Some(Ok(v)) => Ok(Some(v)),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

/// Get a single session by ID.
pub fn get_session_by_id(conn: &Connection, id: &str) -> anyhow::Result<Option<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.project_hash, s.project_name, s.provider, s.tool,
                s.started_at, s.ended_at, s.tokens_in_raw, s.tokens_in_sent,
                s.cost_usd_raw, s.cost_usd_actual,
                CASE WHEN g.project_hash IS NOT NULL THEN 1 ELSE 0 END as has_graph
         FROM sessions s
         LEFT JOIN session_graphs g ON s.project_hash = g.project_hash
         WHERE s.id = ?1",
    )?;

    let mut rows = stmt.query_map(params![id], |row| {
        Ok(SessionSummary {
            id: row.get(0)?,
            project_hash: row.get(1)?,
            project_name: row.get(2)?,
            provider: row.get(3)?,
            tool: row.get(4)?,
            started_at: row.get(5)?,
            ended_at: row.get(6)?,
            tokens_in_raw: row.get::<_, i64>(7)?.unsigned_abs(),
            tokens_in_sent: row.get::<_, i64>(8)?.unsigned_abs(),
            cost_usd_raw: row.get(9)?,
            cost_usd_actual: row.get(10)?,
            has_graph: row.get::<_, i32>(11)? != 0,
        })
    })?;

    match rows.next() {
        Some(Ok(v)) => Ok(Some(v)),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

/// Get token usage for the last N days (for the dashboard chart).
pub fn get_token_usage_last_n_days(
    conn: &Connection,
    days: u32,
) -> anyhow::Result<Vec<(String, u64, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT date,
                COALESCE(SUM(tokens_in_raw), 0) as raw_tokens,
                COALESCE(SUM(tokens_in_sent), 0) as sent_tokens
         FROM token_usage_daily
         WHERE date >= date('now', ?1)
         GROUP BY date
         ORDER BY date ASC",
    )?;

    let offset = format!("-{} days", days);
    let rows = stmt.query_map(params![offset], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?.unsigned_abs(),
            row.get::<_, i64>(2)?.unsigned_abs(),
        ))
    })?;

    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Delete all data from all tables.
pub fn delete_all_data(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "DELETE FROM requests;
         DELETE FROM session_graphs;
         DELETE FROM token_usage_daily;
         DELETE FROM sessions;
         DELETE FROM settings WHERE key NOT IN ('proxy_port', 'session_timeout_minutes',
           'compression_enabled', 'graph_injection_enabled', 'graph_max_tokens', 'tier',
           'sessions_saved_this_month', 'onboarding_complete');",
    )?;
    Ok(())
}

/// Paginated list of past sessions.
/// Lightweight graph index entry — no full JSON, just metadata for the browser list.
pub struct GraphEntry {
    pub project_hash: String,
    pub project_name: Option<String>,
    pub token_count: i64,
    pub last_updated: String,
    pub created_at: String,
    /// JSON array string of the stack (e.g. `["Rust","React"]`), parsed from graph_json.
    pub stack_json: String,
    /// current_task from graph_json state, for preview.
    pub current_task: Option<String>,
}

/// Return all saved session graphs ordered by last_updated DESC.
pub fn list_graphs(conn: &Connection) -> anyhow::Result<Vec<GraphEntry>> {
    let mut stmt = conn.prepare(
        "SELECT g.project_hash,
                s.project_name,
                g.token_count,
                g.created_at,
                g.created_at,
                json_extract(g.graph_json, '$.project.stack'),
                json_extract(g.graph_json, '$.state.current_task')
         FROM session_graphs g
         LEFT JOIN sessions s ON g.session_id = s.id
         ORDER BY g.created_at DESC",
    )?;

    let items = stmt
        .query_map([], |row| {
            Ok(GraphEntry {
                project_hash: row.get(0)?,
                project_name: row.get(1)?,
                token_count: row.get(2)?,
                created_at: row.get(3)?,
                last_updated: row.get(4)?,
                stack_json: row
                    .get::<_, Option<String>>(5)?
                    .unwrap_or_else(|| "[]".into()),
                current_task: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(items)
}

// ── Analytics aggregates for usage sync ─────────────────────────────────

/// Count sessions grouped by tool for the tool distribution payload.
/// Returns a JSON object string like `{"claude-code":12,"cursor":3}`.
pub fn get_tool_usage_json(conn: &Connection) -> String {
    let mut stmt = match conn
        .prepare("SELECT COALESCE(tool, 'unknown'), COUNT(*) FROM sessions GROUP BY tool")
    {
        Ok(s) => s,
        Err(_) => return "{}".into(),
    };
    let rows: Vec<(String, i64)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => return "{}".into(),
    };
    let obj: serde_json::Map<String, serde_json::Value> = rows
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::from(v)))
        .collect();
    serde_json::to_string(&obj).unwrap_or_else(|_| "{}".into())
}

/// Count requests grouped by model for the model distribution payload.
/// Returns a JSON object string like `{"claude-sonnet-4-5":100,"gpt-4o":20}`.
pub fn get_model_usage_json(conn: &Connection) -> String {
    let mut stmt = match conn.prepare("SELECT model, COUNT(*) FROM requests GROUP BY model") {
        Ok(s) => s,
        Err(_) => return "{}".into(),
    };
    let rows: Vec<(String, i64)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => return "{}".into(),
    };
    let obj: serde_json::Map<String, serde_json::Value> = rows
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::from(v)))
        .collect();
    serde_json::to_string(&obj).unwrap_or_else(|_| "{}".into())
}

/// Average requests per session and average session duration in minutes.
/// Returns `(avg_requests, avg_duration_minutes)`.
pub fn get_session_length_stats(conn: &Connection) -> (f64, f64) {
    let avg_req = conn
        .query_row(
            "SELECT AVG(message_count) FROM sessions WHERE status != 'active'",
            [],
            |r| r.get::<_, Option<f64>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or(0.0);

    let avg_dur = conn
        .query_row(
            "SELECT AVG(
                (julianday(COALESCE(ended_at, datetime('now'))) - julianday(started_at)) * 1440.0
             ) FROM sessions WHERE status != 'active'",
            [],
            |r| r.get::<_, Option<f64>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or(0.0);

    (avg_req, avg_dur)
}

/// Average compression ratio per model (only requests where compression ran).
/// Returns a JSON object string like `{"claude-sonnet-4-5":0.72,"gpt-4o":0.68}`.
pub fn get_compression_by_model_json(conn: &Connection) -> String {
    let mut stmt = match conn.prepare(
        "SELECT model, AVG(compression_ratio)
         FROM requests
         WHERE compression_ratio IS NOT NULL AND compression_ratio < 1.0
         GROUP BY model",
    ) {
        Ok(s) => s,
        Err(_) => return "{}".into(),
    };
    let rows: Vec<(String, f64)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => return "{}".into(),
    };
    let obj: serde_json::Map<String, serde_json::Value> = rows
        .into_iter()
        .map(|(k, v)| {
            let rounded = (v * 1000.0).round() / 1000.0;
            (k, serde_json::Value::from(rounded))
        })
        .collect();
    serde_json::to_string(&obj).unwrap_or_else(|_| "{}".into())
}

pub fn list_sessions_paginated(
    conn: &Connection,
    page: u32,
    per_page: u32,
) -> anyhow::Result<(Vec<SessionSummary>, u64)> {
    let total: u64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))
        .map(|v| v.unsigned_abs())?;

    let offset = (page.saturating_sub(1)) * per_page;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.project_hash, s.project_name, s.provider, s.tool,
                s.started_at, s.ended_at, s.tokens_in_raw, s.tokens_in_sent,
                s.cost_usd_raw, s.cost_usd_actual,
                CASE WHEN g.project_hash IS NOT NULL THEN 1 ELSE 0 END as has_graph
         FROM sessions s
         LEFT JOIN session_graphs g ON s.project_hash = g.project_hash
         ORDER BY s.started_at DESC
         LIMIT ?1 OFFSET ?2",
    )?;

    let items = stmt
        .query_map(params![per_page, offset], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                project_hash: row.get(1)?,
                project_name: row.get(2)?,
                provider: row.get(3)?,
                tool: row.get(4)?,
                started_at: row.get(5)?,
                ended_at: row.get(6)?,
                tokens_in_raw: row.get::<_, i64>(7)?.unsigned_abs(),
                tokens_in_sent: row.get::<_, i64>(8)?.unsigned_abs(),
                cost_usd_raw: row.get(9)?,
                cost_usd_actual: row.get(10)?,
                has_graph: row.get::<_, i32>(11)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok((items, total))
}
