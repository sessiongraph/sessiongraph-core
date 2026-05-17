//! Graph extraction, injection, and budget enforcement tests.

#[cfg(test)]
mod graph_tests {
    use rusqlite::Connection;
    use serde_json::json;

    use crate::graph::{injector, schema::SessionGraph};

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        let migration_sql = include_str!("../db/migrations/001_init.sql");
        conn.execute_batch(migration_sql).unwrap();
        // Disable FK for tests that insert graphs without sessions,
        // or we can insert dummy sessions. Let's use session rows.
        conn
    }

    fn insert_dummy_session(db: &Connection, session_id: &str, project_hash: &str) {
        db.execute(
            "INSERT INTO sessions (id, project_hash, project_name, provider, started_at, status)
             VALUES (?1, ?2, 'test', 'anthropic', datetime('now'), 'active')",
            rusqlite::params![session_id, project_hash],
        )
        .unwrap();
    }

    // ── parse_and_validate_graph ─────────────────────────────────────

    #[test]
    fn parse_valid_graph_json() {
        let json = r#"{
            "sg_version": "1.0",
            "session_id": "s1",
            "project_hash": "ph1",
            "created_at": "",
            "last_updated": "",
            "token_count": 0,
            "project": {"name": "test", "stack": [], "entry_points": [], "package_manager": null},
            "state": {"current_task": "fixing bugs", "progress": "done", "next_steps": [], "blockers": []},
            "decisions": [],
            "conventions": {"naming": null, "structure": null, "patterns": []},
            "files": {"active": [], "read": [], "created": []},
            "errors": []
        }"#;

        // Test via the private parse function via public extractor path
        // We test the public injector path instead
        let db = test_db();
        insert_dummy_session(&db, "s1", "ph1");
        crate::db::queries::upsert_session_graph(
            &db, "g1", "s1", "ph1", json, 42, "haiku", 0.001,
        )
        .unwrap();

        let result = injector::inject(
            &db,
            json!({"system": "original system"}),
            "ph1",
            "anthropic",
            500,
        );

        assert!(result.injected);
        assert!(result.graph_tokens > 0);
        let body = result.body;
        let system = body["system"].as_str().unwrap();
        assert!(system.contains("[SESSIONGRAPH CONTEXT"));
        assert!(system.contains("original system"));
    }

    #[test]
    fn inject_handles_nonexistent_graph() {
        let db = test_db();
        let body = json!({"system": "hello"});

        let result = injector::inject(&db, body.clone(), "no-such-project", "anthropic", 500);

        assert!(!result.injected);
        assert_eq!(result.graph_tokens, 0);
        assert_eq!(result.body["system"].as_str().unwrap(), "hello");
    }

    #[test]
    fn inject_openai_creates_system_message_if_none() {
        let db = test_db();
        insert_dummy_session(&db, "sx", "ox");
        let json = r#"{"sg_version":"1.0","session_id":"x","project_hash":"ox","token_count":10,"project":{},"state":{},"decisions":[],"conventions":{},"files":{},"errors":[]}"#;
        crate::db::queries::upsert_session_graph(&db, "gx", "sx", "ox", json, 10, "haiku", 0.001).unwrap();

        let body = json!({"messages": [{"role": "user", "content": "hi"}]});
        let result = injector::inject(&db, body, "ox", "openai", 500);

        assert!(result.injected);
        let msgs = result.body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert!(msgs[0]["content"].as_str().unwrap().contains("[SESSIONGRAPH CONTEXT"));
    }

    #[test]
    fn inject_openai_prepends_to_existing_system_message() {
        let db = test_db();
        insert_dummy_session(&db, "sy", "oy");
        let json = r#"{"sg_version":"1.0","session_id":"x","project_hash":"oy","token_count":10,"project":{},"state":{},"decisions":[],"conventions":{},"files":{},"errors":[]}"#;
        crate::db::queries::upsert_session_graph(&db, "gy", "sy", "oy", json, 10, "haiku", 0.001).unwrap();

        let body = json!({"messages": [
            {"role": "system", "content": "existing system"},
            {"role": "user", "content": "hi"}
        ]});
        let result = injector::inject(&db, body, "oy", "openai", 500);

        assert!(result.injected);
        let msgs = result.body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        let content = msgs[0]["content"].as_str().unwrap();
        assert!(content.contains("existing system"));
        assert!(content.contains("[SESSIONGRAPH CONTEXT"));
    }

    // ── Budget enforcement ───────────────────────────────────────────

    #[test]
    fn enforce_graph_budget_doesnt_truncate_small_graph() {
        let small = r#"{"sg_version":"1.0","session_id":"x","project_hash":"ph","token_count":5}"#;
        // This is a private function — tested via injector with tiny budget
        let db = test_db();
        insert_dummy_session(&db, "ss", "phs");
        crate::db::queries::upsert_session_graph(&db, "gs", "ss", "phs", small, 5, "haiku", 0.001).unwrap();

        let body = json!({"system": "s"});
        let result = injector::inject(&db, body, "phs", "anthropic", 5000);
        assert!(result.injected);
        // Should include the full graph
        let system = result.body["system"].as_str().unwrap();
        assert!(system.contains("sg_version"));
    }

    #[test]
    fn enforce_graph_budget_truncates_large_graph() {
        // Build a very large graph JSON
        let big_files: Vec<String> = (0..500).map(|i| format!("/very/long/path/to/file_{}.ts", i)).collect();
        let big_graph = json!({
            "sg_version": "1.0",
            "session_id": "x",
            "project_hash": "phl",
            "created_at": "",
            "last_updated": "",
            "token_count": 5000,
            "project": {"name": "big", "stack": [], "entry_points": [], "package_manager": null},
            "state": {"current_task": "x", "progress": "x", "next_steps": [], "blockers": []},
            "decisions": [],
            "conventions": {"naming": null, "structure": null, "patterns": []},
            "files": {"active": big_files, "read": [], "created": []},
            "errors": []
        });
        let big_json = serde_json::to_string(&big_graph).unwrap();

        let db = test_db();
        insert_dummy_session(&db, "sl", "phl");
        crate::db::queries::upsert_session_graph(&db, "gl", "sl", "phl", &big_json, 5000, "haiku", 0.001).unwrap();

        let body = json!({"system": "s"});
        // Budget of 50 tokens (~200 chars) — should strip files, errors, etc.
        let result = injector::inject(&db, body, "phl", "anthropic", 50);

        assert!(result.injected);
        // Graph should be significantly smaller than the raw 5000-token input.
        // After stripping files/errors/conventions, only state + marker text remain.
        assert!(result.graph_tokens < 200, "expected truncated graph (<200 tokens), got {}", result.graph_tokens);
    }

    // ── SessionGraph schema ──────────────────────────────────────────

    #[test]
    fn session_graph_serialization_roundtrip() {
        let graph = SessionGraph {
            sg_version: "1.0".into(),
            session_id: "s1".into(),
            project_hash: "ph1".into(),
            created_at: "2025-01-01".into(),
            last_updated: "2025-01-01".into(),
            token_count: 100,
            project: crate::graph::schema::ProjectInfo {
                name: Some("test".into()),
                stack: vec!["rust".into(), "typescript".into()],
                entry_points: vec!["src/main.rs".into()],
                package_manager: Some("pnpm".into()),
            },
            state: crate::graph::schema::WorkState {
                current_task: Some("fixing".into()),
                progress: Some("50%".into()),
                next_steps: vec!["deploy".into()],
                blockers: vec![],
            },
            decisions: vec![crate::graph::schema::Decision {
                topic: "auth".into(),
                decision: "use jwt".into(),
                rationale: "simple".into(),
            }],
            conventions: crate::graph::schema::Conventions {
                naming: Some("snake_case".into()),
                structure: Some("flat".into()),
                patterns: vec!["repository".into()],
            },
            files: crate::graph::schema::FilesInfo {
                active: vec!["src/lib.rs".into()],
                read: vec!["README.md".into()],
                created: vec![],
            },
            errors: vec![crate::graph::schema::ErrorEntry {
                file: Some("main.rs".into()),
                description: "compile error".into(),
                resolution: Some("fixed".into()),
            }],
        };

        let json = serde_json::to_string(&graph).unwrap();
        let parsed: SessionGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sg_version, "1.0");
        assert_eq!(parsed.project.name.unwrap(), "test");
        assert_eq!(parsed.state.current_task.unwrap(), "fixing");
        assert_eq!(parsed.decisions.len(), 1);
        assert_eq!(parsed.decisions[0].topic, "auth");
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.files.active[0], "src/lib.rs");
    }
}
