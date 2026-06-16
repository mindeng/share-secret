use crate::auth::{current_user_id, CurrentUser};
use crate::crypto::generate_slug;
use crate::error::AppError;
use crate::AppState;
use askama::Template;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_sessions::Session;

#[derive(Template)]
#[template(path = "new_share.html")]
pub struct NewShareTemplate;

#[derive(Template)]
#[template(path = "view_share.html")]
pub struct ViewShareTemplate;

#[derive(Serialize)]
pub struct SharePayloadResponse {
    pub encrypted_payload: String,
    pub kdf_salt: Option<String>,
    pub is_owner: bool,
}

#[derive(Debug, Deserialize)]
pub struct SharePayload {
    pub encrypted_payload: String,
    #[serde(default)]
    pub kdf_salt: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateShareResponse {
    pub slug: String,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExportShare {
    pub slug: String,
    pub encrypted_payload: String,
    pub kdf_salt: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportEnvelope {
    pub version: i32,
    pub shares: Vec<ExportShare>,
}

#[derive(Debug, Serialize)]
pub struct ImportSummary {
    pub imported: usize,
    pub skipped: usize,
    pub errors: usize,
}

pub async fn new_share_page(
    CurrentUser(_user): CurrentUser,
) -> Result<Html<String>, AppError> {
    Ok(Html(NewShareTemplate.render()?))
}

pub async fn create_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SharePayload>,
) -> Result<Json<CreateShareResponse>, AppError> {
    if req.encrypted_payload.is_empty() {
        return Err(AppError::BadRequest("加密内容不能为空"));
    }

    let mut slug = generate_slug();
    for attempt in 0..10 {
        let exists = sqlx::query("SELECT 1 FROM shares WHERE slug = $1")
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

    sqlx::query("INSERT INTO shares (user_id, slug, encrypted_payload, kdf_salt) VALUES ($1, $2, $3, $4)")
        .bind(user.id)
        .bind(&slug)
        .bind(&req.encrypted_payload)
        .bind(&req.kdf_salt)
        .execute(&state.db)
        .await?;

    Ok(Json(CreateShareResponse { slug }))
}

pub async fn update_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Path(slug): Path<String>,
    Json(req): Json<SharePayload>,
) -> Result<StatusCode, AppError> {
    if req.encrypted_payload.is_empty() {
        return Err(AppError::BadRequest("加密内容不能为空"));
    }

    let result = sqlx::query(
        "UPDATE shares SET encrypted_payload = $1, kdf_salt = $2 WHERE slug = $3 AND user_id = $4",
    )
    .bind(&req.encrypted_payload)
    .bind(&req.kdf_salt)
    .bind(&slug)
    .bind(user.id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Forbidden);
    }

    Ok(StatusCode::OK)
}

pub async fn delete_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Redirect, AppError> {
    let result = sqlx::query("DELETE FROM shares WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Forbidden);
    }

    Ok(Redirect::to("/dashboard"))
}

pub async fn view_share(Path(_slug): Path<String>) -> Result<Html<String>, AppError> {
    Ok(Html(ViewShareTemplate.render()?))
}

pub async fn get_share_payload(
    State(state): State<Arc<AppState>>,
    session: Session,
    Path(slug): Path<String>,
) -> Result<Json<SharePayloadResponse>, AppError> {
    let row: Option<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT user_id, encrypted_payload, kdf_salt FROM shares WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?;

    match row {
        Some((owner_id, encrypted_payload, kdf_salt)) => {
            let is_owner = current_user_id(&session).await == Some(owner_id);
            Ok(Json(SharePayloadResponse { encrypted_payload, kdf_salt, is_owner }))
        }
        None => Err(AppError::NotFound),
    }
}

pub async fn export_shares(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, AppError> {
    let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT slug, encrypted_payload, kdf_salt, CAST(created_at AS TEXT) FROM shares WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

    let shares: Vec<ExportShare> = rows
        .into_iter()
        .map(|(slug, encrypted_payload, kdf_salt, created_at)| ExportShare {
            slug,
            encrypted_payload,
            kdf_salt,
            created_at,
        })
        .collect();

    let envelope = ExportEnvelope { version: 1, shares };
    // Serializing owned plain structs to a String is infallible.
    let body = serde_json::to_string(&envelope).expect("serialize export envelope");

    Ok((
        [
            (header::CONTENT_TYPE, "application/json"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"share-secret-export.json\"",
            ),
        ],
        body,
    ))
}
