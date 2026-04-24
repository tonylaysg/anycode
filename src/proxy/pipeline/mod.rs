//! 7-stage linear pipeline for proxy request processing.
//!
//! The pipeline processes each request through explicit linear stages:
//! extract → routing → thinking → transform → headers → forward → response.

use axum::body::Body;
use axum::http::{Request, Response};
use std::sync::Arc;

use crate::backend::{BackendState, AgentRegistry};
use crate::metrics::{BackendOverride, DebugLogger, ObservabilityHub, RequestSpan};
use crate::proxy::thinking::TransformerRegistry;

mod extract;
mod forward;
mod headers;
mod response;
mod routing;
mod thinking;
mod transform;

pub use extract::extract_request;
pub use forward::forward_with_retry;
pub use headers::build_headers;
pub use response::handle_response;
pub use routing::{extract_ac_marker, resolve_backend};
pub use thinking::create_thinking;
pub use transform::transform_body;

/// Context shared across pipeline stages.
///
/// Contains observability and debugging context that is needed
/// throughout the request lifecycle, but NOT the parsed body
/// (which is passed explicitly between stages).
#[derive(Clone)]
pub struct PipelineContext {
    /// The request span for observability
    pub span: RequestSpan,
    /// Observability hub for metrics
    pub observability: ObservabilityHub,
    /// Debug logger for auxiliary logging
    pub debug_logger: Arc<DebugLogger>,
    /// Whether the observability span has been finalized
    /// (finish_request or finish_error already called by a late stage).
    pub(crate) span_finalized: bool,
}

impl PipelineContext {
    pub fn new(span: RequestSpan, observability: ObservabilityHub, debug_logger: Arc<DebugLogger>) -> Self {
        Self {
            span,
            observability,
            debug_logger,
            span_finalized: false,
        }
    }
}

/// Configuration for pipeline execution.
#[derive(Clone)]
pub struct PipelineConfig {
    /// Backend state for resolving backends
    pub backend_state: BackendState,
    /// Subagent registry for session affinity lookups
    pub agent_registry: AgentRegistry,
    /// Transformer registry for thinking session management
    pub transformer_registry: Arc<TransformerRegistry>,
    /// Request timeout configuration
    pub timeout_config: crate::proxy::timeout::TimeoutConfig,
    /// Pool configuration for retries
    pub pool_config: crate::proxy::pool::PoolConfig,
    /// HTTP client for upstream requests
    pub http_client: reqwest::Client,
}

impl PipelineConfig {
    pub fn new(
        backend_state: BackendState,
        agent_registry: AgentRegistry,
        transformer_registry: Arc<TransformerRegistry>,
        timeout_config: crate::proxy::timeout::TimeoutConfig,
        pool_config: crate::proxy::pool::PoolConfig,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .connect_timeout(timeout_config.connect)
            .pool_idle_timeout(Some(pool_config.pool_idle_timeout))
            .pool_max_idle_per_host(pool_config.pool_max_idle_per_host)
            .build()
            .expect("Failed to build upstream client");

        Self {
            backend_state,
            agent_registry,
            transformer_registry,
            timeout_config,
            pool_config,
            http_client,
        }
    }
}

/// Execute the 7-stage pipeline for a single request.
///
/// This is the main entry point for the unified pipeline. It orchestrates
/// all 7 stages in sequence and handles error propagation.
///
/// Observability lifecycle: stages 6-7 call `finish_error`/`finish_request`
/// internally for late errors. For early errors (stages 1-5), this function
/// ensures `finish_error` is called before returning.
pub async fn execute_pipeline(
    req: Request<Body>,
    config: &PipelineConfig,
    ctx: &mut PipelineContext,
    backend_override: Option<String>,
    plugin_override: Option<BackendOverride>,
) -> Result<Response<Body>, crate::proxy::error::ProxyError> {
    let is_teammate = backend_override.is_some();

    match execute_pipeline_inner(req, config, ctx, backend_override, plugin_override, is_teammate).await {
        Ok(response) => Ok(response),
        Err(e) => {
            // Late stages (forward, response) set span_finalized=true when they
            // call finish_error/finish_request. For early errors (stages 1-5),
            // finalize the span here to avoid dangling spans.
            if !ctx.span_finalized {
                ctx.observability.finish_error(ctx.span.clone(), Some(e.status_code().as_u16()));
                ctx.span_finalized = true;
            }
            Err(e)
        }
    }
}

async fn execute_pipeline_inner(
    req: Request<Body>,
    config: &PipelineConfig,
    ctx: &mut PipelineContext,
    backend_override: Option<String>,
    plugin_override: Option<BackendOverride>,
    is_teammate: bool,
) -> Result<Response<Body>, crate::proxy::error::ProxyError> {
    // Stage 1: Extract request
    let extracted = extract::extract_request(req, ctx).await?;

    // Stage 2: Resolve backend
    let backend = routing::resolve_backend(
        &config.backend_state,
        backend_override,
        plugin_override,
        extracted.parsed_body.as_ref(),
        &config.agent_registry,
        ctx,
    )?;

    // Stage 3: Create thinking session (after routing, before transform)
    // Teammate requests (those with backend_override) skip thinking.
    let thinking_session = if is_teammate {
        None
    } else {
        thinking::create_thinking(
            &config.transformer_registry,
            &backend,
            ctx,
        )
    };

    // Stage 4: Transform body
    let (transformed_body, is_streaming, model_mapping) = transform::transform_body(
        extracted.body_bytes,
        extracted.parsed_body,
        &backend,
        thinking_session.as_ref(),
        ctx,
    )?;

    // Update span with request bytes after transformation
    ctx.span.set_request_bytes(transformed_body.len());

    // Stage 5: Build headers
    let headers = headers::build_headers(
        &extracted.headers,
        &backend,
        ctx,
    )?;

    // Stage 6: Forward with retry
    let upstream_resp = forward::forward_with_retry(
        &config.http_client,
        extracted.method.clone(),
        extracted.uri.clone(),
        headers.clone(),
        transformed_body.clone(),
        is_streaming,
        &backend,
        config,
        ctx,
    ).await?;

    // Stage 6.5: Auto-retry on invalid_reasoning_effort (non-streaming only).
    // When the backend rejects the effort level, parse its supported values,
    // cap the effort to the highest supported one, and retry once.
    let upstream_resp = if !is_streaming && upstream_resp.status().as_u16() == 400 {
        effort_auto_retry(
            upstream_resp,
            &config.http_client,
            extracted.method,
            extracted.uri,
            headers,
            transformed_body,
            &backend,
            config,
            ctx,
        ).await
    } else {
        upstream_resp
    };

    // Stage 7: Handle response
    let response = response::handle_response(
        upstream_resp,
        backend,
        thinking_session,
        model_mapping,
        config,
        ctx,
    ).await?;

    Ok(response)
}

/// Check a 400 response for `invalid_reasoning_effort` and retry with the
/// highest effort value the backend actually supports.
///
/// Returns `Some(response)` with the retried response (or the original 400 if
/// the error is unrelated), `None` if parsing or retry fails.
/// When a 400 response contains `invalid_reasoning_effort`, parse the backend's
/// supported effort values, cap to the highest one, and retry the request.
/// For any other 400, return the original response unchanged (reconstructed from body).
async fn effort_auto_retry(
    resp: reqwest::Response,
    client: &reqwest::Client,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: Vec<(String, String)>,
    body_bytes: Vec<u8>,
    backend: &crate::config::Backend,
    config: &PipelineConfig,
    _ctx: &mut PipelineContext,
) -> reqwest::Response {
    // Read the 400 body — we consume the response here
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(_) => return rebuild_response(status, resp_headers, String::new()),
    };

    // Only handle invalid_reasoning_effort
    if !body_text.contains("invalid_reasoning_effort") {
        return rebuild_response(status, resp_headers, body_text);
    }

    // Parse highest supported effort from error message
    let max_supported = match parse_max_supported_effort(&body_text) {
        Some(m) => m,
        None => return rebuild_response(status, resp_headers, body_text),
    };

    crate::metrics::app_log(
        "effort_cap",
        &format!("Auto-capped effort '{}' -> '{}' for backend '{}'",
            extract_requested_effort(&body_text).unwrap_or("?"), max_supported, backend.name),
    );

    // Rewrite body: set output_config.effort to max_supported
    let new_body = match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        Ok(mut json) => {
            json["output_config"]["effort"] = serde_json::json!(max_supported);
            match serde_json::to_vec(&json) {
                Ok(b) => b,
                Err(_) => return rebuild_response(status, resp_headers, body_text),
            }
        }
        Err(_) => return rebuild_response(status, resp_headers, body_text),
    };

    // Retry with corrected body
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let upstream_uri = format!("{}{}", backend.base_url, path_and_query);
    let mut builder = client.request(method, &upstream_uri);
    for (name, value) in &headers {
        builder = builder.header(name, value);
    }
    builder = builder.timeout(config.timeout_config.request);

    match builder.body(new_body).send().await {
        Ok(r) => r,
        Err(_) => rebuild_response(status, resp_headers, body_text),
    }
}

/// Reconstruct a reqwest::Response from status + body text.
/// Used to "put back" the 400 when the error isn't effort-related.
fn rebuild_response(
    status: reqwest::StatusCode,
    _headers: reqwest::header::HeaderMap,
    body: String,
) -> reqwest::Response {
    // Build via http crate which reqwest re-exports
    let http_resp = axum::http::Response::builder()
        .status(status.as_u16())
        .body(body.into_bytes())
        .unwrap_or_default();
    reqwest::Response::from(http_resp)
}

/// Extract the requested effort from the error message for logging.
fn extract_requested_effort(body: &str) -> Option<&str> {
    // Format: `output_config.effort "xhigh" is not supported`
    let after_quote = body.find('"')? + 1;
    let end = body[after_quote..].find('"')? + after_quote;
    Some(&body[after_quote..end])
}

/// Parse the highest effort level from an `invalid_reasoning_effort` error message.
///
/// Handles formats like:
///   `"supported values: [medium]"`
///   `"supported values: [low, medium, high]"`
fn parse_max_supported_effort(body: &str) -> Option<String> {
    let start = body.find('[')? + 1;
    let end = body[start..].find(']')? + start;
    let list = &body[start..end];

    const RANKS: &[&str] = &["low", "medium", "high", "xhigh"];

    let max = list
        .split(',')
        .map(|s| s.trim().trim_matches('"'))
        .filter_map(|s| RANKS.iter().position(|r| *r == s).map(|rank| (rank, s)))
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, s)| s.to_string())?;

    Some(max)
}
