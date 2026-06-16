use crate::error::AppError;
use crate::models::User;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use std::sync::Arc;
use tower_sessions::Session;

use crate::AppState;

const USER_ID_KEY: &str = "user_id";

pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .expect("hash password")
        .to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed_hash = PasswordHash::new(hash).expect("parse hash");
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

pub async fn login_user(session: &Session, user_id: i64) {
    session
        .insert(USER_ID_KEY, user_id)
        .await
        .expect("insert session");
}

pub async fn logout_user(session: &Session) {
    session
        .remove::<i64>(USER_ID_KEY)
        .await
        .expect("remove session");
}

/// 从 session 读取当前用户 id（未登录返回 None，永不报错）。
pub async fn current_user_id(session: &Session) -> Option<i64> {
    session.get::<i64>(USER_ID_KEY).await.ok().flatten()
}

#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

#[async_trait]
impl FromRequestParts<Arc<AppState>> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::Auth("未登录"))?;
        let user_id: Option<i64> = session
            .get(USER_ID_KEY)
            .await
            .map_err(|_| AppError::Auth("session 错误"))?;
        let user_id = user_id.ok_or(AppError::Auth("未登录"))?;

        let user: User =
            sqlx::query_as("SELECT id, username, password_hash FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.db)
                .await
                .map_err(|_| AppError::Auth("用户不存在"))?;

        Ok(CurrentUser(user))
    }
}
