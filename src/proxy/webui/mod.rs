//! WebUI module — browser-based configuration management.
//!
//! Serves a single-page management interface at `/ui/` and a REST API
//! at `/api/config/` for reading and writing the AnyClaude configuration.
//!
//! Routes are designed to be merged into the main axum Router without
//! any session-token auth (localhost-only, same policy as hook endpoints).

mod api;

pub use api::WebuiState;

use axum::Router;
use axum::routing::{get, post, put};
use axum::response::{Html, IntoResponse};

static INDEX_HTML: &str = include_str!("index.html");

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

/// Build the WebUI sub-router.
///
/// Mount this under the main router (no prefix — paths are `/ui/` and
/// `/api/config/...`).
pub fn build_webui_router(state: WebuiState) -> Router {
    Router::new()
        // Browser UI
        .route("/ui/", get(serve_index))
        // Config REST API
        .route("/api/config", get(api::get_config))
        .route("/api/config", put(api::put_config))
        .route("/api/config/active", post(api::post_active_backend))
        .route("/api/config/backends/{name}", get(api::get_backend))
        .with_state(state)
}
