//! Regression tests for Copilot BYOK (Bring-Your-Own-Key) env var injection.
//!
//! When anycode spawns the GitHub Copilot CLI, it must inject the
//! `COPILOT_OFFLINE` + `COPILOT_PROVIDER_*` family so the CLI completely
//! skips GitHub OAuth and routes all model traffic at the anycode proxy.
//!
//! These tests pin that contract — if anyone re-introduces the old
//! `ANTHROPIC_BASE_URL` / `COPILOT_API_URL` injection the build will fail.

use anycode::args::build_spawn_params;
use anycode::config::ClaudeSettingsManager;

fn settings() -> ClaudeSettingsManager {
    ClaudeSettingsManager::new()
}

fn env_get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn env_has(env: &[(String, String)], key: &str) -> bool {
    env.iter().any(|(k, _)| k == key)
}

#[test]
fn copilot_spawn_injects_byok_env_vars() {
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:54321",
        "COPILOT_API_URL",
        "copilot",
        "session-tok-xyz",
        &settings(),
        None,
        Some(54321),
        false,
        "anthropic",
    );

    assert_eq!(env_get(&p.env, "COPILOT_OFFLINE"), Some("true"));
    assert_eq!(
        env_get(&p.env, "COPILOT_PROVIDER_BASE_URL"),
        Some("http://127.0.0.1:54321")
    );
    assert_eq!(env_get(&p.env, "COPILOT_PROVIDER_TYPE"), Some("anthropic"));
    assert_eq!(
        env_get(&p.env, "COPILOT_PROVIDER_API_KEY"),
        Some("session-tok-xyz")
    );
    // Anthropic wire ignores WIRE_API and Copilot CLI logs a warning if it's
    // set. anycode therefore omits it for this provider.
    assert!(!env_has(&p.env, "COPILOT_PROVIDER_WIRE_API"));
    assert!(env_has(&p.env, "COPILOT_HOME"));
}

#[test]
fn copilot_openai_spawn_sets_wire_api_completions() {
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:54321",
        "COPILOT_API_URL",
        "copilot",
        "tok",
        &settings(),
        None,
        Some(54321),
        false,
        "openai",
    );
    assert_eq!(env_get(&p.env, "COPILOT_PROVIDER_TYPE"), Some("openai"));
    assert_eq!(
        env_get(&p.env, "COPILOT_PROVIDER_WIRE_API"),
        Some("completions")
    );
}

#[test]
fn copilot_openai_responses_spawn_sets_wire_api_responses() {
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:54321",
        "COPILOT_API_URL",
        "copilot",
        "tok",
        &settings(),
        None,
        Some(54321),
        false,
        "openai-responses",
    );
    assert_eq!(env_get(&p.env, "COPILOT_PROVIDER_TYPE"), Some("openai"));
    assert_eq!(
        env_get(&p.env, "COPILOT_PROVIDER_WIRE_API"),
        Some("responses")
    );
}

#[test]
fn copilot_spawn_does_not_inject_legacy_anthropic_env() {
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:54321",
        "COPILOT_API_URL",
        "copilot",
        "tok",
        &settings(),
        None,
        Some(54321),
        false,
        "anthropic",
    );

    // Pre-BYOK code injected ANTHROPIC_BASE_URL and a placeholder
    // ANTHROPIC_API_KEY; Copilot CLI ignored them and still showed an OAuth
    // prompt. They must not return.
    assert!(!env_has(&p.env, "ANTHROPIC_BASE_URL"));
    assert!(!env_has(&p.env, "ANTHROPIC_API_KEY"));
    assert!(!env_has(&p.env, "ANTHROPIC_CUSTOM_HEADERS"));

    // The old `COPILOT_API_URL=<proxy>` injection repointed Copilot's
    // GitHub backend but didn't disable OAuth. Replaced by the BYOK set.
    assert!(!env_has(&p.env, "COPILOT_API_URL"));
}

#[test]
fn copilot_spawn_propagates_provider_type() {
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:54321",
        "COPILOT_API_URL",
        "copilot",
        "tok",
        &settings(),
        None,
        Some(54321),
        false,
        "openai",
    );
    assert_eq!(env_get(&p.env, "COPILOT_PROVIDER_TYPE"), Some("openai"));
    // WIRE_API defaults to "completions" for the "openai" selector.
    assert_eq!(
        env_get(&p.env, "COPILOT_PROVIDER_WIRE_API"),
        Some("completions")
    );
}

#[test]
fn claude_spawn_unaffected_by_byok_changes() {
    let p = build_spawn_params(
        &[],
        "http://127.0.0.1:54321",
        "ANTHROPIC_BASE_URL",
        "claude",
        "tok",
        &settings(),
        None,
        Some(54321),
        false,
        "anthropic",
    );

    // Claude path keeps its existing env contract.
    assert_eq!(
        env_get(&p.env, "ANTHROPIC_BASE_URL"),
        Some("http://127.0.0.1:54321")
    );
    // None of the COPILOT_* BYOK vars must leak into a Claude spawn.
    assert!(!env_has(&p.env, "COPILOT_OFFLINE"));
    assert!(!env_has(&p.env, "COPILOT_PROVIDER_BASE_URL"));
    assert!(!env_has(&p.env, "COPILOT_PROVIDER_TYPE"));
    assert!(!env_has(&p.env, "COPILOT_PROVIDER_API_KEY"));
}
