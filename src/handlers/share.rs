use crate::auth::CurrentUser;
use crate::crypto::generate_slug;
use crate::error::AppError;
use crate::AppState;
use askama::Template;
use axum::{extract::State, response::Html, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Template)]
#[template(path = "new_share.html")]
pub struct NewShareTemplate;

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
    for _ in 0..10 {
        let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM shares WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?
            .is_some();
        if !exists {
            break;
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

pub async fn view_share() -> Html<&'static str> {
    Html("")
}

pub async fn get_share_payload() -> Html<&'static str> {
    Html("")
}
