//! `GET /v1/models` proxy handler — fetches the active backend's model list,
//! probes `/v1/models` then `/models` if the backend didn't configure
//! `models_path` explicitly, and caches the result for 30 minutes.
//!
//! ## Why a dedicated handler?
//!
//! Most Anthropic-API-compatible backends expose `/v1/models`, but a large
//! fraction of OpenAI-compatible gateways (including several popular LLM
//! routers — "openrouter.ai", on-prem vLLM / LiteLLM instances, etc.) expose
//! the list at plain `/models`. Copilot CLI in BYOK mode always asks the
//! proxy for `/v1/models`, so the proxy must do the translation.
//!
//! The fallback-based probing is done once per (backend.name, base_url) tuple
//! and cached; when a backend switches active or its base_url changes the
//! cache entry is recomputed on next access.
//!
//! ## Cache policy
//!
//! * TTL = 30 minutes, matching Copilot CLI's internal `LIST_MODELS_CACHE_TTL_MS`.
//! * Keyed by `(backend.name, backend.base_url)` so a backend edit invalidates
//!   the old entry naturally.
//! * On backend error we return the upstream status as-is but do **not** cache
//!   it — a transient 5xx shouldn't poison the next 30 minutes.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::config::{build_auth_header, Backend};
use crate::proxy::router::RouterEngine;

const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const PROBE_PATHS: &[&str] = &["/v1/models", "/models"];

#[derive(Clone)]
struct CacheEntry {
    body: Vec<u8>,
    content_type: String,
    fetched_at: Instant,
    resolved_path: String,
}

type CacheKey = (String, String); // (backend.name, backend.base_url)

fn cache() -> &'static RwLock<HashMap<CacheKey, CacheEntry>> {
    static CACHE: OnceLock<RwLock<HashMap<CacheKey, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Clear all cached models responses. Called when config is reloaded or
/// when the active backend is switched.
pub fn invalidate_cache() {
    cache().write().clear();
}

/// Handler for `GET /v1/models` (and `GET /models`, which Copilot CLI
/// historically used for GitHub-native routing — still accepted here to
/// stay robust across Copilot CLI versions).
pub async fn handle_list_models(State(state): State<RouterEngine>) -> Response<Body> {
    let backend = match state.backend_state.get_active_backend_config() {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("No active backend: {}", e),
            );
        }
    };

    if !backend.is_configured() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("Backend '{}' is missing an API key", backend.name),
        );
    }

    let cache_key = (backend.name.clone(), backend.base_url.clone());
    if let Some(entry) = get_cached(&cache_key) {
        return build_response(&entry);
    }

    match fetch_models(&backend).await {
        Ok(entry) => {
            cache().write().insert(cache_key, entry.clone());
            build_response(&entry)
        }
        Err((status, msg)) => error_response(status, &msg),
    }
}

fn get_cached(key: &CacheKey) -> Option<CacheEntry> {
    let cache = cache().read();
    cache
        .get(key)
        .filter(|e| e.fetched_at.elapsed() < CACHE_TTL)
        .cloned()
}

async fn fetch_models(backend: &Backend) -> Result<CacheEntry, (StatusCode, String)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("http client init failed: {}", e),
            )
        })?;

    let candidate_paths: Vec<String> = match &backend.models_path {
        Some(configured) => vec![configured.clone()],
        None => PROBE_PATHS.iter().map(|s| s.to_string()).collect(),
    };

    let mut last_status: Option<(StatusCode, String)> = None;
    for path in &candidate_paths {
        let url = format!("{}{}", backend.base_url.trim_end_matches('/'), path);
        let mut req = client.get(&url);

        if let Some((name, value)) = build_auth_header(backend) {
            req = req.header(name, value);
        }
        req = req.header("accept", "application/json");

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                crate::metrics::app_log(
                    "models",
                    &format!("Probe failed backend='{}' url='{}' err={}", backend.name, url, e),
                );
                last_status = Some((
                    StatusCode::BAD_GATEWAY,
                    format!("upstream error for {}: {}", path, e),
                ));
                continue;
            }
        };

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            crate::metrics::app_log(
                "models",
                &format!("Probe 404 backend='{}' path='{}', trying next", backend.name, path),
            );
            last_status = Some((
                StatusCode::NOT_FOUND,
                format!("no models endpoint at {}", path),
            ));
            continue;
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_string();

        let body = resp.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("failed to read body from {}: {}", path, e),
            )
        })?;

        if !status.is_success() {
            return Err((
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                String::from_utf8_lossy(&body).into_owned(),
            ));
        }

        crate::metrics::app_log(
            "models",
            &format!(
                "Cached models list backend='{}' resolved_path='{}' bytes={}",
                backend.name,
                path,
                body.len()
            ),
        );
        return Ok(CacheEntry {
            body: body.to_vec(),
            content_type,
            fetched_at: Instant::now(),
            resolved_path: path.clone(),
        });
    }

    Err(last_status.unwrap_or((
        StatusCode::BAD_GATEWAY,
        "no models endpoint responded".to_string(),
    )))
}

fn build_response(entry: &CacheEntry) -> Response<Body> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", entry.content_type.parse().unwrap());
    headers.insert("x-anycode-models-source", entry.resolved_path.parse().unwrap());
    headers.insert(
        "x-anycode-models-age-secs",
        entry.fetched_at.elapsed().as_secs().to_string().parse().unwrap(),
    );

    let mut resp = Response::new(Body::from(entry.body.clone()));
    *resp.headers_mut() = headers;
    resp
}

fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    let body = format!(r#"{{"error":{{"message":{:?}}}}}"#, message);
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
