//! Smart upstream URL builder.
//!
//! Copilot CLI always emits paths prefixed with `/v1` (`/v1/messages`,
//! `/v1/chat/completions`, `/v1/responses`, `/v1/models`). Backends, however,
//! are inconsistent about whether `/v1` belongs in the base URL:
//!
//! * `https://api.anthropic.com`         → expects `/v1/messages`
//! * `https://api.deepseek.com/anthropic`→ expects `/v1/messages`
//! * `https://api.openai.com/v1`         → expects `/chat/completions` (no leading /v1)
//! * `https://api.deepseek.com/v1`       → expects `/chat/completions`
//!
//! Naïve concatenation (`base_url + path`) doubles `/v1` when the base already
//! contains it. This helper performs version-prefix deduplication: if the base
//! ends with `/vN` and the incoming path begins with `/vN/`, one copy is
//! stripped. The rule only fires on matching version numbers, so unrelated
//! paths (e.g. `/v1beta/...` vs `/v1/...`) are left alone.

/// Build an upstream URL from a backend base URL and a request path.
///
/// Strips a duplicate `/vN` segment when both `base_url` and `path_and_query`
/// carry the same version prefix.
pub fn build_upstream_url(base_url: &str, path_and_query: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if let Some(base_ver) = trailing_version_segment(base) {
        if let Some(rest) = strip_leading_version(path_and_query, base_ver) {
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

fn strip_leading_version<'a>(path_and_query: &'a str, version: &str) -> Option<&'a str> {
    let rest = path_and_query.strip_prefix('/')?.strip_prefix(version)?;
    match rest.chars().next() {
        Some('/') | Some('?') | None => Some(rest),
        _ => None,
    }
}
