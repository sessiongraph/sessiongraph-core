//! Database unit tests — schema, CRUD, settings, stats.

#[cfg(test)]
mod db_tests {
    use rusqlite::Connection;

    use crate::db::queries;
    use crate::proxy::session::ActiveSession;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        let migration_sql = include_str!("migrations/001_init.sql");
        conn.execute_batch(migration_sql).unwrap();
        conn
    }

    fn test_session(project_hash: &str, provider: &str) -> ActiveSession {
        ActiveSession::new(
            project_hash.into(),
            "abc123def456".into(),
            Some("test-project".into()),
            provider.into(),
            Some("claude-code".into()),
        )
    }

    // ── Settings ──────────────────────────────────────────────────────

    #[test]
    fn settings_defaults_on_fresh_db() {
        let db = test_db();
        assert_eq!(
            queries::get_setting(&db, "proxy_port").unwrap(),
            Some("4200".into())
        );
        assert_eq!(
            queries::get_setting(&db, "session_timeout_minutes").unwrap(),
            Some("30".into())
        );
        assert_eq!(
            queries::get_setting(&db, "compression_enabled").unwrap(),
            Some("true".into())
        );
        assert_eq!(
            queries::get_setting(&db, "graph_max_tokens").unwrap(),
            Some("500".into())
        );
        assert_eq!(
            queries::get_setting(&db, "tier").unwrap(),
            Some("free".into())
        );
        assert_eq!(
            queries::get_setting(&db, "onboarding_complete").unwrap(),
            Some("false".into())
        );
    }

    #[test]
    fn settings_read_write() {
        let db = test_db();
        queries::set_setting(&db, "proxy_port", "8080").unwrap();
        assert_eq!(
            queries::get_setting(&db, "proxy_port").unwrap(),
            Some("8080".into())
        );
    }

    #[test]
    fn settings_missing_key_returns_none() {
        let db = test_db();
        assert_eq!(queries::get_setting(&db, "nonexistent").unwrap(), None);
    }

    // ── Sessions ─────────────────────────────────────────────────────

    #[test]
    fn insert_and_end_session() {
        let db = test_db();
        let s = test_session("deadbeef00000001", "anthropic");
        queries::insert_session(&db, &s).unwrap();

        let ended = chrono::Utc::now().to_rfc3339();
        queries::end_session(&db, &s.id, &ended).unwrap();

        // Verify via list
        let (items, total) = queries::list_sessions_paginated(&db, 1, 10).unwrap();
        assert_eq!(total, 1);
        assert_eq!(items[0].id, s.id);
        assert_eq!(items[0].project_hash, "deadbeef00000001");
    }

    #[test]
    fn increment_session_counters() {
        let db = test_db();
        let s = test_session("deadbeef00000002", "anthropic");
        queries::insert_session(&db, &s).unwrap();

        queries::increment_session(&db, &s.id, 2, 500, 300, 700, 0.001, 0.0005).unwrap();

        let detail = queries::get_session_by_id(&db, &s.id).unwrap().unwrap();
        assert_eq!(detail.tokens_in_raw, 500);
        assert_eq!(detail.tokens_in_sent, 300);
        assert!(detail.cost_usd_raw > 0.0);
    }

    #[test]
    fn get_session_by_id_returns_none_for_missing() {
        let db = test_db();
        assert!(queries::get_session_by_id(&db, "nonexistent")
            .unwrap()
            .is_none());
    }

    #[test]
    fn list_sessions_paginated() {
        let db = test_db();
        for i in 0..5 {
            let s = test_session(&format!("hash{:08x}", i), "anthropic");
            queries::insert_session(&db, &s).unwrap();
        }

        let (items, total) = queries::list_sessions_paginated(&db, 1, 3).unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);

        let (items2, _) = queries::list_sessions_paginated(&db, 2, 3).unwrap();
        assert_eq!(items2.len(), 2);
    }

    // ── Requests ─────────────────────────────────────────────────────

    #[test]
    fn insert_request_and_query_stats() {
        let db = test_db();
        let s = test_session("deadbeef00000003", "openai");
        queries::insert_session(&db, &s).unwrap();

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        queries::insert_request(
            &db,
            "req-1",
            &s.id,
            1,
            "openai",
            "gpt-4o",
            1000,
            400,
            600,
            Some(0.4),
            true,
            50,
            1200,
            0.005,
            0.003,
        )
        .unwrap();

        queries::upsert_daily_usage(&db, &today, "openai", 1000, 400, 600, 0.005, 0.003).unwrap();

        let today_stats = queries::get_today_stats(&db, &today).unwrap();
        assert_eq!(today_stats.requests, 1);
        assert!(today_stats.tokens_saved > 0);
    }

    // ── Session graphs ───────────────────────────────────────────────

    #[test]
    fn upsert_and_retrieve_session_graph() {
        let db = test_db();
        let s = test_session("deadbeef00000004", "anthropic");
        queries::insert_session(&db, &s).unwrap();

        let graph_json = r#"{"sg_version":"1.0","session_id":"x","project_hash":"deadbeef00000004","token_count":42}"#;

        queries::upsert_session_graph(
            &db,
            "graph-1",
            &s.id,
            "deadbeef00000004",
            graph_json,
            42,
            "claude-3-haiku-20240307",
            0.0001,
        )
        .unwrap();

        let retrieved = queries::get_latest_graph_json(&db, "deadbeef00000004").unwrap();
        assert!(retrieved.is_some());
        assert!(retrieved.unwrap().contains("sg_version"));
    }

    #[test]
    fn upsert_session_graph_replaces_previous() {
        let db = test_db();
        let s = test_session("deadbeef00000005", "anthropic");
        queries::insert_session(&db, &s).unwrap();

        queries::upsert_session_graph(
            &db,
            "g1",
            &s.id,
            "deadbeef00000005",
            r#"{"v":1}"#,
            10,
            "haiku",
            0.0001,
        )
        .unwrap();
        queries::upsert_session_graph(
            &db,
            "g2",
            &s.id,
            "deadbeef00000005",
            r#"{"v":2}"#,
            10,
            "haiku",
            0.0001,
        )
        .unwrap();

        let retrieved = queries::get_latest_graph_json(&db, "deadbeef00000005")
            .unwrap()
            .unwrap();
        assert!(retrieved.contains(r#""v":2"#));
    }

    #[test]
    fn get_latest_graph_json_returns_none_for_unknown_project() {
        let db = test_db();
        assert!(queries::get_latest_graph_json(&db, "nope")
            .unwrap()
            .is_none());
    }

    // ── Daily usage ──────────────────────────────────────────────────

    #[test]
    fn upsert_daily_usage_accumulates() {
        let db = test_db();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        queries::upsert_daily_usage(&db, &today, "anthropic", 500, 200, 300, 0.002, 0.001).unwrap();
        queries::upsert_daily_usage(&db, &today, "anthropic", 300, 100, 200, 0.001, 0.0005)
            .unwrap();

        // Second upsert should sum into the same row
        let rows = queries::get_token_usage_last_n_days(&db, 1).unwrap();
        assert_eq!(rows.len(), 1);
        let (_, raw, sent) = &rows[0];
        assert_eq!(*raw, 800);
        assert_eq!(*sent, 300);
    }

    // ── Delete all data ──────────────────────────────────────────────

    #[test]
    fn delete_all_data_preserves_settings() {
        let db = test_db();
        let s = test_session("deadbeef00000006", "anthropic");
        queries::insert_session(&db, &s).unwrap();
        queries::set_setting(&db, "proxy_port", "9999").unwrap();

        queries::delete_all_data(&db).unwrap();

        // Sessions gone
        let (_, total) = queries::list_sessions_paginated(&db, 1, 10).unwrap();
        assert_eq!(total, 0);

        // Settings preserved
        assert_eq!(
            queries::get_setting(&db, "proxy_port").unwrap(),
            Some("9999".into())
        );
    }
}
