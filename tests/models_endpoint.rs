//! Integration tests for the `GET /v1/models` proxy endpoint.
//!
//! Exercises the end-to-end flow:
//!   Copilot CLI → anycode proxy `/v1/models` → active backend upstream →
//!   path-probing (`/v1/models` → fallback `/models`) → cached 30 min.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use anycode::cli_mode::CliMode;
use anycode::config::{Backend, CliProfile, Config, ConfigStore, Defaults};
use anycode::metrics::DebugLogger;
use anycode::proxy::ProxyServer;

use common::mock_backend::{MockBackend, MockResponse};

fn make_backend(name: &str, base_url: &str, models_path: Option<&str>) -> Backend {
    Backend {
        name: name.to_string(),
        display_name: name.to_string(),
        base_url: base_url.to_string(),
        auth_type_str: "passthrough".to_string(),
        api_key: Some("test-key".to_string()),
        pricing: None,
        thinking_compat: None,
        thinking_budget_tokens: None,
        model_opus: None,
        model_opus_max_effort: None,
        model_sonnet: None,
        model_sonnet_max_effort: None,
        model_haiku: None,
        model_haiku_max_effort: None,
        models_path: models_path.map(String::from),
        wire_api: None,
    }
}

async fn start_proxy_with_backend(backend: Backend) -> (String, tokio::task::JoinHandle<()>) {
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

    let config_store = ConfigStore::new(
        config,
        PathBuf::from("/tmp/test-models-endpoint.toml"),
    );
    let debug_logger = Arc::new(DebugLogger::new(Default::default()));
    let mut server = ProxyServer::new(
        config_store.clone(),
        CliMode::Claude,
        debug_logger,
        Some("tk".to_string()),
    )
    .expect("Failed to create proxy server");
    let (addr, _) = server.try_bind(&config_store).await.expect("bind failed");
    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Invalidate cache so previous tests' entries don't leak.
    anycode::proxy::models::invalidate_cache();

    (format!("{}", addr), handle)
}

#[tokio::test]
async fn v1_models_proxied_from_backend_v1_path() {
    let mock = MockBackend::start().await;
    mock.enqueue_response(MockResponse::json(
        r#"{"data":[{"id":"gpt-4","object":"model"}],"object":"list"}"#,
    ))
    .await;

    let backend = make_backend("b", &mock.base_url(), None);
    let (addr, _h) = start_proxy_with_backend(backend).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{}/v1/models", addr))
        .header("authorization", "Bearer tk")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let src = resp
        .headers()
        .get("x-anycode-models-source")
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(src.as_deref(), Some("/v1/models"));
    let body = resp.text().await.unwrap();
    assert!(body.contains("gpt-4"));

    // Mock should have received exactly one probe to /v1/models.
    let reqs = mock.captured_requests().await;
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/v1/models");
}

#[tokio::test]
async fn v1_models_falls_back_to_plain_models_on_404() {
    let mock = MockBackend::start().await;
    // First response: 404 for /v1/models (the probe).
    mock.enqueue_response(MockResponse::error(404, "not found")).await;
    // Second response: 200 for /models.
    mock.enqueue_response(MockResponse::json(
        r#"{"data":[{"id":"llama-3","object":"model"}]}"#,
    ))
    .await;

    let backend = make_backend("b", &mock.base_url(), None);
    let (addr, _h) = start_proxy_with_backend(backend).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{}/v1/models", addr))
        .header("authorization", "Bearer tk")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers()
            .get("x-anycode-models-source")
            .and_then(|v| v.to_str().ok()),
        Some("/models")
    );
    let body = resp.text().await.unwrap();
    assert!(body.contains("llama-3"));

    let reqs = mock.captured_requests().await;
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].path, "/v1/models");
    assert_eq!(reqs[1].path, "/models");
}

#[tokio::test]
async fn v1_models_respects_explicit_models_path() {
    let mock = MockBackend::start().await;
    mock.enqueue_response(MockResponse::json(
        r#"{"data":[{"id":"custom-1"}]}"#,
    ))
    .await;

    let backend = make_backend("b", &mock.base_url(), Some("/api/models"));
    let (addr, _h) = start_proxy_with_backend(backend).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{}/v1/models", addr))
        .header("authorization", "Bearer tk")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    let reqs = mock.captured_requests().await;
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/api/models");
}

#[tokio::test]
async fn v1_models_caches_subsequent_calls() {
    let mock = MockBackend::start().await;
    mock.enqueue_response(MockResponse::json(r#"{"data":[{"id":"m"}]}"#))
        .await;
    // Do NOT enqueue a second response — if the proxy hits the upstream twice
    // the mock will return its default 200+ok body and the test would still
    // pass based on status, so we additionally assert the request count.

    let backend = make_backend("b", &mock.base_url(), None);
    let (addr, _h) = start_proxy_with_backend(backend).await;

    for _ in 0..3 {
        let resp = reqwest::Client::new()
            .get(format!("http://{}/v1/models", addr))
            .header("authorization", "Bearer tk")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }

    let reqs = mock.captured_requests().await;
    assert_eq!(
        reqs.len(),
        1,
        "expected exactly one upstream call (cache hit on 2nd/3rd), got {}",
        reqs.len()
    );
}

#[tokio::test]
async fn v1_models_auth_required() {
    let mock = MockBackend::start().await;
    mock.enqueue_response(MockResponse::json(r#"{"data":[]}"#)).await;

    let backend = make_backend("b", &mock.base_url(), None);
    let (addr, _h) = start_proxy_with_backend(backend).await;

    // No Bearer → 401 from our middleware.
    let resp = reqwest::Client::new()
        .get(format!("http://{}/v1/models", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
}
