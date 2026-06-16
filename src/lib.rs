pub mod auth;
pub mod crypto;
pub mod db;
pub mod error;
pub mod handlers;
pub mod models;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::SqlitePool,
}

pub fn build_app(db: sqlx::SqlitePool) -> Router {
    let state = Arc::new(AppState { db });

    let session_store = MemoryStore::default();
    // Cookies are marked `Secure` only when explicitly enabled (e.g. behind an
    // HTTPS-terminating proxy). The app is served over plain HTTP by default, and
    // browsers drop `Secure` cookies on non-localhost HTTP — which would lose the
    // session right after login. Default off; set SECURE_COOKIES=true under HTTPS.
    let secure_cookies = std::env::var("SECURE_COOKIES")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let session_layer = SessionManagerLayer::new(session_store).with_secure(secure_cookies);

    Router::new()
        .nest_service("/static", ServeDir::new("static"))
        .route("/", get(handlers::dashboard::index))
        .route("/register", get(handlers::auth::register_page).post(handlers::auth::register))
        .route("/login", get(handlers::auth::login_page).post(handlers::auth::login))
        .route("/logout", post(handlers::auth::logout))
        .route("/dashboard", get(handlers::dashboard::dashboard))
        .route("/shares/new", get(handlers::share::new_share_page))
        .route("/api/shares", post(handlers::share::create_share))
        .route("/api/shares/:id/delete", post(handlers::share::delete_share))
        .route("/s/:slug", get(handlers::share::view_share))
        .route("/api/shares/:slug", get(handlers::share::get_share_payload))
        .layer(session_layer)
        .with_state(state)
}
