use crate::auth::CurrentUser;
use crate::crypto::generate_slug;
use crate::error::AppError;
use crate::AppState;
use askama::Template;
use axum::{extract::{Path, State}, response::Html, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Template)]
#[template(path = "new_share.html")]
pub struct NewShareTemplate;

#[derive(Template)]
#[template(path = "view_share.html")]
pub struct ViewShareTemplate;

#[derive(Serialize)]
pub struct SharePayloadResponse {
    pub encrypted_payload: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateShareRequest {
    pub encrypted_payload: String,
}

#[derive(Debug, Serialize)]
pub struct CreateShareResponse {
    pub slug: String,
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
) -> Result<Json<CreateShareResponse>, AppError> {
    if req.encrypted_payload.is_empty() {
        return Err(AppError::BadRequest("加密内容不能为空"));
    }

    let mut slug = generate_slug();
    for attempt in 0..10 {
        let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM shares WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?
            .is_some();
        if !exists {
            break;
        }
        if attempt == 9 {
            return Err(AppError::BadRequest("无法生成唯一链接，请重试"));
        }
        slug = generate_slug();
    }

    sqlx::query("INSERT INTO shares (user_id, slug, encrypted_payload) VALUES (?, ?, ?)")
        .bind(user.id)
        .bind(&slug)
        .bind(&req.encrypted_payload)
        .execute(&state.db)
        .await?;

    Ok(Json(CreateShareResponse { slug }))
}

pub async fn delete_share() -> Html<&'static str> {
    Html("")
}

pub async fn view_share(Path(_slug): Path<String>) -> Result<Html<String>, AppError> {
    Ok(Html(ViewShareTemplate.render()?))
}

pub async fn get_share_payload(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<Json<SharePayloadResponse>, AppError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT encrypted_payload FROM shares WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await?;

    match row {
        Some((encrypted_payload,)) => Ok(Json(SharePayloadResponse { encrypted_payload })),
        None => Err(AppError::NotFound),
    }
}
