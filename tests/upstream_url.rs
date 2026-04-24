//! Unit tests for smart upstream URL construction.

use anycode::proxy::pipeline::build_upstream_url;

#[test]
fn no_version_in_base_passes_through() {
    assert_eq!(
        build_upstream_url("https://api.anthropic.com", "/v1/messages"),
        "https://api.anthropic.com/v1/messages"
    );
    assert_eq!(
        build_upstream_url("https://api.deepseek.com/anthropic", "/v1/messages"),
        "https://api.deepseek.com/anthropic/v1/messages"
    );
}

#[test]
fn duplicate_v1_is_deduplicated() {
    assert_eq!(
        build_upstream_url("https://api.openai.com/v1", "/v1/chat/completions"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        build_upstream_url("https://api.deepseek.com/v1/", "/v1/messages"),
        "https://api.deepseek.com/v1/messages"
    );
}

#[test]
fn non_matching_versions_are_left_alone() {
    // Base has v2, path has /v1 — these name different APIs, don't merge.
    assert_eq!(
        build_upstream_url("https://api.example.com/v2", "/v1/messages"),
        "https://api.example.com/v2/v1/messages"
    );
}

#[test]
fn version_lookalikes_are_not_stripped() {
    // v1beta must not match v1.
    assert_eq!(
        build_upstream_url("https://api.example.com/v1", "/v10/messages"),
        "https://api.example.com/v1/v10/messages"
    );
    // vX (not a digit) isn't a version.
    assert_eq!(
        build_upstream_url("https://api.example.com/vx", "/vx/foo"),
        "https://api.example.com/vx/vx/foo"
    );
}

#[test]
fn query_string_preserved() {
    assert_eq!(
        build_upstream_url("https://api.openai.com/v1", "/v1/models?limit=10"),
        "https://api.openai.com/v1/models?limit=10"
    );
}

#[test]
fn root_path_unchanged() {
    assert_eq!(
        build_upstream_url("https://api.openai.com/v1", "/"),
        "https://api.openai.com/v1/"
    );
}
