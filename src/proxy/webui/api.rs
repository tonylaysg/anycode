//! WebUI REST API handlers for configuration management.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::backend::BackendState;
use crate::config::{save_config, AgentsConfig, BackendPricing, Config, ConfigStore, Defaults};

/// Shared state for WebUI handlers.
#[derive(Clone)]
pub struct WebuiState {
    pub config_store: ConfigStore,
    pub backend_state: BackendState,
}

// ── DTOs ─────────────────────────────────────────────────────────────────────

/// Serialized backend with api_key masked for safe transport over HTTP.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackendDto {
    pub name: String,
    pub display_name: String,
    pub base_url: String,
    pub auth_type: String,
    /// True when an api_key is configured (never exposes the actual value).
    #[serde(default)]
    pub api_key_set: bool,
    /// New api_key to set. `None` = keep existing, `Some("")` = clear.
    #[serde(default)]
    pub api_key_input: Option<String>,
    #[serde(default)]
    pub thinking_compat: Option<bool>,
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
    #[serde(default)]
    pub model_opus: Option<String>,
    #[serde(default)]
    pub model_sonnet: Option<String>,
    #[serde(default)]
    pub model_haiku: Option<String>,
    #[serde(default)]
    pub pricing: Option<BackendPricing>,
}

/// Full config DTO returned to the browser — api_keys are masked.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigDto {
    pub defaults: Defaults,
    #[serde(default)]
    pub backends: Vec<BackendDto>,
    #[serde(default)]
    pub agents: Option<AgentsConfig>,
}

// ── Conversions ───────────────────────────────────────────────────────────────

fn config_to_dto(config: &Config) -> ConfigDto {
    ConfigDto {
        defaults: config.defaults.clone(),
        backends: config.backends.iter().map(|b| BackendDto {
            name: b.name.clone(),
            display_name: b.display_name.clone(),
            base_url: b.base_url.clone(),
            auth_type: b.auth_type_str.clone(),
            api_key_set: b.api_key.as_ref().is_some_and(|k| !k.is_empty()),
            api_key_input: None,
            thinking_compat: b.thinking_compat,
            thinking_budget_tokens: b.thinking_budget_tokens,
            model_opus: b.model_opus.clone(),
            model_sonnet: b.model_sonnet.clone(),
            model_haiku: b.model_haiku.clone(),
            pricing: b.pricing.clone(),
        }).collect(),
        agents: config.agents.clone(),
    }
}

/// Merge a `BackendDto` from the browser back into a real `Backend`.
///
/// If `api_key_input` is `None` (not provided), the existing key in
/// `existing_key` is preserved, so callers must supply the old value.
fn dto_to_backend(
    dto: &BackendDto,
    existing_key: Option<String>,
) -> crate::config::Backend {
    let api_key = match &dto.api_key_input {
        Some(k) if !k.is_empty() => Some(k.clone()),
        Some(_) => None,   // empty string → clear
        None => existing_key, // not provided → keep existing
    };

    crate::config::Backend {
        name: dto.name.clone(),
        display_name: dto.display_name.clone(),
        base_url: dto.base_url.clone(),
        auth_type_str: dto.auth_type.clone(),
        api_key,
        thinking_compat: dto.thinking_compat,
        thinking_budget_tokens: dto.thinking_budget_tokens,
        model_opus: dto.model_opus.clone(),
        model_sonnet: dto.model_sonnet.clone(),
        model_haiku: dto.model_haiku.clone(),
        pricing: dto.pricing.clone(),
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/config — return current config with api_keys masked.
pub async fn get_config(State(state): State<WebuiState>) -> impl IntoResponse {
    let config = state.config_store.get();
    Json(config_to_dto(&config))
}

/// PUT /api/config — replace config, persist to file, hot-reload runtime state.
pub async fn put_config(
    State(state): State<WebuiState>,
    Json(dto): Json<ConfigDto>,
) -> Response {
    let existing = state.config_store.get();

    // Rebuild backends, preserving existing api_keys when not provided.
    let backends: Vec<crate::config::Backend> = dto.backends.iter().map(|d| {
        let old_key = existing.backends.iter()
            .find(|b| b.name == d.name)
            .and_then(|b| b.api_key.clone());
        dto_to_backend(d, old_key)
    }).collect();

    let new_config = Config {
        defaults: dto.defaults,
        proxy: existing.proxy.clone(),
        webui: existing.webui.clone(),
        terminal: existing.terminal.clone(),
        debug_logging: existing.debug_logging.clone(),
        claude_settings: existing.claude_settings.clone(),
        backends,
        agents: dto.agents,
    };

    if let Err(e) = new_config.validate() {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let path = state.config_store.path().to_path_buf();
    if let Err(e) = save_config(&path, &new_config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    // Reload ConfigStore so in-process reads see the new config.
    let _ = state.config_store.reload();

    // Hot-update BackendState so active routing reflects the new config.
    if let Err(e) = state.backend_state.update_config(new_config.clone()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Config saved but runtime update failed: {e}")).into_response();
    }

    Json(config_to_dto(&new_config)).into_response()
}

// ── Active backend ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ActiveBackendRequest {
    pub name: String,
}

/// POST /api/config/active — hot-switch the active backend at runtime.
pub async fn post_active_backend(
    State(state): State<WebuiState>,
    Json(req): Json<ActiveBackendRequest>,
) -> Response {
    if let Err(e) = state.backend_state.switch_backend(&req.name) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    StatusCode::OK.into_response()
}

/// GET /api/config/backends/:name — return a single backend (api_key masked).
pub async fn get_backend(
    State(state): State<WebuiState>,
    Path(name): Path<String>,
) -> Response {
    let config = state.config_store.get();
    match config.backends.iter().find(|b| b.name == name) {
        Some(b) => {
            let dto = BackendDto {
                name: b.name.clone(),
                display_name: b.display_name.clone(),
                base_url: b.base_url.clone(),
                auth_type: b.auth_type_str.clone(),
                api_key_set: b.api_key.as_ref().is_some_and(|k| !k.is_empty()),
                api_key_input: None,
                thinking_compat: b.thinking_compat,
                thinking_budget_tokens: b.thinking_budget_tokens,
                model_opus: b.model_opus.clone(),
                model_sonnet: b.model_sonnet.clone(),
                model_haiku: b.model_haiku.clone(),
                pricing: b.pricing.clone(),
            };
            Json(dto).into_response()
        }
        None => (StatusCode::NOT_FOUND, format!("Backend '{}' not found", name)).into_response(),
    }
}
