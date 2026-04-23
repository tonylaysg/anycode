//! WebUI module — browser-based configuration management.
//!
//! Serves a single-page management interface at `/ui/` and a REST API
//! at `/api/config/`. Runs as an **independent HTTP server** (separate from
//! the proxy server), so it can be bound to `0.0.0.0` for LAN/remote access
//! while the proxy stays on `127.0.0.1`.
//!
//! Authentication uses session cookies with a custom HTML login page.
//! No browser-native Basic Auth dialogs.

mod api;

pub use api::WebuiState;

use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

static INDEX_HTML: &str = include_str!("index.html");
static LOGIN_HTML: &str = include_str!("login.html");

const SESSION_TTL: Duration = Duration::from_secs(24 * 3600); // 24 hours
const SESSION_COOKIE: &str = "ac_session";

// ── Session store ─────────────────────────────────────────────────────────────

/// In-memory session store: token → expiry time.
#[derive(Clone, Default)]
struct SessionStore(Arc<Mutex<HashMap<String, Instant>>>);

impl SessionStore {
    fn create(&self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let expiry = Instant::now() + SESSION_TTL;
        let mut map = self.0.lock().unwrap();
        // Purge expired sessions while we have the lock
        map.retain(|_, exp| *exp > Instant::now());
        map.insert(token.clone(), expiry);
        token
    }

    fn is_valid(&self, token: &str) -> bool {
        let map = self.0.lock().unwrap();
        map.get(token).map(|exp| *exp > Instant::now()).unwrap_or(false)
    }

    fn revoke(&self, token: &str) {
        self.0.lock().unwrap().remove(token);
    }
}

// ── Auth state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AuthState {
    /// Expected credentials (username, password). None = auth disabled.
    credentials: Option<Arc<(String, String)>>,
    sessions: SessionStore,
}

/// Constant-time byte comparison — prevents timing attacks on credentials.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}

fn get_cookie(req: &Request<Body>, name: &str) -> Option<String> {
    req.headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|part| {
                let part = part.trim();
                let (k, v) = part.split_once('=')?;
                if k.trim() == name { Some(v.trim().to_string()) } else { None }
            })
        })
}

// ── Auth middleware ───────────────────────────────────────────────────────────

async fn auth_middleware(
    State(auth): State<AuthState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // No credentials configured → pass through
    let Some(_) = &auth.credentials else {
        return next.run(req).await;
    };

    // Login page itself is always accessible
    let path = req.uri().path();
    if path == "/login" {
        return next.run(req).await;
    }

    // Check session cookie
    if let Some(token) = get_cookie(&req, SESSION_COOKIE) {
        if auth.sessions.is_valid(&token) {
            return next.run(req).await;
        }
    }

    // Not authenticated — redirect to login page
    let redirect_to = format!("/login?next={}", urlencoded(req.uri().path()));
    Redirect::to(&redirect_to).into_response()
}

fn urlencoded(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/' {
                vec![c]
            } else {
                format!("%{:02X}", c as u32).chars().collect()
            }
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

// ── Combined app state ────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    webui: WebuiState,
    auth: AuthState,
}

// ── Request handlers (all use AppState) ───────────────────────────────────────

async fn handler_login_get() -> impl IntoResponse {
    Html(LOGIN_HTML)
}

async fn handler_login_post(
    State(app): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let Some(ref creds) = app.auth.credentials else {
        return Redirect::to("/ui/").into_response();
    };
    let (expected_user, expected_pass) = creds.as_ref();
    let ok = constant_time_eq(form.username.as_bytes(), expected_user.as_bytes())
          && constant_time_eq(form.password.as_bytes(), expected_pass.as_bytes());
    if ok {
        let token = app.auth.sessions.create();
        Response::builder()
            .status(StatusCode::OK)
            .header("Set-Cookie", format!(
                "{}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
                SESSION_COOKIE, token, SESSION_TTL.as_secs()
            ))
            .body(Body::empty())
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("Invalid credentials"))
            .unwrap()
    }
}

async fn handler_logout(State(app): State<AppState>, req: Request<Body>) -> Response {
    if let Some(token) = get_cookie(&req, SESSION_COOKIE) {
        app.auth.sessions.revoke(&token);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header("Set-Cookie", format!(
            "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0", SESSION_COOKIE
        ))
        .body(Body::empty())
        .unwrap()
}

async fn handler_get_config(State(app): State<AppState>) -> Response {
    api::get_config(State(app.webui)).await.into_response()
}
async fn handler_put_config(
    State(app): State<AppState>,
    json: axum::Json<api::ConfigDto>,
) -> Response {
    api::put_config(State(app.webui), json).await
}
async fn handler_post_active(
    State(app): State<AppState>,
    json: axum::Json<api::ActiveBackendRequest>,
) -> Response {
    api::post_active_backend(State(app.webui), json).await
}
async fn handler_get_backend(
    State(app): State<AppState>,
    path: axum::extract::Path<String>,
) -> Response {
    api::get_backend(State(app.webui), path).await.into_response()
}

async fn auth_mw(State(app): State<AppState>, req: Request<Body>, next: Next) -> Response {
    auth_middleware(State(app.auth), req, next).await
}

// ── Router builder ────────────────────────────────────────────────────────────

fn build_router(app: AppState) -> Router {
    Router::new()
        .route("/ui/",                          get(serve_index))
        .route("/login",                        get(handler_login_get).post(handler_login_post))
        .route("/logout",                       post(handler_logout))
        .route("/api/config",                   get(handler_get_config).put(handler_put_config))
        .route("/api/config/active",            post(handler_post_active))
        .route("/api/config/backends/{name}",   get(handler_get_backend))
        .layer(axum::middleware::from_fn_with_state(app.clone(), auth_mw))
        .with_state(app)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Bind the WebUI listener and return `(SocketAddr, TcpListener)`.
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
    let credentials = match (username, password) {
        (Some(u), Some(p)) => Some(Arc::new((u, p))),
        _ => None,
    };
    let app = AppState {
        webui: state,
        auth: AuthState { credentials, sessions: SessionStore::default() },
    };
    axum::serve(listener, build_router(app)).await?;
    Ok(())
}
