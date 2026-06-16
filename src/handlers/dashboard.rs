use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::models::Share;
use crate::AppState;
use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
};
use std::sync::Arc;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate;

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub shares: Vec<Share>,
}

pub async fn index(user: Option<CurrentUser>) -> Result<Response, AppError> {
    if user.is_some() {
        return Ok(Redirect::to("/dashboard").into_response());
    }
    Ok(Html(IndexTemplate.render()?).into_response())
}

pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>, AppError> {
    let shares: Vec<Share> = sqlx::query_as(
        "SELECT id, user_id, slug, encrypted_payload, kdf_salt, CAST(created_at AS TEXT) AS created_at FROM shares WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

    Ok(Html(DashboardTemplate { shares }.render()?))
}
