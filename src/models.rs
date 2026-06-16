use serde::Deserialize;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Share {
    pub id: i64,
    pub user_id: i64,
    pub slug: String,
    pub encrypted_payload: String,
    pub kdf_salt: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterForm {
    pub username: String,
    pub password: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeForm {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}
