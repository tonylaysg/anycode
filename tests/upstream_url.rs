//! Unit tests for smart upstream URL construction.

use anycode::proxy::pipeline::build_upstream_url;

#[test]
fn no_version_in_base_passes_through() {
    assert_eq!(
        build_upstream_url("https://api.anthropic.com", None, "/v1/messages"),
        "https://api.anthropic.com/v1/messages"
    );
    assert_eq!(
        build_upstream_url("https://api.deepseek.com/anthropic", None, "/v1/messages"),
        "https://api.deepseek.com/anthropic/v1/messages"
    );
}

#[test]
fn duplicate_v1_is_deduplicated() {
    assert_eq!(
        build_upstream_url("https://api.openai.com/v1", None, "/v1/chat/completions"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        build_upstream_url("https://api.deepseek.com/v1/", None, "/v1/messages"),
        "https://api.deepseek.com/v1/messages"
    );
}

#[test]
fn non_matching_versions_are_left_alone() {
    assert_eq!(
        build_upstream_url("https://api.example.com/v2", None, "/v1/messages"),
        "https://api.example.com/v2/v1/messages"
    );
}

#[test]
fn version_lookalikes_are_not_stripped() {
    assert_eq!(
        build_upstream_url("https://api.example.com/v1", None, "/v10/messages"),
        "https://api.example.com/v1/v10/messages"
    );
    assert_eq!(
        build_upstream_url("https://api.example.com/vx", None, "/vx/foo"),
        "https://api.example.com/vx/vx/foo"
    );
}

#[test]
fn query_string_preserved() {
    assert_eq!(
        build_upstream_url("https://api.openai.com/v1", None, "/v1/models?limit=10"),
        "https://api.openai.com/v1/models?limit=10"
    );
}

#[test]
fn root_path_unchanged() {
    assert_eq!(
        build_upstream_url("https://api.openai.com/v1", None, "/"),
        "https://api.openai.com/v1/"
    );
}

// ── Explicit strip_request_prefix ─────────────────────────────────────────────

#[test]
fn explicit_strip_removes_leading_segment() {
    // Backend exposes /chat/completions (no /v1).
    assert_eq!(
        build_upstream_url("https://api.foo.com", Some("/v1"), "/v1/chat/completions"),
        "https://api.foo.com/chat/completions"
    );
    // Backend exposes /models (no /v1).
    assert_eq!(
        build_upstream_url("https://api.foo.com", Some("/v1"), "/v1/models"),
        "https://api.foo.com/models"
    );
}

#[test]
fn explicit_strip_combined_with_non_v1_base_prefix() {
    // CLI emits /v1/messages, backend lives under /openai with no /v1.
    assert_eq!(
        build_upstream_url("https://api.foo.com/openai", Some("/v1"), "/v1/messages"),
        "https://api.foo.com/openai/messages"
    );
}

#[test]
fn explicit_strip_only_matches_at_boundary() {
    // /v10/... should not be affected by strip="/v1".
    assert_eq!(
        build_upstream_url("https://api.foo.com", Some("/v1"), "/v10/foo"),
        "https://api.foo.com/v10/foo"
    );
}

#[test]
fn explicit_strip_empty_string_is_ignored() {
    assert_eq!(
        build_upstream_url("https://api.foo.com", Some(""), "/v1/messages"),
        "https://api.foo.com/v1/messages"
    );
}

#[test]
fn explicit_strip_takes_precedence_over_dedup() {
    // base ends with /v1 AND strip_prefix is /v1. Explicit strip wins but the
    // outcome is the same: one /v1 remains (from base).
    assert_eq!(
        build_upstream_url("https://api.foo.com/v1", Some("/v1"), "/v1/models"),
        "https://api.foo.com/v1/models"
    );
}
