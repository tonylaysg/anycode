//! Integration tests for POST /api/config/backends/{name}/copy
//!
//! These tests exercise the handler through a real HTTP listener to keep the
//! `webui::api` module private — they drive the same path a browser would.

use std::time::Duration;

use anycode::backend::BackendState;
use anycode::cli_mode::CliMode;
use anycode::config::{Backend, Config, ConfigStore};
use anycode::proxy::webui::{bind_webui, serve_webui, WebuiState};
use tempfile::TempDir;

mod common;

fn make_backend(name: &str, api_key: Option<&str>) -> Backend {
    Backend {
        name: name.to_string(),
        display_name: format!("Display {name}"),
        base_url: "https://example.test".to_string(),
        api_key: api_key.map(|s| s.to_string()),
        ..Backend::default()
    }
}

/// Spawn the WebUI server against an in-memory config with the given Claude/Copilot backends.
/// Returns (base_url, TempDir holding config path, ConfigStore so tests can inspect on-disk state).
async fn spawn_webui(
    claude_backends: Vec<Backend>,
    copilot_backends: Vec<Backend>,
    running_mode: CliMode,
) -> (String, TempDir, ConfigStore) {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let mut cfg = Config::default();
    // Replace Claude backends (Config::default seeds one).
    cfg.claude.backends = claude_backends;
    cfg.claude.defaults.active = cfg
        .claude
        .backends
        .first()
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "anthropic".to_string());

    cfg.copilot.backends = copilot_backends;
    cfg.copilot.defaults.active = cfg
        .copilot
        .backends
        .first()
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "copilot".to_string());

    // Persist so save_config has a real file to write to.
    anycode::config::save_config(&config_path, &cfg).expect("write config");

    let store = ConfigStore::new(cfg.clone(), config_path.clone());

    let profile = cfg.profile(running_mode).clone();
    let backend_state = BackendState::from_config(profile).expect("backend state");

    let state = WebuiState {
        config_store: store.clone(),
        backend_state,
        cli_mode: running_mode,
    };

    let (addr, listener) = bind_webui("127.0.0.1:0").await.expect("bind");
    tokio::spawn(async move {
        let _ = serve_webui(listener, state, None, None).await;
    });

    assert!(common::wait_for_server(addr, Duration::from_secs(2)).await);
    (format!("http://{addr}"), tmp, store)
}

#[tokio::test]
async fn clone_within_same_profile_preserves_api_key_and_fields() {
    let src = make_backend("src", Some("secret-123"));
    let (base, _tmp, store) = spawn_webui(vec![src], vec![], CliMode::Claude).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/config/backends/src/copy?profile=claude"))
        .json(&serde_json::json!({
            "target_profile": "claude",
            "new_name": "src-copy",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "body={}", resp.text().await.unwrap());

    // The persisted config must have both backends with matching api_key.
    let cfg = store.get();
    let backends = &cfg.profile(CliMode::Claude).backends;
    assert_eq!(backends.len(), 2);
    let copy = backends.iter().find(|b| b.name == "src-copy").unwrap();
    assert_eq!(copy.api_key.as_deref(), Some("secret-123"));
    assert_eq!(copy.display_name, "Display src");
    assert_eq!(copy.base_url, "https://example.test");
}

#[tokio::test]
async fn copy_to_other_profile_creates_in_target_only() {
    let src = make_backend("src", Some("k"));
    let (base, _tmp, store) =
        spawn_webui(vec![src], vec![make_backend("copilot", None)], CliMode::Claude).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/config/backends/src/copy?profile=claude"))
        .json(&serde_json::json!({
            "target_profile": "copilot",
            "new_name": "from-claude",
            "new_display_name": "From Claude",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let cfg = store.get();
    // Source profile unchanged.
    assert_eq!(cfg.profile(CliMode::Claude).backends.len(), 1);
    // Target profile gained one.
    let copilot = &cfg.profile(CliMode::Copilot).backends;
    assert_eq!(copilot.len(), 2);
    let new = copilot.iter().find(|b| b.name == "from-claude").unwrap();
    assert_eq!(new.api_key.as_deref(), Some("k"));
    assert_eq!(new.display_name, "From Claude");
}

/// Regression: copying the *first* backend into an empty copilot profile used to
/// fail `validate_for` because `defaults.active` still held the stale "claude"
/// default. post_copy_backend must self-heal by adopting the new backend.
#[tokio::test]
async fn copy_to_empty_profile_self_heals_active() {
    let src = make_backend("src", Some("sek"));
    let (base, _tmp, store) = spawn_webui(
        vec![src],
        vec![], // copilot profile starts empty; active defaults to "claude"
        CliMode::Claude,
    )
    .await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/config/backends/src/copy?profile=claude"))
        .json(&serde_json::json!({
            "target_profile": "copilot",
            "new_name": "moved",
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    assert_eq!(status, 200, "copy failed: {body}");

    let cfg = store.get();
    let cp = cfg.profile(CliMode::Copilot);
    assert_eq!(cp.backends.len(), 1);
    assert_eq!(cp.backends[0].name, "moved");
    // Self-healed: active now points at the only backend.
    assert_eq!(cp.defaults.active, "moved");
}

#[tokio::test]
async fn copy_rejects_duplicate_name_in_target() {
    let src = make_backend("src", None);
    let dup = make_backend("dup", None);
    let (base, _tmp, store) = spawn_webui(vec![src, dup], vec![], CliMode::Claude).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/config/backends/src/copy?profile=claude"))
        .json(&serde_json::json!({
            "target_profile": "claude",
            "new_name": "dup",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);

    // No new backend added.
    assert_eq!(store.get().profile(CliMode::Claude).backends.len(), 2);
}

#[tokio::test]
async fn copy_rejects_missing_source() {
    let (base, _tmp, _store) =
        spawn_webui(vec![make_backend("only", None)], vec![], CliMode::Claude).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/config/backends/ghost/copy?profile=claude"))
        .json(&serde_json::json!({
            "target_profile": "claude",
            "new_name": "whatever",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn copy_rejects_empty_new_name() {
    let (base, _tmp, _store) =
        spawn_webui(vec![make_backend("src", None)], vec![], CliMode::Claude).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/config/backends/src/copy?profile=claude"))
        .json(&serde_json::json!({
            "target_profile": "claude",
            "new_name": "   ",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// Regression: saving the copilot profile failed with 400 because `Defaults::default()`
/// sets `active = "claude"` while user-added copilot backends have different names.
/// The PUT handler must self-heal by adopting the first backend as active.
#[tokio::test]
async fn put_config_self_heals_stale_active_backend() {
    // Running instance is Claude so editing the copilot profile only persists to disk.
    let (base, _tmp, store) = spawn_webui(
        vec![make_backend("anthropic", Some("k"))],
        vec![], // copilot starts empty; defaults.active is still "claude" (stale).
        CliMode::Claude,
    )
    .await;

    // Simulate the WebUI adding a backend to the copilot profile.
    let body = serde_json::json!({
        "profile": "copilot",
        "defaults": {
            "active": "claude",  // stale, doesn't match any backend
            "timeout_seconds": 30,
            "connect_timeout_seconds": 5,
            "idle_timeout_seconds": 60,
            "pool_idle_timeout_seconds": 90,
            "pool_max_idle_per_host": 8,
            "max_retries": 3,
            "retry_backoff_base_ms": 100
        },
        "backends": [{
            "name": "openrouter",
            "display_name": "OpenRouter",
            "base_url": "https://openrouter.ai",
            "auth_type": "passthrough",
            "api_key_input": "sk-xxx",
            "api_key_set": false,
            "model_opus": null, "model_opus_max_effort": null,
            "model_sonnet": null, "model_sonnet_max_effort": null,
            "model_haiku": null, "model_haiku_max_effort": null,
            "thinking_compat": null,
            "thinking_budget_tokens": null,
            "pricing": null
        }],
        "agents": null
    });

    let resp = reqwest::Client::new()
        .put(format!("{base}/api/config?profile=copilot"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    assert_eq!(status, 200, "put failed: {text}");

    let cfg = store.get();
    let cp = cfg.profile(CliMode::Copilot);
    assert_eq!(cp.backends.len(), 1);
    assert_eq!(cp.backends[0].name, "openrouter");
    // Self-healed: active now points at the only backend.
    assert_eq!(cp.defaults.active, "openrouter");
}
