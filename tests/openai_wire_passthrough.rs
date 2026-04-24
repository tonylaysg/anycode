//! OpenAI wire pass-through regression test.
//!
//! Verifies that `POST /v1/chat/completions` — the request shape Copilot CLI
//! emits under `COPILOT_PROVIDER_TYPE=openai` — flows cleanly through the
//! proxy pipeline when paired with an OpenAI-compatible backend:
//!
//! * The path + body are forwarded unchanged (modulo model rewriting).
//! * Family-based model mapping still fires (so `claude-sonnet-4` can map to
//!   `deepseek-chat`, etc., driven by the backend config).
//! * The non-streaming response model field is reverse-mapped back so the
//!   client sees the name it asked for.
//!
//! Thinking-compat logic is a no-op on OpenAI bodies (no `thinking` field,
//! no `content[].thinking` blocks) — exercised implicitly: the backend gets
//! a body that still parses as the original OpenAI shape.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use anycode::cli_mode::CliMode;
use anycode::config::{Backend, CliProfile, Config, ConfigStore, Defaults};
use anycode::metrics::DebugLogger;
use anycode::proxy::ProxyServer;

use common::mock_backend::{MockBackend, MockResponse};

fn backend_with_sonnet_map(base_url: &str) -> Backend {
    Backend {
        name: "oai".to_string(),
        display_name: "OAI".to_string(),
        base_url: base_url.to_string(),
        auth_type_str: "passthrough".to_string(),
        api_key: Some("k".to_string()),
        pricing: None,
        thinking_compat: None,
        thinking_budget_tokens: None,
        model_opus: None,
        model_opus_max_effort: None,
        model_sonnet: Some("deepseek-chat".to_string()),
        model_sonnet_max_effort: None,
        model_haiku: None,
        model_haiku_max_effort: None,
        models_path: None,
    }
}

async fn start_proxy(backend: Backend) -> String {
    let config = Config {
        claude: CliProfile {
            defaults: Defaults {
                active: backend.name.clone(),
                ..Default::default()
            },
            backends: vec![backend],
            ..Default::default()
        },
        ..Default::default()
    };
    let config_store = ConfigStore::new(config, PathBuf::from("/tmp/test-openai-wire.toml"));
    let debug_logger = Arc::new(DebugLogger::new(Default::default()));
    let mut server = ProxyServer::new(
        config_store.clone(),
        CliMode::Claude,
        debug_logger,
        Some("tk".to_string()),
    )
    .unwrap();
    let (addr, _) = server.try_bind(&config_store).await.unwrap();
    tokio::spawn(async move { let _ = server.run().await; });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    format!("{}", addr)
}

#[tokio::test]
async fn chat_completions_forwarded_with_model_rewrite() {
    let mock = MockBackend::start().await;
    // Backend replies in OpenAI shape; model echoed as the rewritten name.
    mock.enqueue_response(MockResponse::json(
        r#"{"id":"cmpl-1","object":"chat.completion","model":"deepseek-chat","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#,
    ))
    .await;

    let addr = start_proxy(backend_with_sonnet_map(&mock.base_url())).await;

    let req_body = r#"{"model":"claude-sonnet-4-5","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = reqwest::Client::new()
        .post(format!("http://{}/v1/chat/completions", addr))
        .header("authorization", "Bearer tk")
        .header("content-type", "application/json")
        .body(req_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    // The client must see the original model name — proxy reverse-maps it.
    assert_eq!(body["model"].as_str(), Some("claude-sonnet-4-5"));
    assert_eq!(body["choices"][0]["message"]["content"].as_str(), Some("hi"));

    // Upstream should have received /v1/chat/completions with rewritten model.
    let reqs = mock.captured_requests().await;
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/v1/chat/completions");
    assert_eq!(reqs[0].method, "POST");
    let upstream_body: serde_json::Value =
        serde_json::from_slice(&reqs[0].body).expect("upstream body json");
    assert_eq!(upstream_body["model"].as_str(), Some("deepseek-chat"));
    // Messages array is forwarded verbatim — no wire translation tampering.
    assert_eq!(
        upstream_body["messages"][0]["content"].as_str(),
        Some("hi")
    );
}

#[tokio::test]
async fn chat_completions_streaming_passthrough() {
    let mock = MockBackend::start().await;
    // OpenAI-style SSE stream with a model field in the first chunk.
    mock.enqueue_response(MockResponse::sse(&[
        r#"{"id":"c1","object":"chat.completion.chunk","model":"deepseek-chat","choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#,
        r#"{"id":"c1","object":"chat.completion.chunk","model":"deepseek-chat","choices":[{"index":0,"delta":{"content":"hi"}}]}"#,
        r#"{"id":"c1","object":"chat.completion.chunk","model":"deepseek-chat","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
    ]))
    .await;

    let addr = start_proxy(backend_with_sonnet_map(&mock.base_url())).await;

    let req_body = r#"{"model":"claude-sonnet-4-5","stream":true,"messages":[{"role":"user","content":"hi"}]}"#;
    let resp = reqwest::Client::new()
        .post(format!("http://{}/v1/chat/completions", addr))
        .header("authorization", "Bearer tk")
        .header("content-type", "application/json")
        .body(req_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.contains("text/event-stream"), "got content-type={}", ct);

    let body = resp.text().await.unwrap();
    // Reverse model mapping must rewrite every chunk's model field.
    assert!(
        body.contains(r#""model":"claude-sonnet-4-5""#),
        "stream body should contain reverse-mapped model, got: {}",
        body
    );
    assert!(
        !body.contains(r#""model":"deepseek-chat""#),
        "stream body should NOT leak backend model name, got: {}",
        body
    );

    let reqs = mock.captured_requests().await;
    assert_eq!(reqs[0].path, "/v1/chat/completions");
}
