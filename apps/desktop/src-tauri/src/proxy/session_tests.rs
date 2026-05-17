//! Session lifecycle, hashing, and timeout tests.

#[cfg(test)]
#[allow(clippy::module_inception)]
mod session_tests {
    use chrono::Utc;

    use crate::proxy::session::{self, ActiveSession};

    fn test_session() -> ActiveSession {
        ActiveSession::new(
            "deadbeef00000042".into(),
            "abc123hash".into(),
            Some("test-project".into()),
            "anthropic".into(),
            Some("claude-code".into()),
        )
    }

    // ── Session creation ─────────────────────────────────────────────

    #[test]
    fn new_session_has_zero_counters() {
        let s = test_session();
        assert_eq!(s.message_count, 0);
        assert_eq!(s.tokens_in_raw, 0);
        assert_eq!(s.tokens_in_sent, 0);
        assert_eq!(s.tokens_out, 0);
        assert_eq!(s.cost_usd_raw, 0.0);
        assert_eq!(s.cost_usd_actual, 0.0);
    }

    #[test]
    fn new_session_generates_unique_id() {
        let s1 = test_session();
        let s2 = test_session();
        assert_ne!(s1.id, s2.id);
    }

    #[test]
    fn new_session_records_start_time() {
        let s = test_session();
        let elapsed = Utc::now() - s.started_at;
        assert!(elapsed.num_seconds() < 5);
    }

    // ── Timeout detection ────────────────────────────────────────────

    #[test]
    fn session_not_timed_out_immediately() {
        let s = test_session();
        assert!(!s.is_timed_out(30));
    }

    #[test]
    fn session_timed_out_after_period() {
        let mut s = test_session();
        // Fake the last_request_at to be 31 minutes ago
        s.last_request_at = Utc::now() - chrono::Duration::minutes(31);
        assert!(s.is_timed_out(30));
    }

    #[test]
    fn session_not_timed_out_before_period() {
        let mut s = test_session();
        s.last_request_at = Utc::now() - chrono::Duration::minutes(29);
        assert!(!s.is_timed_out(30));
    }

    #[test]
    fn session_timeout_respects_custom_timeout() {
        let mut s = test_session();
        s.last_request_at = Utc::now() - chrono::Duration::minutes(10);
        assert!(!s.is_timed_out(30));
        assert!(s.is_timed_out(5));
    }

    // ── Message body accumulation ────────────────────────────────────

    #[test]
    fn push_body_keeps_most_recent() {
        let mut s = test_session();
        for i in 0..15 {
            s.push_body(&serde_json::json!({"msg": i}));
        }
        // MAX_MESSAGE_SNAPSHOTS is 10
        assert!(s.recent_bodies.len() <= 10);
        // Last pushed should be last
        let last = s.recent_bodies.last().unwrap();
        assert_eq!(last["msg"], serde_json::json!(14));
    }

    #[test]
    fn push_body_strips_large_fields() {
        let mut s = test_session();
        s.push_body(&serde_json::json!({
            "messages": [{"role": "user", "content": "hello"}],
            "image_data": "base64_aaaaaaaaaaaaaa",
            "base64_file": "more_base64",
            "normal_field": "keep_me"
        }));
        let body = &s.recent_bodies[0];
        assert_eq!(body["normal_field"], "keep_me");
        // Image/base64 fields should be stripped
        assert_eq!(body["image_data"], "[stripped]");
        assert_eq!(body["base64_file"], "[stripped]");
        // Messages should be preserved (capped to 20)
        assert!(body["messages"].is_array());
    }

    #[test]
    fn messages_for_extraction_serializes() {
        let mut s = test_session();
        s.push_body(&serde_json::json!({"role": "user", "content": "test"}));
        let json = s.messages_for_extraction();
        assert!(json.contains("test"));
    }

    // ── Project hash ─────────────────────────────────────────────────

    #[test]
    fn project_hash_from_system_prompt() {
        let hash = session::compute_project_hash(Some("You are working on sessiongraph"), None);
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn project_hash_same_for_same_input() {
        let h1 = session::compute_project_hash(Some("hello world"), None);
        let h2 = session::compute_project_hash(Some("hello world"), None);
        assert_eq!(h1, h2);
    }

    #[test]
    fn project_hash_different_for_different_input() {
        let h1 = session::compute_project_hash(Some("project alpha"), None);
        let h2 = session::compute_project_hash(Some("project beta"), None);
        assert_ne!(h1, h2);
    }

    #[test]
    fn project_hash_fallback_to_unknown() {
        let hash = session::compute_project_hash(None, None);
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn project_hash_truncates_long_input() {
        let long = "x".repeat(500);
        let hash = session::compute_project_hash(Some(&long), None);
        assert_eq!(hash.len(), 16);
        // Should use first 100 chars only
        let short_hash = session::compute_project_hash(Some(&"x".repeat(50)), None);
        assert_ne!(hash, short_hash);
    }

    // ── API key hash ─────────────────────────────────────────────────

    #[test]
    fn api_key_hash_is_stable() {
        let h1 = session::hash_api_key("sk-ant-api03-abc123");
        let h2 = session::hash_api_key("sk-ant-api03-abc123");
        assert_eq!(h1, h2);
    }

    #[test]
    fn api_key_hash_different_for_different_keys() {
        let h1 = session::hash_api_key("key-a");
        let h2 = session::hash_api_key("key-b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn api_key_hash_handles_empty() {
        let hash = session::hash_api_key("");
        assert_eq!(hash, "unknown");
    }

    // ── Project name inference ───────────────────────────────────────

    #[test]
    fn infer_project_name_from_explicit_marker() {
        let name = session::infer_project_name(Some("project: sessiongraph"));
        assert_eq!(name, Some("sessiongraph".into()));
    }

    #[test]
    fn infer_project_name_from_case_insensitive_marker() {
        let name = session::infer_project_name(Some("Project: MyApp"));
        assert_eq!(name, Some("MyApp".into()));
    }

    #[test]
    fn infer_project_name_from_package_json() {
        let name = session::infer_project_name(Some(r#"some text "name":"@scope/package" more"#));
        assert_eq!(name, Some("@scope/package".into()));
    }

    #[test]
    fn infer_project_name_from_github_url() {
        let name = session::infer_project_name(Some(
            "working on https://github.com/user/my-repo/issues/5",
        ));
        assert_eq!(name, Some("my-repo".into()));
    }

    #[test]
    fn infer_project_name_returns_none_for_empty() {
        assert_eq!(session::infer_project_name(None), None);
        assert_eq!(session::infer_project_name(Some("")), None);
    }
}
