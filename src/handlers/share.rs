use crate::auth::CurrentUser;
use crate::crypto::{is_valid_slug, SLUG_LEN};
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

#[derive(Debug, Deserialize)]
pub struct CreateShareRequest {
    pub slug: String,
    pub encrypted_payload: String,
}

pub async fn new_share_page(
    CurrentUser(_user): CurrentUser,
) -> Result<Html<String>, AppError> {
    Ok(Html(NewShareTemplate.render()?))
}

pub async fn create_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateShareRequest>,
) -> Result<(), AppError> {
    if req.slug.len() != SLUG_LEN || !is_valid_slug(&req.slug) {
        return Err(AppError::BadRequest("slug 无效"));
    }
    if req.encrypted_payload.is_empty() {
        return Err(AppError::BadRequest("加密内容不能为空"));
    }

    sqlx::query("INSERT INTO shares (user_id, slug, encrypted_payload) VALUES (?, ?, ?)")
        .bind(user.id)
        .bind(&req.slug)
        .bind(&req.encrypted_payload)
        .execute(&state.db)
        .await?;

    Ok(())
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
