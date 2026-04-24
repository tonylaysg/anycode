//! Reverse model mapping for proxy responses.
//!
//! When the proxy rewrites model names in requests (forward mapping via
//! `Backend::resolve_model()`), the response must have model names mapped
//! back to what the client originally sent (reverse mapping).
//!
//! Two paths are handled:
//! - **SSE streaming**: a stateful `ChunkRewriter` closure transforms the
//!   `message_start` event's `message.model` field in the first chunk
//! - **Non-streaming JSON**: the top-level `$.model` field is rewritten

use axum::body::Bytes;
use crate::metrics::ChunkRewriter;

/// Forward and reverse model mapping pair.
#[derive(Debug, Clone)]
pub struct ModelMapping {
    /// Model name sent to the backend (e.g., "glm-5").
    pub backend: String,
    /// Original model name from the client (e.g., "claude-opus-4-6").
    pub original: String,
}

/// Create a stateful chunk rewriter that replaces `message.model` in the
/// `message_start` SSE event back to the original model name.
///
/// # Lifecycle
///
/// ```text
/// [Waiting] --chunk without message_start--> [Waiting] (pass through)
/// [Waiting] --chunk with message_start-----> [Done]    (rewrite model)
/// [Done]    --any chunk--------------------> [Done]    (pass through)
/// ```
///
/// After the first chunk containing `message_start` is processed, the rewriter
/// becomes a zero-cost no-op for all subsequent chunks.
/// Create a stateful chunk rewriter that replaces `model` in streaming
/// responses back to the original model name.
///
/// Handles two wire formats:
///
/// * **Anthropic** — model appears once, in the `message_start` event.
///   After rewriting it, the rewriter becomes a zero-cost no-op.
/// * **OpenAI** (`chat.completion.chunk`) — model is echoed in every chunk.
///   The rewriter keeps scanning each chunk until the stream ends, but the
///   per-chunk fast path skips chunks that don't carry a `model` key at all.
pub fn make_reverse_model_rewriter(mapping: ModelMapping) -> ChunkRewriter {
    let mut anthropic_done = false;
    Box::new(move |bytes: Bytes| {
        let haystack = bytes.as_ref();
        let has_anthropic_marker = !anthropic_done
            && contains_bytes(haystack, b"\"message_start\"");
        // OpenAI wire: every chunk carries "object":"chat.completion.*" plus a model field.
        let has_openai_marker = contains_bytes(haystack, b"chat.completion")
            && contains_bytes(haystack, b"\"model\"");
        if !has_anthropic_marker && !has_openai_marker {
            return bytes;
        }

        let text = String::from_utf8_lossy(&bytes);
        let mut result = String::with_capacity(text.len());
        let mut rewritten = false;

        for line in text.split('\n') {
            if !result.is_empty() {
                result.push('\n');
            }

            let trimmed = line.trim();
            let data_payload = trimmed
                .strip_prefix("data:")
                .map(|rest| rest.trim_start());

            if let Some(payload) = data_payload {
                if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(payload) {
                    let is_msg_start = json
                        .get("type")
                        .and_then(|t| t.as_str())
                        == Some("message_start");
                    let is_openai_chunk = json
                        .get("object")
                        .and_then(|o| o.as_str())
                        .is_some_and(|s| s.starts_with("chat.completion"));

                    let mut did_rewrite_here = false;
                    if is_msg_start {
                        if let Some(msg) = json.get_mut("message") {
                            if let Some(model) = msg.get("model").and_then(|m| m.as_str()) {
                                if model == mapping.backend {
                                    msg["model"] = serde_json::json!(&mapping.original);
                                    did_rewrite_here = true;
                                } else {
                                    crate::metrics::app_log(
                                        "model_map",
                                        &format!(
                                            "Reverse mapping skipped: expected '{}' but found '{}'",
                                            mapping.backend, model
                                        ),
                                    );
                                }
                            }
                        }
                        anthropic_done = true;
                    } else if is_openai_chunk {
                        if let Some(model) = json.get("model").and_then(|m| m.as_str()) {
                            if model == mapping.backend {
                                json["model"] = serde_json::json!(&mapping.original);
                                did_rewrite_here = true;
                            }
                        }
                    }

                    if is_msg_start || is_openai_chunk {
                        rewritten |= did_rewrite_here;
                        result.push_str("data: ");
                        result.push_str(
                            &serde_json::to_string(&json)
                                .unwrap_or_else(|_| payload.to_string()),
                        );
                        continue;
                    }
                }
            }
            result.push_str(line);
        }

        if rewritten {
            Bytes::from(result.into_bytes())
        } else {
            bytes
        }
    })
}

/// Rewrite `$.model` in a non-streaming JSON response body.
pub fn reverse_model_in_response(
    body_bytes: &Bytes,
    mapping: &ModelMapping,
) -> Bytes {
    let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(body_bytes) else {
        return body_bytes.clone();
    };
    match json.get("model").and_then(|m| m.as_str()) {
        Some(model) if model == mapping.backend => {}
        Some(model) => {
            crate::metrics::app_log(
                "model_map",
                &format!(
                    "Reverse mapping skipped in response: expected '{}' but found '{}'",
                    mapping.backend, model
                ),
            );
            return body_bytes.clone();
        }
        None => return body_bytes.clone(),
    }
    json["model"] = serde_json::json!(&mapping.original);
    match serde_json::to_vec(&json) {
        Ok(bytes) => {
            crate::metrics::app_log(
                "model_map",
                &format!(
                    "Reverse mapped model in response: '{}' → '{}'",
                    mapping.backend, mapping.original
                ),
            );
            Bytes::from(bytes)
        }
        Err(_) => body_bytes.clone(),
    }
}

/// Check if `haystack` contains `needle` as a contiguous subsequence.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
