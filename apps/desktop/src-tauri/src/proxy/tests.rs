//! Proxy pipeline, compression, and forwarding unit tests.

#[cfg(test)]
mod proxy_tests {
    use axum::http::HeaderMap;

    use crate::proxy::forward;
    use crate::proxy::intercept;

    // ── Provider detection ───────────────────────────────────────────

    #[test]
    fn detect_anthropic_via_x_api_key_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "sk-ant-api03-abc123".parse().unwrap());
        let provider = forward::detect_provider(&headers);
        assert_eq!(provider.as_str(), "anthropic");
    }

    #[test]
    fn detect_openrouter_via_bearer_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-or-v1-abc123".parse().unwrap());
        let provider = forward::detect_provider(&headers);
        assert_eq!(provider.as_str(), "openrouter");
    }

    #[test]
    fn detect_anthropic_via_bearer_sk_ant() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-ant-api03-abc123".parse().unwrap());
        let provider = forward::detect_provider(&headers);
        assert_eq!(provider.as_str(), "anthropic");
    }

    #[test]
    fn detect_openai_via_bearer_sk() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-proj-abc123".parse().unwrap());
        let provider = forward::detect_provider(&headers);
        assert_eq!(provider.as_str(), "openai");
    }

    #[test]
    fn detect_openai_default_when_no_headers() {
        let headers = HeaderMap::new();
        let provider = forward::detect_provider(&headers);
        assert_eq!(provider.as_str(), "openai");
    }

    // ── API key extraction ───────────────────────────────────────────

    #[test]
    fn extract_anthropic_api_key_from_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "sk-ant-api03-secret".parse().unwrap());
        let key = forward::extract_api_key(&headers, &forward::Provider::Anthropic);
        assert_eq!(key, Some("sk-ant-api03-secret".into()));
    }

    #[test]
    fn extract_openai_api_key_from_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-secret-key".parse().unwrap());
        let key = forward::extract_api_key(&headers, &forward::Provider::OpenAI);
        assert_eq!(key, Some("sk-secret-key".into()));
    }

    #[test]
    fn extract_api_key_returns_none_when_missing() {
        let headers = HeaderMap::new();
        let key = forward::extract_api_key(&headers, &forward::Provider::Anthropic);
        assert_eq!(key, None);
    }

    // ── Token estimation ─────────────────────────────────────────────

    #[test]
    fn estimate_tokens_from_json() {
        let body = serde_json::json!({
            "model": "claude-sonnet",
            "messages": [{"role": "user", "content": "hello world"}]
        });
        let tokens = forward::estimate_tokens(&body);
        assert!(tokens > 0);
        assert!(tokens < 100);
    }

    #[test]
    fn bytes_to_tokens_rounds_up() {
        assert_eq!(forward::bytes_to_tokens(0), 0);
        assert_eq!(forward::bytes_to_tokens(1), 1);
        assert_eq!(forward::bytes_to_tokens(4), 1);
        assert_eq!(forward::bytes_to_tokens(5), 2);
        assert_eq!(forward::bytes_to_tokens(400), 100);
    }

    // ── Cost computation ─────────────────────────────────────────────

    #[test]
    fn compute_cost_for_known_models() {
        // Claude Sonnet: $3/M input, $15/M output
        let cost = forward::compute_cost("claude-sonnet-4-20250514", 1_000_000, 0);
        assert!((cost - 3.0).abs() < 0.01);

        let cost_out = forward::compute_cost("claude-sonnet-4-20250514", 0, 1_000_000);
        assert!((cost_out - 15.0).abs() < 0.01);
    }

    #[test]
    fn compute_cost_for_gpt4o() {
        let cost = forward::compute_cost("gpt-4o", 1_000_000, 0);
        assert!((cost - 2.50).abs() < 0.01);
    }

    #[test]
    fn compute_cost_for_unknown_model_uses_default() {
        let cost = forward::compute_cost("some-unknown-model", 1_000_000, 0);
        assert!((cost - 2.50).abs() < 0.01); // default is gpt-4o pricing
    }

    #[test]
    fn compute_cost_for_deepseek_chat() {
        let cost = forward::compute_cost("deepseek-chat", 1_000_000, 1_000_000);
        assert!((cost - 1.37).abs() < 0.01); // $0.27 + $1.10
    }

    #[test]
    fn compute_cost_for_deepseek_reasoner() {
        let cost = forward::compute_cost("deepseek-reasoner", 1_000_000, 1_000_000);
        assert!((cost - 2.74).abs() < 0.01); // $0.55 + $2.19
    }

    // ── Model extraction ─────────────────────────────────────────────

    #[test]
    fn extract_model_from_body() {
        let body = serde_json::json!({"model": "claude-sonnet-4"});
        assert_eq!(forward::extract_model(&body), "claude-sonnet-4");
    }

    #[test]
    fn extract_model_defaults_to_unknown() {
        let body = serde_json::json!({});
        assert_eq!(forward::extract_model(&body), "unknown");
    }

    // ── Tool detection ───────────────────────────────────────────────

    #[test]
    fn detect_claude_code_tool() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "claude-code/1.0".parse().unwrap());
        let tool = forward::detect_tool(&headers, &forward::Provider::Anthropic);
        assert_eq!(tool, Some("claude-code".into()));
    }

    #[test]
    fn detect_cursor_tool() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "Cursor/1.0".parse().unwrap());
        let tool = forward::detect_tool(&headers, &forward::Provider::OpenAI);
        assert_eq!(tool, Some("cursor".into()));
    }

    #[test]
    fn detect_windsurf_tool() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "windsurf/1.0".parse().unwrap());
        let tool = forward::detect_tool(&headers, &forward::Provider::OpenAI);
        assert_eq!(tool, Some("windsurf".into()));
    }

    #[test]
    fn detect_tool_defaults_to_none() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "curl/8.0".parse().unwrap());
        let tool = forward::detect_tool(&headers, &forward::Provider::OpenAI);
        assert_eq!(tool, None);
    }

    // ── System prompt extraction ─────────────────────────────────────

    #[test]
    fn extract_anthropic_system_from_string() {
        let body = serde_json::json!({
            "model": "claude-sonnet",
            "system": "You are a helpful assistant",
            "messages": [{"role": "user", "content": "hi"}]
        });
        assert_eq!(
            intercept::extract_anthropic_system(&body),
            Some("You are a helpful assistant")
        );
    }

    #[test]
    fn extract_anthropic_system_from_array() {
        let body = serde_json::json!({
            "model": "claude-sonnet",
            "system": [{"type": "text", "text": "You are helpful"}],
            "messages": [{"role": "user", "content": "hi"}]
        });
        assert_eq!(
            intercept::extract_anthropic_system(&body),
            Some("You are helpful")
        );
    }

    #[test]
    fn extract_openai_system_from_messages() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "hi"}
            ]
        });
        assert_eq!(
            intercept::extract_openai_system(&body),
            Some("You are helpful")
        );
    }

    #[test]
    fn extract_openai_system_returns_none_when_no_system_message() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "hi"}
            ]
        });
        assert_eq!(intercept::extract_openai_system(&body), None);
    }
}
