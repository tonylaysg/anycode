//! WebUI REST API handlers for configuration management.
//!
//! Dual-profile aware: endpoints accept an optional `?profile=claude|copilot`
//! query parameter. When omitted, the running instance's mode (`cli_mode`) is
//! used. Hot-updates to `BackendState` only apply when the edited profile
//! matches the running instance's mode; edits to the other profile are
//! persisted to disk but take effect on next start of that instance type.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::backend::BackendState;
use crate::cli_mode::CliMode;
use crate::config::{save_config, AgentsConfig, BackendPricing, CliProfile, ConfigStore, Defaults};

/// Shared state for WebUI handlers.
#[derive(Clone)]
pub struct WebuiState {
    pub config_store: ConfigStore,
    pub backend_state: BackendState,
    /// The CLI mode of the running instance that owns this WebUI.
    /// Used as the default profile when API requests omit `?profile=`.
    pub cli_mode: CliMode,
}

// ── Profile query parameter ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ProfileQuery {
    pub profile: Option<String>,
}

impl ProfileQuery {
    /// Resolve the profile name, defaulting to the running instance's cli_mode.
    fn resolve(&self, cli_mode: CliMode) -> Result<CliMode, String> {
        match self.profile.as_deref() {
            None | Some("") => Ok(cli_mode),
            Some("claude") => Ok(CliMode::Claude),
            Some("copilot") => Ok(CliMode::Copilot),
            Some(other) => Err(format!("Invalid profile '{}'. Must be 'claude' or 'copilot'.", other)),
        }
    }
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
    pub model_opus_max_effort: Option<String>,
    #[serde(default)]
    pub model_sonnet: Option<String>,
    #[serde(default)]
    pub model_sonnet_max_effort: Option<String>,
    #[serde(default)]
    pub model_haiku: Option<String>,
    #[serde(default)]
    pub model_haiku_max_effort: Option<String>,
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
    /// Which profile this config belongs to ("claude" or "copilot").
    /// Ignored on PUT (the URL query param is authoritative).
    #[serde(default)]
    pub profile: Option<String>,
}

/// Info about available profiles and the running instance's mode.
#[derive(Debug, Serialize)]
pub struct ProfilesInfo {
    /// Profile key of the running instance ("claude" or "copilot").
    pub current: String,
    /// All profile keys the WebUI can manage.
    pub available: Vec<String>,
}

// ── Conversions ───────────────────────────────────────────────────────────────

fn profile_to_dto(profile: &CliProfile, profile_key: &str) -> ConfigDto {
    ConfigDto {
        defaults: profile.defaults.clone(),
        backends: profile.backends.iter().map(|b| BackendDto {
            name: b.name.clone(),
            display_name: b.display_name.clone(),
            base_url: b.base_url.clone(),
            auth_type: b.auth_type_str.clone(),
            api_key_set: b.api_key.as_ref().is_some_and(|k| !k.is_empty()),
            api_key_input: None,
            thinking_compat: b.thinking_compat,
            thinking_budget_tokens: b.thinking_budget_tokens,
            model_opus: b.model_opus.clone(),
            model_opus_max_effort: b.model_opus_max_effort.clone(),
            model_sonnet: b.model_sonnet.clone(),
            model_sonnet_max_effort: b.model_sonnet_max_effort.clone(),
            model_haiku: b.model_haiku.clone(),
            model_haiku_max_effort: b.model_haiku_max_effort.clone(),
            pricing: b.pricing.clone(),
        }).collect(),
        agents: profile.agents.clone(),
        profile: Some(profile_key.to_string()),
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
        model_opus_max_effort: dto.model_opus_max_effort.clone(),
        model_sonnet: dto.model_sonnet.clone(),
        model_sonnet_max_effort: dto.model_sonnet_max_effort.clone(),
        model_haiku: dto.model_haiku.clone(),
        model_haiku_max_effort: dto.model_haiku_max_effort.clone(),
        pricing: dto.pricing.clone(),
    }
}

fn profile_key(mode: CliMode) -> &'static str {
    mode.profile_key()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/profiles — return available profiles and the running instance's mode.
pub async fn get_profiles(State(state): State<WebuiState>) -> impl IntoResponse {
    Json(ProfilesInfo {
        current: profile_key(state.cli_mode).to_string(),
        available: vec!["claude".to_string(), "copilot".to_string()],
    })
}

/// GET /api/config?profile=claude|copilot — return current config for a profile.
pub async fn get_config(
    State(state): State<WebuiState>,
    Query(q): Query<ProfileQuery>,
) -> Response {
    let target = match q.resolve(state.cli_mode) {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let config = state.config_store.get();
    Json(profile_to_dto(config.profile(target), profile_key(target))).into_response()
}

/// PUT /api/config?profile=claude|copilot — replace a profile's config.
///
/// Always persists to disk. Hot-updates the running BackendState only when
/// the target profile matches the running instance's cli_mode.
pub async fn put_config(
    State(state): State<WebuiState>,
    Query(q): Query<ProfileQuery>,
    Json(dto): Json<ConfigDto>,
) -> Response {
    let target = match q.resolve(state.cli_mode) {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    let existing = state.config_store.get();
    let existing_profile = existing.profile(target);

    // Rebuild backends, preserving existing api_keys when not provided.
    let backends: Vec<crate::config::Backend> = dto.backends.iter().map(|d| {
        let old_key = existing_profile.backends.iter()
            .find(|b| b.name == d.name)
            .and_then(|b| b.api_key.clone());
        dto_to_backend(d, old_key)
    }).collect();

    // Start from existing Config and replace only the target profile.
    let mut new_config = existing.clone();
    {
        let new_profile = new_config.profile_mut(target);
        new_profile.defaults = dto.defaults;
        new_profile.backends = backends;
        new_profile.agents = dto.agents;
        // claude_settings is preserved (not editable via this endpoint).

        // Self-heal: if the submitted `active` backend doesn't exist (e.g. the
        // copilot profile inherits the default active="claude" from Defaults,
        // but the user adds a backend named "openrouter" via the UI), adopt
        // the first backend as active so the config validates and saves.
        // Without this, the PUT returns 400 and the user's edits are lost.
        if !new_profile.backends.is_empty()
            && !new_profile
                .backends
                .iter()
                .any(|b| b.name == new_profile.defaults.active)
        {
            new_profile.defaults.active = new_profile.backends[0].name.clone();
        }
    }

    if let Err(e) = new_config.validate_for(target) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let path = state.config_store.path().to_path_buf();
    if let Err(e) = save_config(&path, &new_config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    // Reload ConfigStore so in-process reads see the new config.
    let _ = state.config_store.reload();

    // Hot-update BackendState only when the edited profile matches this instance.
    if target == state.cli_mode {
        let new_profile = new_config.profile(target).clone();
        if let Err(e) = state.backend_state.update_config(new_profile) {
            return (StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Config saved but runtime update failed: {e}")).into_response();
        }
    }

    Json(profile_to_dto(new_config.profile(target), profile_key(target))).into_response()
}

// ── Active backend ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ActiveBackendRequest {
    pub name: String,
}

/// POST /api/config/active?profile=... — hot-switch the active backend.
///
/// Hot-switching is only meaningful for the running instance's profile.
/// For the other profile, the change is persisted to disk only.
pub async fn post_active_backend(
    State(state): State<WebuiState>,
    Query(q): Query<ProfileQuery>,
    Json(req): Json<ActiveBackendRequest>,
) -> Response {
    let target = match q.resolve(state.cli_mode) {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    if target == state.cli_mode {
        // Hot-switch the running instance's active backend.
        if let Err(e) = state.backend_state.switch_backend(&req.name) {
            return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
    }

    // Persist to disk regardless of whether this is the running mode.
    let mut config = state.config_store.get();
    {
        let profile = config.profile_mut(target);
        if !profile.backends.iter().any(|b| b.name == req.name) {
            return (StatusCode::BAD_REQUEST,
                    format!("Backend '{}' not found in {} profile", req.name, profile_key(target))).into_response();
        }
        profile.defaults.active = req.name.clone();
    }
    let path = state.config_store.path().to_path_buf();
    if let Err(e) = save_config(&path, &config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let _ = state.config_store.reload();

    StatusCode::OK.into_response()
}

/// GET /api/config/backends/:name?profile=... — return a single backend (api_key masked).
pub async fn get_backend(
    State(state): State<WebuiState>,
    Query(q): Query<ProfileQuery>,
    Path(name): Path<String>,
) -> Response {
    let target = match q.resolve(state.cli_mode) {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let config = state.config_store.get();
    let profile = config.profile(target);
    match profile.backends.iter().find(|b| b.name == name) {
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
                model_opus_max_effort: b.model_opus_max_effort.clone(),
                model_sonnet: b.model_sonnet.clone(),
                model_sonnet_max_effort: b.model_sonnet_max_effort.clone(),
                model_haiku: b.model_haiku.clone(),
                model_haiku_max_effort: b.model_haiku_max_effort.clone(),
                pricing: b.pricing.clone(),
            };
            Json(dto).into_response()
        }
        None => (StatusCode::NOT_FOUND, format!("Backend '{}' not found in {} profile", name, profile_key(target))).into_response(),
    }
}

// ── Copy / clone backend ──────────────────────────────────────────────────────

/// Request body for `POST /api/config/backends/{name}/copy`.
#[derive(Debug, Deserialize)]
pub struct CopyBackendRequest {
    /// Target profile: "claude" or "copilot".
    /// May equal the source profile (in which case this is a same-profile clone).
    pub target_profile: String,
    /// New backend `name` (unique id) in the target profile.
    pub new_name: String,
    /// Optional new display name. If omitted, reuses the source display name.
    #[serde(default)]
    pub new_display_name: Option<String>,
}

/// POST /api/config/backends/{name}/copy?profile=src — copy a backend
/// (including its api_key) into another profile or clone within the same profile.
///
/// Server-side copy is required because `api_key` is never exposed to the
/// browser (it's masked in all GET responses), so a purely client-side
/// duplicate would lose the credential.
///
/// Validates:
/// - source backend exists in `?profile=`
/// - `new_name` is non-empty and doesn't collide with an existing backend in
///   the target profile
/// - target profile is a known profile key
///
/// Persists the updated config and hot-updates `BackendState` when the
/// target profile matches the running instance's cli_mode.
pub async fn post_copy_backend(
    State(state): State<WebuiState>,
    Query(q): Query<ProfileQuery>,
    Path(name): Path<String>,
    Json(req): Json<CopyBackendRequest>,
) -> Response {
    let source_mode = match q.resolve(state.cli_mode) {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    let target_mode = match req.target_profile.as_str() {
        "claude" => CliMode::Claude,
        "copilot" => CliMode::Copilot,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid target_profile '{}'. Must be 'claude' or 'copilot'.", other),
            )
                .into_response();
        }
    };

    let new_name = req.new_name.trim().to_string();
    if new_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "new_name must not be empty".to_string()).into_response();
    }

    let existing = state.config_store.get();

    // Look up the source backend and deep-clone it (preserves api_key).
    let source_backend = {
        let src = existing.profile(source_mode);
        match src.backends.iter().find(|b| b.name == name) {
            Some(b) => b.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    format!(
                        "Backend '{}' not found in {} profile",
                        name,
                        profile_key(source_mode)
                    ),
                )
                    .into_response();
            }
        }
    };

    // Check name uniqueness in the target profile.
    if existing
        .profile(target_mode)
        .backends
        .iter()
        .any(|b| b.name == new_name)
    {
        return (
            StatusCode::CONFLICT,
            format!(
                "Backend '{}' already exists in {} profile",
                new_name,
                profile_key(target_mode)
            ),
        )
            .into_response();
    }

    // Build the new backend from the cloned source.
    let mut copied = source_backend;
    copied.name = new_name.clone();
    if let Some(dn) = req.new_display_name.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        copied.display_name = dn.to_string();
    }

    // Insert into the target profile and persist.
    let mut new_config = existing.clone();
    new_config.profile_mut(target_mode).backends.push(copied);

    if let Err(e) = new_config.validate_for(target_mode) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let path = state.config_store.path().to_path_buf();
    if let Err(e) = save_config(&path, &new_config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let _ = state.config_store.reload();

    // Hot-update BackendState when the target profile is the running instance's mode.
    if target_mode == state.cli_mode {
        let new_profile = new_config.profile(target_mode).clone();
        if let Err(e) = state.backend_state.update_config(new_profile) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Config saved but runtime update failed: {e}"),
            )
                .into_response();
        }
    }

    Json(profile_to_dto(
        new_config.profile(target_mode),
        profile_key(target_mode),
    ))
    .into_response()
}
