mod common;

use anycode::backend::{BackendError, BackendState};
use anycode::config::{Backend, Config, Defaults, ProxyConfig, TerminalConfig, DebugLoggingConfig};
use std::collections::HashMap;

fn create_test_config() -> Config {
    Config {
        proxy: ProxyConfig::default(),
        webui: anycode::config::WebuiConfig::default(),
        terminal: TerminalConfig::default(),
        debug_logging: DebugLoggingConfig::default(),
    claude: anycode::config::CliProfile {
        defaults: Defaults {
            active: "backend1".to_string(),
            timeout_seconds: 30,
            connect_timeout_seconds: 5,
            idle_timeout_seconds: 60,
            pool_idle_timeout_seconds: 90,
            pool_max_idle_per_host: 8,
            max_retries: 3,
            retry_backoff_base_ms: 100,
        },
        claude_settings: HashMap::new(),
        backends: vec![
            Backend {
                name: "backend1".to_string(),
                display_name: "Backend 1".to_string(),
                base_url: "https://api1.example.com".to_string(),
                auth_type_str: "api_key".to_string(),
                api_key: None,
                pricing: None,
                thinking_compat: None,
                thinking_budget_tokens: None,
                model_opus: None,
                model_sonnet: None,
                model_haiku: None,
            model_opus_max_effort: None,
            model_sonnet_max_effort: None,
            model_haiku_max_effort: None,
            models_path: None,
            wire_api: None,
            strip_request_prefix: None,
        },
            Backend {
                name: "backend2".to_string(),
                display_name: "Backend 2".to_string(),
                base_url: "https://api2.example.com".to_string(),
                auth_type_str: "bearer".to_string(),
                api_key: None,
                pricing: None,
                thinking_compat: None,
                thinking_budget_tokens: None,
                model_opus: None,
                model_sonnet: None,
                model_haiku: None,
            model_opus_max_effort: None,
            model_sonnet_max_effort: None,
            model_haiku_max_effort: None,
            models_path: None,
            wire_api: None,
            strip_request_prefix: None,
        },
        ],
        agents: None,
    ..Default::default()
},
    ..Default::default()

}
}

#[test]
fn test_from_config_with_default() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();
    assert_eq!(state.get_active_backend(), "backend1");
}

#[test]
fn test_from_config_no_default_uses_first() {
    let mut config = create_test_config();
    config.claude.defaults.active = "".to_string();
    let state = BackendState::from_config(config.claude.clone()).unwrap();
    assert_eq!(state.get_active_backend(), "backend1");
}

#[test]
fn test_from_config_empty_backends_fails() {
    let mut config = create_test_config();
    config.claude.backends.clear();
    assert!(matches!(
        BackendState::from_config(config.claude.clone()),
        Err(BackendError::NoBackendsConfigured)
    ));
}

#[test]
fn test_from_config_invalid_default_fails() {
    let mut config = create_test_config();
    config.claude.defaults.active = "nonexistent".to_string();
    assert!(matches!(
        BackendState::from_config(config.claude.clone()),
        Err(BackendError::BackendNotFound { .. })
    ));
}

#[test]
fn test_switch_backend_success() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    assert_eq!(state.get_active_backend(), "backend1");
    state.switch_backend("backend2").unwrap();
    assert_eq!(state.get_active_backend(), "backend2");
}

#[test]
fn test_switch_backend_invalid_fails() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    assert!(matches!(
        state.switch_backend("nonexistent"),
        Err(BackendError::BackendNotFound { .. })
    ));
    // State should be unchanged
    assert_eq!(state.get_active_backend(), "backend1");
}

#[test]
fn test_switch_backend_same_noop() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    state.switch_backend("backend1").unwrap();
    assert_eq!(state.get_active_backend(), "backend1");
    // Should not create a log entry for no-op switch (only initial entry)
    assert_eq!(state.get_switch_log().len(), 1);
}

#[test]
fn test_switch_log() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    state.switch_backend("backend2").unwrap();
    state.switch_backend("backend1").unwrap();

    let log = state.get_switch_log();
    assert_eq!(log.len(), 3); // initial + 2 switches
    assert_eq!(log[0].old_backend, None);
    assert_eq!(log[0].new_backend, "backend1".to_string());
    assert_eq!(log[1].old_backend, Some("backend1".to_string()));
    assert_eq!(log[1].new_backend, "backend2".to_string());
    assert_eq!(log[2].old_backend, Some("backend2".to_string()));
    assert_eq!(log[2].new_backend, "backend1".to_string());
}

#[test]
fn test_validate_backend() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    assert!(state.validate_backend("backend1"));
    assert!(state.validate_backend("backend2"));
    assert!(!state.validate_backend("nonexistent"));
}

#[test]
fn test_list_backends() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    let backends = state.list_backends();
    assert_eq!(backends.len(), 2);
    assert!(backends.contains(&"backend1".to_string()));
    assert!(backends.contains(&"backend2".to_string()));
}

#[test]
fn test_get_active_backend_config() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    let backend = state.get_active_backend_config().unwrap();
    assert_eq!(backend.name, "backend1");
    assert_eq!(backend.base_url, "https://api1.example.com");
}

#[test]
fn test_update_config() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    // Switch to backend2
    state.switch_backend("backend2").unwrap();

    // Update config with new backend
    let mut new_config = config;
    new_config.claude.backends.push(Backend {
        name: "backend3".to_string(),
        display_name: "Backend 3".to_string(),
        base_url: "https://api3.example.com".to_string(),
        auth_type_str: "api_key".to_string(),
        api_key: None,
        pricing: None,
        thinking_compat: None,
        thinking_budget_tokens: None,
        model_opus: None,
        model_sonnet: None,
        model_haiku: None,
            model_opus_max_effort: None,
            model_sonnet_max_effort: None,
            model_haiku_max_effort: None,
            models_path: None,
            wire_api: None,
            strip_request_prefix: None,
        });

    state.update_config(new_config.claude.clone()).unwrap();
    assert_eq!(state.get_active_backend(), "backend2"); // Should stay the same
    assert!(state.validate_backend("backend3"));
}

#[test]
fn test_update_config_removes_active_backend() {
    let config = create_test_config();
    let state = BackendState::from_config(config.claude.clone()).unwrap();

    // Switch to backend2
    state.switch_backend("backend2").unwrap();

    // Update config removing backend2
    let mut new_config = config;
    new_config.claude.backends.retain(|b| b.name != "backend2");

    state.update_config(new_config.claude.clone()).unwrap();
    // Should switch to default (backend1)
    assert_eq!(state.get_active_backend(), "backend1");
}
