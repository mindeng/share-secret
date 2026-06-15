use crate::auth::{hash_password, login_user, logout_user, verify_password};
use crate::models::{LoginForm, RegisterForm};
use crate::AppState;
use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
    Form,
};
use std::sync::Arc;
use tower_sessions::Session;

#[derive(Template)]
#[template(path = "register.html")]
pub struct RegisterTemplate {
    pub error: Option<String>,
}

impl IntoResponse for RegisterTemplate {
    fn into_response(self) -> axum::response::Response {
        Html(self.render().expect("render register template")).into_response()
    }
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

impl IntoResponse for LoginTemplate {
    fn into_response(self) -> axum::response::Response {
        Html(self.render().expect("render login template")).into_response()
    }
}

pub async fn register_page() -> RegisterTemplate {
    RegisterTemplate { error: None }
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Form(form): Form<RegisterForm>,
) -> Result<Redirect, RegisterTemplate> {
    if form.username.is_empty() || form.password.is_empty() {
        return Err(RegisterTemplate {
            error: Some("用户名和密码不能为空".to_string()),
        });
    }

    let password_hash = hash_password(&form.password);
    let result = sqlx::query("INSERT INTO users (username, password_hash) VALUES (?, ?)")
        .bind(&form.username)
        .bind(&password_hash)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => Ok(Redirect::to("/login")),
        Err(_) => Err(RegisterTemplate {
            error: Some("用户名已存在".to_string()),
        }),
    }
}

pub async fn login_page() -> LoginTemplate {
    LoginTemplate { error: None }
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Result<Redirect, LoginTemplate> {
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, password_hash FROM users WHERE username = ?",
    )
    .bind(&form.username)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| {
        LoginTemplate {
            error: Some("数据库错误".to_string()),
        }
    })?;

    match row {
        Some((id, hash)) if verify_password(&form.password, &hash) => {
            login_user(&session, id).await;
            Ok(Redirect::to("/dashboard"))
        }
        _ => Err(LoginTemplate {
            error: Some("用户名或密码错误".to_string()),
        }),
    }
}

pub async fn logout(session: Session) -> Redirect {
    logout_user(&session).await;
    Redirect::to("/")
}
