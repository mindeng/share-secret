use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;
use askama::Template;
use axum::{
    extract::State,
    response::Html,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Template)]
#[template(path = "new_share.html")]
pub struct NewShareTemplate;

#[derive(Template)]
#[template(path = "share_created.html")]
pub struct ShareCreatedTemplate {
    pub slug: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateShareRequest {
    pub slug: String,
    pub encrypted_payload: String,
}

pub async fn new_share_page() -> Result<Html<String>, AppError> {
    Ok(Html(NewShareTemplate.render()?))
}

pub async fn create_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateShareRequest>,
) -> Result<Html<String>, AppError> {
    sqlx::query("INSERT INTO shares (user_id, slug, encrypted_payload) VALUES (?, ?, ?)")
        .bind(user.id)
        .bind(&req.slug)
        .bind(&req.encrypted_payload)
        .execute(&state.db)
        .await?;

    let template = ShareCreatedTemplate { slug: req.slug };
    Ok(Html(template.render()?))
}

pub async fn delete_share() -> Html<&'static str> {
    Html("")
}

pub async fn view_share() -> Html<&'static str> {
    Html("")
}

pub async fn get_share_payload() -> Html<&'static str> {
    Html("")
}
