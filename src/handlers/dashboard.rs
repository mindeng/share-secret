use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::models::Share;
use crate::AppState;
use askama::Template;
use axum::{extract::State, response::Html};
use std::sync::Arc;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate;

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub shares: Vec<Share>,
}

pub async fn index() -> Result<Html<String>, AppError> {
    Ok(Html(IndexTemplate.render()?))
}

pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>, AppError> {
    let shares: Vec<Share> = sqlx::query_as(
        "SELECT id, user_id, slug, encrypted_payload, created_at FROM shares WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

    Ok(Html(DashboardTemplate { shares }.render()?))
}
