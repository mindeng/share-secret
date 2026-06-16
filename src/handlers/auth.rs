use crate::auth::{hash_password, login_user, logout_user, verify_password};
use crate::models::{CodeForm, LoginForm, RegisterForm};
use crate::security::CodeError;
use crate::AppState;
use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
    Form, Json,
};
use serde::Serialize;
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

#[derive(Serialize)]
pub struct CodeResponse {
    pub ok: bool,
    pub message: String,
}

pub async fn register_code(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CodeForm>,
) -> Json<CodeResponse> {
    let username = form.username.trim();
    if username.is_empty() {
        return Json(CodeResponse {
            ok: false,
            message: "请先填写用户名".to_string(),
        });
    }
    match state.codes.issue(username) {
        Ok(code) => {
            println!("[验证码] 用户 {username} 的注册验证码: {code}");
            Json(CodeResponse {
                ok: true,
                message: "验证码已打印到服务器控制台".to_string(),
            })
        }
        Err(remaining) => Json(CodeResponse {
            ok: false,
            message: format!("请 {} 秒后再获取验证码", remaining.as_secs() + 1),
        }),
    }
}

fn code_error_message(e: CodeError) -> String {
    match e {
        CodeError::NoCode => "请先获取验证码",
        CodeError::Expired => "验证码已过期，请重新获取",
        CodeError::Wrong => "验证码错误",
        CodeError::TooManyAttempts => "验证码错误次数过多，请重新获取",
    }
    .to_string()
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Form(form): Form<RegisterForm>,
) -> Result<Redirect, RegisterTemplate> {
    let username = form.username.trim();
    if username.is_empty() || form.password.is_empty() {
        return Err(RegisterTemplate {
            error: Some("用户名和密码不能为空".to_string()),
        });
    }

    if let Err(e) = state.codes.verify(username, &form.code) {
        return Err(RegisterTemplate {
            error: Some(code_error_message(e)),
        });
    }

    let password_hash = hash_password(&form.password);
    let result = sqlx::query("INSERT INTO users (username, password_hash) VALUES ($1, $2)")
        .bind(username)
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
    let username = form.username.trim();

    if let Err(remaining) = state.login_guard.check(username) {
        let mins = remaining.as_secs() / 60 + 1;
        return Err(LoginTemplate {
            error: Some(format!("尝试过于频繁，请 {mins} 分钟后再试")),
        });
    }

    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = $1")
            .bind(username)
            .fetch_optional(&state.db)
            .await
            .map_err(|_| LoginTemplate {
                error: Some("数据库错误".to_string()),
            })?;

    match row {
        Some((id, hash)) if verify_password(&form.password, &hash) => {
            state.login_guard.record_success(username);
            login_user(&session, id).await;
            Ok(Redirect::to("/dashboard"))
        }
        _ => {
            state.login_guard.record_failure(username);
            Err(LoginTemplate {
                error: Some("用户名或密码错误".to_string()),
            })
        }
    }
}

pub async fn logout(session: Session) -> Redirect {
    logout_user(&session).await;
    Redirect::to("/")
}
