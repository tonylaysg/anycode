//! WebUI module — browser-based configuration management.
//!
//! Serves a single-page management interface at `/ui/` and a REST API
//! at `/api/config/`. Runs as an **independent HTTP server** (separate from
//! the proxy server), so it can be bound to `0.0.0.0` for LAN/remote access
//! while the proxy stays on `127.0.0.1`.
//!
//! Optional Basic Auth is supported via the `[webui] password` config field.

mod api;

pub use api::WebuiState;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use std::sync::Arc;
use tokio::net::TcpListener;

static INDEX_HTML: &str = include_str!("index.html");

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

/// Optional Basic Auth state shared with the auth middleware.
#[derive(Clone)]
struct AuthState {
    /// Pre-encoded `Basic <base64(username:password)>` value, or None when auth disabled.
    expected: Option<Arc<String>>,
}

/// Middleware: enforce Basic Auth when a password is configured.
async fn basic_auth_middleware(
    State(auth): State<AuthState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if let Some(ref expected) = auth.expected {
        let provided = req
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if provided != expected.as_str() {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("WWW-Authenticate", r#"Basic realm="AnyClaude WebUI""#)
                .body(Body::from("Unauthorized"))
                .unwrap();
        }
    }
    next.run(req).await
}

/// Encode username+password into `Basic <base64(username:password)>` header value.
fn encode_basic_auth(username: &str, password: &str) -> String {
    use std::io::Write;
    let input = format!("{}:{}", username, password);
    let mut buf = Vec::new();
    // base64 encode manually using standard library
    // We use a simple byte-by-byte approach to avoid adding a dependency
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] as u32 } else { 0 };
        let _ = write!(buf, "{}", TABLE[((b0 >> 2) & 0x3f) as usize] as char);
        let _ = write!(buf, "{}", TABLE[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            let _ = write!(buf, "{}", TABLE[(((b1 & 0xf) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            buf.push(b'=');
        }
        if i + 2 < bytes.len() {
            let _ = write!(buf, "{}", TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            buf.push(b'=');
        }
        i += 3;
    }
    format!("Basic {}", String::from_utf8(buf).unwrap_or_default())
}

/// Build the WebUI router (routes only, no server binding).
fn build_webui_router(state: WebuiState, auth: AuthState) -> Router {
    Router::new()
        .route("/ui/", get(serve_index))
        .route("/api/config", get(api::get_config))
        .route("/api/config", put(api::put_config))
        .route("/api/config/active", post(api::post_active_backend))
        .route("/api/config/backends/{name}", get(api::get_backend))
        .layer(axum::middleware::from_fn_with_state(auth, basic_auth_middleware))
        .with_state(state)
}

/// Start the WebUI HTTP server.
///
/// Binds to `bind_addr`, optionally enforces Basic Auth with `username`+`password`.
/// Returns the actual bound address (useful when port 0 is used).
/// This function runs until the server exits.
pub async fn run_webui_server(
    state: WebuiState,
    bind_addr: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    let auth = AuthState {
        expected: match (username, password) {
            (Some(u), Some(p)) => Some(Arc::new(encode_basic_auth(u, p))),
            _ => None,
        },
    };

    let app = build_webui_router(state, auth);

    let listener = TcpListener::bind(bind_addr).await
        .map_err(|e| format!("WebUI: cannot bind to '{}': {}", bind_addr, e))?;

    let addr = listener.local_addr()?;

    axum::serve(listener, app).await
        .map_err(|e| format!("WebUI server error: {}", e))?;

    Ok(addr)
}

/// Bind the WebUI listener and return (SocketAddr, TcpListener).
///
/// Separated from `serve_webui` so the caller can log the address
/// before spawning the server task.
pub async fn bind_webui(
    bind_addr: &str,
) -> Result<(std::net::SocketAddr, TcpListener), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(bind_addr).await
        .map_err(|e| format!("WebUI: cannot bind to '{}': {}", bind_addr, e))?;
    let addr = listener.local_addr()?;
    Ok((addr, listener))
}

/// Serve WebUI on a pre-bound listener.
pub async fn serve_webui(
    listener: TcpListener,
    state: WebuiState,
    username: Option<String>,
    password: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth = AuthState {
        expected: match (username.as_deref(), password.as_deref()) {
            (Some(u), Some(p)) => Some(Arc::new(encode_basic_auth(u, p))),
            _ => None,
        },
    };
    let app = build_webui_router(state, auth);
    axum::serve(listener, app).await?;
    Ok(())
}
