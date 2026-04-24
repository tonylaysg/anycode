//! Smart upstream URL builder.
//!
//! Copilot CLI always emits paths prefixed with `/v1` (`/v1/messages`,
//! `/v1/chat/completions`, `/v1/responses`, `/v1/models`). Backends, however,
//! are inconsistent about whether `/v1` belongs in the base URL — or at all:
//!
//! * `https://api.anthropic.com`         → expects `/v1/messages`
//! * `https://api.deepseek.com/anthropic`→ expects `/v1/messages`
//! * `https://api.openai.com/v1`         → expects `/chat/completions`
//! * `https://api.example.com`           → expects `/chat/completions` (no v1!)
//!
//! This helper handles three cases:
//! 1. **Explicit strip** — when `strip_request_prefix` is set (e.g. `"/v1"`),
//!    the matching leading segment is removed from the request path. Use for
//!    backends that refuse `/v1/...` entirely.
//! 2. **Version dedup** — when the base URL ends with `/vN` and the request
//!    path begins with `/vN/` (same `N`), one copy is stripped.
//! 3. **Passthrough** — otherwise, concatenate verbatim.
//!
//! Version matching is strict: `/v1` ≠ `/v2`, and `/v1beta`, `/v10` are not
//! treated as `/v1`.

/// Build an upstream URL from a backend base URL, an optional explicit prefix
/// to strip, and the request path (`path_and_query`).
pub fn build_upstream_url(
    base_url: &str,
    strip_request_prefix: Option<&str>,
    path_and_query: &str,
) -> String {
    let base = base_url.trim_end_matches('/');

    if let Some(prefix) = strip_request_prefix.filter(|s| !s.is_empty()) {
        if let Some(rest) = strip_path_prefix(path_and_query, prefix) {
            return format!("{}{}", base, rest);
        }
    }

    if let Some(base_ver) = trailing_version_segment(base) {
        if let Some(rest) = strip_path_prefix(path_and_query, &format!("/{}", base_ver)) {
            return format!("{}{}", base, rest);
        }
    }

    format!("{}{}", base, path_and_query)
}

fn trailing_version_segment(base: &str) -> Option<&str> {
    let seg = base.rsplit('/').next()?;
    if seg.len() >= 2
        && seg.as_bytes()[0] == b'v'
        && seg[1..].chars().all(|c| c.is_ascii_digit())
    {
        Some(seg)
    } else {
        None
    }
}

/// Strip `prefix` (e.g. `/v1`) from the start of `path_and_query`, returning
/// the remainder. Only matches at a complete path-segment boundary: the char
/// after the prefix must be `/`, `?`, or end-of-string. Returns `None` when
/// the prefix does not match cleanly.
fn strip_path_prefix<'a>(path_and_query: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path_and_query.strip_prefix(prefix)?;
    match rest.chars().next() {
        Some('/') | Some('?') | None => Some(rest),
        _ => None,
    }
}
