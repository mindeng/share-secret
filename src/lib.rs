pub mod auth;
pub mod crypto;
pub mod db;
pub mod error;
pub mod handlers;
pub mod models;
pub mod security;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};

use crate::security::{CodeStore, LoginGuard};

pub struct AppState {
    pub db: sqlx::AnyPool,
    pub codes: CodeStore,
    pub login_guard: LoginGuard,
}

pub fn build_app(db: sqlx::AnyPool) -> Router {
    let state = Arc::new(AppState {
        db,
        codes: CodeStore::new(),
        login_guard: LoginGuard::new(),
    });
    build_router(state)
}

/// 用已构造的 state 组装路由（测试可注入并保留对 state 的引用）。
pub fn build_router(state: Arc<AppState>) -> Router {
    let session_store = MemoryStore::default();
    // Session cookies are marked `Secure` by default (safe for HTTPS deployments).
    // For plain-HTTP local development, set SECURE_COOKIES=false — otherwise browsers
    // drop the cookie on non-localhost HTTP and the session is lost right after login.
    let secure_cookies = std::env::var("SECURE_COOKIES")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);
    let session_layer = SessionManagerLayer::new(session_store).with_secure(secure_cookies);

    Router::new()
        .nest_service("/static", ServeDir::new("static"))
        .route("/", get(handlers::dashboard::index))
        .route(
            "/register",
            get(handlers::auth::register_page).post(handlers::auth::register),
        )
        .route("/register/code", post(handlers::auth::register_code))
        .route(
            "/login",
            get(handlers::auth::login_page).post(handlers::auth::login),
        )
        .route("/logout", post(handlers::auth::logout))
        .route("/dashboard", get(handlers::dashboard::dashboard))
        .route("/shares/new", get(handlers::share::new_share_page))
        .route("/api/shares", post(handlers::share::create_share))
        .route("/api/shares/:id/delete", post(handlers::share::delete_share))
        .route("/s/:slug", get(handlers::share::view_share))
        .route("/api/shares/:slug", get(handlers::share::get_share_payload))
        .route("/api/shares/:slug/update", post(handlers::share::update_share))
        .layer(session_layer)
        .with_state(state)
}
