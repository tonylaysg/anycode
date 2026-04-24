//! Stage 4: Transform request body.
//!
//! Applies transformations to the request body:
//! - Model rewriting (family-based mapping)
//! - Thinking compatibility conversion (adaptive -> enabled)
//! - Thinking block filtering (via ThinkingSession)

use serde_json::Value;

use crate::config::Backend;
use crate::proxy::error::ProxyError;
use crate::proxy::model_rewrite::ModelMapping;
use crate::proxy::thinking::ThinkingSession;
use crate::proxy::pipeline::PipelineContext;

/// Result of body transformation.
/// Stage 4: Transform request body.
///
/// Applies all body transformations and returns the transformed bytes
/// along with metadata about the transformation.
pub fn transform_body(
    body_bytes: Vec<u8>,
    parsed_body: Option<Value>,
    backend: &Backend,
    thinking: Option<&ThinkingSession>,
    ctx: &mut PipelineContext,
) -> Result<(Vec<u8>, bool, Option<ModelMapping>, bool), ProxyError> {
    let needs_thinking_compat = backend.needs_thinking_compat();
    let mut model_mapping: Option<ModelMapping> = None;

    // If no JSON body, return as-is
    let Some(mut json_body) = parsed_body else {
        return Ok((body_bytes, false, None, false));
    };

    // Detect streaming from body
    let is_streaming_request = json_body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Track if any transformation occurred
    let mut model_rewritten = false;
    let mut thinking_converted = false;
    let mut filtered_count = 0u32;
    let mut effort_capped = false;

    // Remember original model name for per-model effort cap (before rewriting)
    let original_model = json_body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    // 1. Rewrite model field via family-based mapping
    if let Some(model_val) = json_body.get("model").and_then(|m| m.as_str()) {
        if let Some(new_model) = backend.resolve_model(model_val) {
            ctx.debug_logger.log_auxiliary(
                "model_map",
                None,
                None,
                Some(&format!("Rewrote model '{}' -> '{}'", model_val, new_model)),
                None,
            );
            model_mapping = Some(ModelMapping {
                backend: new_model.to_string(),
                original: model_val.to_string(),
            });
            json_body["model"] = serde_json::json!(new_model);
            model_rewritten = true;
        }
    }

    // 2. Convert adaptive thinking to standard format for non-Anthropic backends
    if needs_thinking_compat {
        if let Some(changed) = convert_adaptive_thinking(&mut json_body, backend.thinking_budget_tokens) {
            if changed {
                thinking_converted = true;
                let budget = json_body
                    .get("thinking")
                    .and_then(|t| t.get("budget_tokens"))
                    .and_then(|b| b.as_u64())
                    .unwrap_or(0);
                ctx.debug_logger.log_auxiliary(
                    "thinking_compat",
                    None,
                    None,
                    Some(&format!(
                        "Converted adaptive -> enabled for backend '{}', budget={}",
                        backend.name, budget
                    )),
                    None,
                );
            }
        }
    }

    // 3. Filter thinking blocks (main agent only - ThinkingSession present)
    if let Some(session) = thinking {
        filtered_count = session.filter(&mut json_body);

        // Strip the top-level `thinking` parameter when thinking blocks are absent
        // from a multi-turn conversation. Without this, Anthropic-compatible backends
        // (e.g. DeepSeek) return 400: "content[].thinking must be passed back to the API".
        //
        // Guard: only strip when assistant messages exist. On the very first turn there
        // are no assistant messages yet, so `thinking` must be kept for the backend to
        // start generating thinking blocks.
        if !has_remaining_thinking_blocks(&json_body) && has_assistant_messages(&json_body) {
            if json_body.as_object_mut().is_some_and(|obj| obj.remove("thinking").is_some()) {
                thinking_converted = false;
                ctx.debug_logger.log_auxiliary(
                    "thinking_filter",
                    None,
                    None,
                    Some(&format!(
                        "Removed 'thinking' field: {} block(s) stripped, no thinking blocks \
                         remain in conversation (backend '{}')",
                        filtered_count, backend.name
                    )),
                    None,
                );
            }
        }
    }

    // Whether thinking is still active in the final body (used by Stage 5 headers).
    // Ground truth: check the actual body state after all transforms.
    let thinking_active = json_body.get("thinking").is_some();

    // 4. Cap output_config.effort per model family
    if let Some(output_config) = json_body.get("output_config").and_then(|v| v.as_object()) {
        if let Some(effort_str) = output_config.get("effort").and_then(|v| v.as_str()) {
            if let Some(capped) = backend.cap_effort(effort_str, &original_model) {
                ctx.debug_logger.log_auxiliary(
                    "effort_cap",
                    None,
                    None,
                    Some(&format!(
                        "Capped output_config.effort '{}' -> '{}' for backend '{}'",
                        effort_str, capped, backend.name
                    )),
                    None,
                );
                let capped_owned = capped.to_string();
                json_body["output_config"]["effort"] = serde_json::json!(capped_owned);
                effort_capped = true;
            }
        }
    }

    // Re-serialize body if any transformation occurred.
    // `thinking_active` is false when we stripped the `thinking` field — that counts as a change.
    if model_rewritten || thinking_converted || filtered_count > 0 || effort_capped
        || !thinking_active
    {
        if thinking_converted {
            let thinking_json = json_body
                .get("thinking")
                .map(|t| t.to_string())
                .unwrap_or_else(|| "null".to_string());
            ctx.debug_logger.log_auxiliary(
                "thinking_compat",
                None,
                None,
                Some(&format!("Final request thinking field: {}", thinking_json)),
                None,
            );
        }

        match serde_json::to_vec(&json_body) {
            Ok(updated) => Ok((updated, is_streaming_request, model_mapping, thinking_active)),
            Err(e) => {
                crate::metrics::app_log_error(
                    "upstream",
                    "Failed to serialize transformed request body, using original",
                    &e.to_string(),
                );
                Ok((body_bytes, is_streaming_request, model_mapping, thinking_active))
            }
        }
    } else {
        Ok((body_bytes, is_streaming_request, model_mapping, thinking_active))
    }
}

/// Returns `true` if the request body still contains thinking or redacted_thinking blocks.
///
/// Used after `ThinkingSession::filter` to decide whether the top-level
/// `thinking` parameter should be stripped (when all blocks were removed).
fn has_remaining_thinking_blocks(body: &Value) -> bool {
    let Some(messages) = body.get("messages").and_then(|v| v.as_array()) else {
        return false;
    };
    for message in messages {
        let Some(content) = message.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for item in content {
            let item_type = item.get("type").and_then(|t| t.as_str());
            if matches!(item_type, Some("thinking") | Some("redacted_thinking")) {
                return true;
            }
        }
    }
    false
}

/// Returns `true` if the request body contains any assistant-role messages.
///
/// Used to distinguish a first-turn request (no assistant messages yet, `thinking`
/// must be kept so the backend can start generating thinking blocks) from a
/// multi-turn resumed session where assistant messages are present but thinking
/// blocks are absent (inconsistent state that causes DeepSeek-style 400 errors).
fn has_assistant_messages(body: &Value) -> bool {
    let Some(messages) = body.get("messages").and_then(|v| v.as_array()) else {
        return false;
    };
    messages
        .iter()
        .any(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
}

/// Convert `"thinking": {"type": "adaptive"}` to `"thinking": {"type": "enabled", "budget_tokens": N}`.
///
/// Budget priority: explicit config (`thinking_budget_tokens`) > `max_tokens - 1` from request > default 10000.
///
/// Returns `Some(true)` if converted, `Some(false)` if thinking exists but not adaptive,
/// `None` if no thinking field present.
fn convert_adaptive_thinking(body: &mut Value, configured_budget: Option<u32>) -> Option<bool> {
    let is_adaptive = body
        .get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(|t| t.as_str())
        == Some("adaptive");

    if !is_adaptive {
        return body.get("thinking").map(|_| false);
    }

    let budget = configured_budget.unwrap_or_else(|| {
        body.get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|mt| mt.saturating_sub(1) as u32)
            .unwrap_or(10_000)
    });

    body.as_object_mut()?.insert(
        "thinking".to_string(),
        serde_json::json!({
            "type": "enabled",
            "budget_tokens": budget
        }),
    );
    Some(true)
}
