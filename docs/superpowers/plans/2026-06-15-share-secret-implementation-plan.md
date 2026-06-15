# Share Secret Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现一个 Rust Web 服务，支持用户注册登录、浏览器端加密创建密文分享、通过带密钥的链接查看并一键复制字段值。

**Architecture:** 单服务 Axum + SQLite + Askama 服务端渲染，前端使用 Web Crypto API 进行 AES-GCM 加密/解密，服务端仅存储 slug 和加密 payload，无法访问明文。所有页面模板采用响应式设计，兼容 PC 和移动端。

**Tech Stack:** `axum`, `askama`, `sqlx` (sqlite), `argon2`, `tower-sessions`, `tower-cookies`, `rand`, `serde`, `base64`, `tokio`

---

## File Structure

```
share-secret/
├── Cargo.toml
├── src/
│   ├── main.rs              # 启动服务、路由配置
│   ├── db.rs                # SQLite 连接池与建表
│   ├── error.rs             # AppError 统一错误类型
│   ├── auth.rs              # 密码哈希、session 提取、登录态守卫
│   ├── models.rs            # User, Share, EncryptedPayload
│   ├── crypto.rs            # slug/key 生成工具函数
│   ├── handlers/
│   │   ├── auth.rs          # 注册/登录/登出
│   │   ├── dashboard.rs     # 首页/dashboard
│   │   └── share.rs         # 创建/删除/查看分享
│   └── templates/           # Askama 模板
├── static/
│   └── crypto.js            # 前端加密/解密/复制逻辑
├── templates/               # Askama html 模板
└── tests/
    └── integration_test.rs  # 集成测试
```

---

## Task 1: 项目依赖与基础结构

**Files:**
- Modify: `Cargo.toml`
- Create: `src/error.rs`
- Create: `src/db.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: 添加依赖到 Cargo.toml**

```toml
[package]
name = "share-secret"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
askama = "0.13"
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "migrate"] }
argon2 = "0.5"
tower-sessions = "0.13"
tower-cookies = "0.10"
rand = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"
once_cell = "1"
```

- [ ] **Step 2: 定义统一错误类型 `src/error.rs`**

```rust
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use askama::Error as AskamaError;

#[derive(Debug)]
pub enum AppError {
    Db(sqlx::Error),
    Template(AskamaError),
    Auth(&'static str),
    NotFound,
    Forbidden,
    BadRequest(&'static str),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, "数据库错误"),
            AppError::Template(_) => (StatusCode::INTERNAL_SERVER_ERROR, "模板渲染错误"),
            AppError::Auth(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::NotFound => (StatusCode::NOT_FOUND, "页面不存在"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "无权操作"),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
        };
        (status, message).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self { AppError::Db(e) }
}

impl From<AskamaError> for AppError {
    fn from(e: AskamaError) -> Self { AppError::Template(e) }
}
```

- [ ] **Step 3: 创建数据库模块 `src/db.rs`**

```rust
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::env;

pub async fn init_db() -> SqlitePool {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:share_secret.db".to_string());
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to connect to sqlite");

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
        "#,
    )
    .execute(&pool)
    .await
    .expect("failed to create tables");

    pool
}

pub async fn init_db_memory() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
        "#,
    )
    .execute(&pool)
    .await
    .expect("failed to create tables");

    pool
}
```

- [ ] **Step 4: 修改 `src/main.rs` 搭建基础服务**

```rust
mod auth;
mod crypto;
mod db;
mod error;
mod handlers;
mod models;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_sessions::{MemoryStore, SessionManagerLayer};

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::SqlitePool,
}

#[tokio::main]
async fn main() {
    let db = db::init_db().await;
    let state = Arc::new(AppState { db });

    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store);

    let app = Router::new()
        .route("/", get(handlers::dashboard::index))
        .route("/register", get(handlers::auth::register_page).post(handlers::auth::register))
        .route("/login", get(handlers::auth::login_page).post(handlers::auth::login))
        .route("/logout", post(handlers::auth::logout))
        .route("/dashboard", get(handlers::dashboard::dashboard))
        .route("/shares/new", get(handlers::share::new_share_page))
        .route("/api/shares", post(handlers::share::create_share))
        .route("/api/shares/:id/delete", post(handlers::share::delete_share))
        .route("/s/:slug", get(handlers::share::view_share))
        .route("/api/shares/:slug", get(handlers::share::get_share_payload))
        .layer(session_layer)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 5: 创建占位模块文件**

创建空文件（后续任务填充）：
- `src/auth.rs`
- `src/crypto.rs`
- `src/models.rs`
- `src/handlers/mod.rs`
- `src/handlers/auth.rs`
- `src/handlers/dashboard.rs`
- `src/handlers/share.rs`

- [ ] **Step 6: 编译检查**

Run: `cargo check`
Expected: 编译通过（此时处理器模块为空，但结构已搭建好）

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/main.rs src/error.rs src/db.rs src/auth.rs src/crypto.rs src/models.rs src/handlers/mod.rs src/handlers/auth.rs src/handlers/dashboard.rs src/handlers/share.rs
git commit -m "chore: setup axum project structure and dependencies"
```

---

## Task 2: 认证模块（argon2 密码 + session）

**Files:**
- Create: `src/auth.rs`
- Modify: `src/models.rs`
- Modify: `src/handlers/auth.rs`
- Create: `templates/register.html`, `templates/login.html`, `templates/base.html`

- [ ] **Step 1: 添加模型 `src/models.rs`**

```rust
use serde::{Deserialize, Serialize};

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
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterForm {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}
```

- [ ] **Step 2: 实现 `src/auth.rs`**

```rust
use crate::error::AppError;
use crate::models::User;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::extract::{FromRequestParts, State};
use http::request::Parts;
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
    session.insert(USER_ID_KEY, user_id).await.expect("insert session");
}

pub async fn logout_user(session: &Session) {
    session.remove::<i64>(USER_ID_KEY).await.expect("remove session");
}

#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl FromRequestParts<Arc<AppState>> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state).await.map_err(|_| AppError::Auth("未登录"))?;
        let user_id: Option<i64> = session.get(USER_ID_KEY).await.map_err(|_| AppError::Auth("session 错误"))?;
        let user_id = user_id.ok_or(AppError::Auth("未登录"))?;

        let user: User = sqlx::query_as("SELECT id, username, password_hash FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .map_err(|_| AppError::Auth("用户不存在"))?;

        Ok(CurrentUser(user))
    }
}
```

- [ ] **Step 3: 实现认证处理器 `src/handlers/auth.rs`**

```rust
use crate::auth::{hash_password, login_user, logout_user, verify_password};
use crate::error::AppError;
use crate::models::{LoginForm, RegisterForm};
use crate::AppState;
use askama::Template;
use axum::{
    extract::State,
    response::{Html, Redirect},
    Form,
};
use std::sync::Arc;
use tower_sessions::Session;

#[derive(Template)]
#[template(path = "register.html")]
pub struct RegisterTemplate {
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

pub async fn register_page() -> RegisterTemplate {
    RegisterTemplate { error: None }
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Form(form): Form<RegisterForm>,
) -> Result<Redirect, Html<RegisterTemplate>> {
    if form.username.is_empty() || form.password.is_empty() {
        return Err(Html(RegisterTemplate {
            error: Some("用户名和密码不能为空".to_string()),
        }));
    }

    let password_hash = hash_password(&form.password);
    let result = sqlx::query("INSERT INTO users (username, password_hash) VALUES (?, ?)")
        .bind(&form.username)
        .bind(&password_hash)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => Ok(Redirect::to("/login")),
        Err(_) => Err(Html(RegisterTemplate {
            error: Some("用户名已存在".to_string()),
        })),
    }
}

pub async fn login_page() -> LoginTemplate {
    LoginTemplate { error: None }
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Result<Redirect, Html<LoginTemplate>> {
    let row: Option<(i64, String)> = sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
        .bind(&form.username)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| AppError::Db(sqlx::Error::RowNotFound))
        .unwrap_or(None);

    match row {
        Some((id, hash)) if verify_password(&form.password, &hash) => {
            login_user(&session, id).await;
            Ok(Redirect::to("/dashboard"))
        }
        _ => Err(Html(LoginTemplate {
            error: Some("用户名或密码错误".to_string()),
        })),
    }
}

pub async fn logout(session: Session) -> Redirect {
    logout_user(&session).await;
    Redirect::to("/")
}
```

- [ ] **Step 4: 创建模板**

`templates/base.html`:

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{% block title %}Share Secret{% endblock %}</title>
    <style>
        * { box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            margin: 0;
            padding: 0;
            background: #f5f5f5;
            color: #333;
        }
        header {
            background: #2563eb;
            color: white;
            padding: 1rem;
        }
        header a {
            color: white;
            text-decoration: none;
            font-weight: bold;
            font-size: 1.25rem;
        }
        main {
            max-width: 720px;
            margin: 0 auto;
            padding: 1rem;
        }
        h1 { font-size: 1.5rem; margin-bottom: 1rem; }
        form label {
            display: block;
            margin-bottom: 0.75rem;
        }
        input, button {
            font-size: 1rem;
            padding: 0.5rem;
            width: 100%;
            margin-top: 0.25rem;
        }
        button {
            background: #2563eb;
            color: white;
            border: none;
            border-radius: 4px;
            cursor: pointer;
        }
        button:hover { background: #1d4ed8; }
        .field {
            background: white;
            padding: 1rem;
            border-radius: 8px;
            margin-bottom: 0.75rem;
            box-shadow: 0 1px 3px rgba(0,0,0,0.1);
        }
        table {
            width: 100%;
            border-collapse: collapse;
            background: white;
            border-radius: 8px;
            overflow: hidden;
            box-shadow: 0 1px 3px rgba(0,0,0,0.1);
        }
        td, th {
            padding: 0.75rem;
            border-bottom: 1px solid #eee;
        }
        td input { width: 100%; }
        td button { width: auto; padding: 0.4rem 0.8rem; }
        @media (max-width: 480px) {
            h1 { font-size: 1.25rem; }
            table, tr, td {
                display: block;
                width: 100%;
            }
            td { border-bottom: none; padding: 0.5rem 0.75rem; }
            tr { border-bottom: 1px solid #eee; margin-bottom: 0.5rem; }
        }
    </style>
</head>
<body>
    <header>
        <a href="/">Share Secret</a>
    </header>
    <main>
        {% block content %}{% endblock %}
    </main>
</body>
</html>
```

`templates/register.html`:

```html
{% extends "base.html" %}

{% block title %}注册 - Share Secret{% endblock %}

{% block content %}
<h1>注册</h1>
{% if let Some(error) = error %}
<p style="color: red;">{{ error }}</p>
{% endif %}
<form method="post" action="/register">
    <label>用户名 <input type="text" name="username" required></label>
    <label>密码 <input type="password" name="password" required></label>
    <button type="submit">注册</button>
</form>
<p>已有账号？<a href="/login">登录</a></p>
{% endblock %}
```

`templates/login.html`:

```html
{% extends "base.html" %}

{% block title %}登录 - Share Secret{% endblock %}

{% block content %}
<h1>登录</h1>
{% if let Some(error) = error %}
<p style="color: red;">{{ error }}</p>
{% endif %}
<form method="post" action="/login">
    <label>用户名 <input type="text" name="username" required></label>
    <label>密码 <input type="password" name="password" required></label>
    <button type="submit">登录</button>
</form>
<p>没有账号？<a href="/register">注册</a></p>
{% endblock %}
```

- [ ] **Step 5: 暴露 handlers 模块 `src/handlers/mod.rs`**

```rust
pub mod auth;
pub mod dashboard;
pub mod share;
```

- [ ] **Step 6: 修复 `src/handlers/auth.rs` 中的错误处理**

上面的实现直接 unwrap 了，需要调整成返回 `Result<Redirect, Html<LoginTemplate>>`。如果步骤 3 的代码已经可以直接编译，则跳过。

- [ ] **Step 7: 编译检查**

Run: `cargo check`
Expected: 通过

- [ ] **Step 8: Commit**

```bash
git add src/auth.rs src/models.rs src/handlers/auth.rs src/handlers/mod.rs templates/
git commit -m "feat: add user registration and login with argon2 sessions"
```

---

## Task 3: 前端加密与创建分享

**Files:**
- Create: `src/crypto.rs`
- Modify: `src/handlers/share.rs`
- Create: `templates/new_share.html`, `templates/share_created.html`
- Create: `static/crypto.js`

- [ ] **Step 1: 实现 `src/crypto.rs`**

```rust
use rand::RngCore;

const SLUG_LEN: usize = 12;
const KEY_LEN: usize = 32;

pub fn generate_slug() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    let mut bytes = vec![0u8; SLUG_LEN];
    rng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| CHARSET[(b % CHARSET.len() as u8) as usize] as char)
        .collect()
}

pub fn generate_key() -> Vec<u8> {
    let mut key = vec![0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    key
}
```

- [ ] **Step 2: 实现分享处理器 `src/handlers/share.rs`（创建部分）**

```rust
use crate::auth::CurrentUser;
use crate::crypto::{generate_key, generate_slug};
use crate::error::AppError;
use crate::AppState;
use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, Redirect},
    Json,
};
use serde::{Deserialize, Serialize};
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

pub async fn new_share_page() -> NewShareTemplate {
    NewShareTemplate
}

pub async fn create_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateShareRequest>,
) -> Result<Html<ShareCreatedTemplate>, AppError> {
    sqlx::query("INSERT INTO shares (user_id, slug, encrypted_payload) VALUES (?, ?, ?)")
        .bind(user.id)
        .bind(&req.slug)
        .bind(&req.encrypted_payload)
        .execute(&state.db)
        .await?;

    Ok(Html(ShareCreatedTemplate { slug: req.slug }))
}
```

- [ ] **Step 3: 创建 `templates/new_share.html`**

```html
{% extends "base.html" %}

{% block title %}创建分享 - Share Secret{% endblock %}

{% block content %}
<h1>创建分享</h1>
<form id="share-form">
    <label>标题 <input type="text" id="title" required></label>

    <div id="fields">
        <div class="field">
            <label>字段名 <input type="text" class="label" required></label>
            <label>值 <input type="text" class="value" required></label>
        </div>
    </div>
    <button type="button" id="add-field" style="margin-bottom: 1rem;">添加字段</button>

    <button type="submit">创建分享</button>
</form>

<div id="result" style="display:none; margin-top: 1rem; background: white; padding: 1rem; border-radius: 8px;">
    <p>分享链接：</p>
    <input type="text" id="share-link" readonly>
    <button id="copy-link">复制链接</button>
</div>

<script src="/static/crypto.js"></script>
<script>
    document.getElementById('add-field').addEventListener('click', () => {
        const div = document.createElement('div');
        div.className = 'field';
        div.innerHTML = `
            <label>字段名 <input type="text" class="label" required></label>
            <label>值 <input type="text" class="value" required></label>
        `;
        document.getElementById('fields').appendChild(div);
    });

    document.getElementById('share-form').addEventListener('submit', async (e) => {
        e.preventDefault();
        const title = document.getElementById('title').value;
        const fields = [];
        document.querySelectorAll('.field').forEach(el => {
            const label = el.querySelector('.label').value;
            const value = el.querySelector('.value').value;
            fields.push({ label, value });
        });

        const payload = { title, fields };
        const { slug, key, encryptedPayload } = await createShare(payload);

        const fullUrl = `${window.location.origin}/s/${slug}#key=${key}`;
        document.getElementById('share-link').value = fullUrl;
        document.getElementById('result').style.display = 'block';
    });

    document.getElementById('copy-link').addEventListener('click', () => {
        const input = document.getElementById('share-link');
        input.select();
        navigator.clipboard.writeText(input.value).then(() => {
            const btn = document.getElementById('copy-link');
            btn.textContent = '已复制';
            setTimeout(() => btn.textContent = '复制链接', 1500);
        });
    });
</script>
{% endblock %}
```

- [ ] **Step 4: 创建 `templates/share_created.html`**

```html
{% extends "base.html" %}

{% block title %}分享创建成功{% endblock %}

{% block content %}
<h1>分享创建成功</h1>
<p>请复制页面上的完整链接。关闭后无法再次获取。</p>
{% endblock %}
```

- [ ] **Step 5: 创建 `static/crypto.js`**

```javascript
async function deriveKey(rawKey) {
    const keyData = Uint8Array.from(atob(rawKey), c => c.charCodeAt(0));
    return await crypto.subtle.importKey(
        'raw',
        keyData,
        { name: 'AES-GCM' },
        false,
        ['encrypt', 'decrypt']
    );
}

function generateKey() {
    const bytes = crypto.getRandomValues(new Uint8Array(32));
    return btoa(String.fromCharCode(...bytes));
}

function generateSlug() {
    const charset = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    const bytes = crypto.getRandomValues(new Uint8Array(12));
    return Array.from(bytes).map(b => charset[b % charset.length]).join('');
}

async function encryptPayload(key, payload) {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const encoded = new TextEncoder().encode(JSON.stringify(payload));
    const cryptoKey = await deriveKey(key);
    const ciphertext = await crypto.subtle.encrypt(
        { name: 'AES-GCM', iv },
        cryptoKey,
        encoded
    );
    const combined = new Uint8Array(iv.length + ciphertext.byteLength);
    combined.set(iv);
    combined.set(new Uint8Array(ciphertext), iv.length);
    return btoa(String.fromCharCode(...combined));
}

async function decryptPayload(key, encrypted) {
    const combined = Uint8Array.from(atob(encrypted), c => c.charCodeAt(0));
    const iv = combined.slice(0, 12);
    const ciphertext = combined.slice(12);
    const cryptoKey = await deriveKey(key);
    const decrypted = await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv },
        cryptoKey,
        ciphertext
    );
    return JSON.parse(new TextDecoder().decode(decrypted));
}

async function createShare(payload) {
    const slug = generateSlug();
    const key = generateKey();
    const encryptedPayload = await encryptPayload(key, payload);

    const res = await fetch('/api/shares', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ slug, encrypted_payload: encryptedPayload })
    });

    if (!res.ok) {
        throw new Error('创建失败');
    }

    return { slug, key, encryptedPayload };
}
```

- [ ] **Step 6: 配置静态文件服务**

在 `src/main.rs` 的路由中添加：

```rust
.use_static_files("static")
```

如果使用 `tower-http`：

```toml
tower-http = { version = "0.6", features = ["fs"] }
```

```rust
use tower_http::services::ServeDir;

let app = Router::new()
    .nest_service("/static", ServeDir::new("static"))
    // ... routes
```

- [ ] **Step 7: 编译检查**

Run: `cargo check`
Expected: 通过

- [ ] **Step 8: Commit**

```bash
git add src/crypto.rs src/handlers/share.rs templates/new_share.html templates/share_created.html static/crypto.js Cargo.toml src/main.rs
git commit -m "feat: add client-side encryption and share creation"
```

---

## Task 4: 查看分享与一键复制

**Files:**
- Modify: `src/handlers/share.rs`
- Create: `templates/view_share.html`
- Modify: `static/crypto.js`

- [ ] **Step 1: 添加查看分享处理器 `src/handlers/share.rs`**

```rust
#[derive(Template)]
#[template(path = "view_share.html")]
pub struct ViewShareTemplate {
    pub slug: String,
}

#[derive(Serialize)]
pub struct SharePayloadResponse {
    pub encrypted_payload: String,
}

pub async fn view_share(Path(slug): Path<String>) -> ViewShareTemplate {
    ViewShareTemplate { slug }
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
```

- [ ] **Step 2: 创建 `templates/view_share.html`**

```html
{% extends "base.html" %}

{% block title %}查看分享 - Share Secret{% endblock %}

{% block content %}
<h1>查看分享</h1>
<div id="loading">正在加载...</div>
<div id="error" style="display:none; color: red;"></div>
<div id="content" style="display:none;">
    <h2 id="title"></h2>
    <table>
        <tbody id="fields"></tbody>
    </table>
</div>

<script src="/static/crypto.js"></script>
<script>
    (async () => {
        const hash = window.location.hash;
        const keyMatch = hash.match(/^#key=(.+)$/);
        if (!keyMatch) {
            document.getElementById('loading').style.display = 'none';
            document.getElementById('error').textContent = '链接不完整，缺少解密密钥';
            document.getElementById('error').style.display = 'block';
            return;
        }

        const key = keyMatch[1];
        const slug = window.location.pathname.split('/').pop();

        try {
            const res = await fetch(`/api/shares/${slug}`);
            if (!res.ok) throw new Error('分享不存在');
            const data = await res.json();
            const payload = await decryptPayload(key, data.encrypted_payload);

            document.getElementById('loading').style.display = 'none';
            document.getElementById('content').style.display = 'block';
            document.getElementById('title').textContent = payload.title;

            const tbody = document.getElementById('fields');
            payload.fields.forEach(field => {
                const tr = document.createElement('tr');
                tr.innerHTML = `
                    <td>${escapeHtml(field.label)}</td>
                    <td><input type="text" value="${escapeHtml(field.value)}" readonly></td>
                    <td><button class="copy-btn">复制</button></td>
                `;
                tbody.appendChild(tr);
            });

            document.querySelectorAll('.copy-btn').forEach(btn => {
                btn.addEventListener('click', () => {
                    const input = btn.closest('tr').querySelector('input');
                    navigator.clipboard.writeText(input.value).then(() => {
                        btn.textContent = '已复制';
                        setTimeout(() => btn.textContent = '复制', 1500);
                    });
                });
            });
        } catch (e) {
            document.getElementById('loading').style.display = 'none';
            document.getElementById('error').textContent = '无法解密，请检查链接是否完整';
            document.getElementById('error').style.display = 'block';
        }
    })();

    function escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }
</script>
{% endblock %}
```

- [ ] **Step 3: 更新 `static/crypto.js` 暴露 `decryptPayload` 和 `escapeHtml`**

`decryptPayload` 已在 Task 3 中实现。确保 `escapeHtml` 在 view_share.html 内联，或者移到 crypto.js 中都可以。

- [ ] **Step 4: 编译检查**

Run: `cargo check`
Expected: 通过

- [ ] **Step 5: Commit**

```bash
git add src/handlers/share.rs templates/view_share.html static/crypto.js
git commit -m "feat: add share viewing with client-side decryption and copy"
```

---

## Task 5: Dashboard 与删除分享

**Files:**
- Modify: `src/handlers/dashboard.rs`
- Modify: `src/handlers/share.rs`
- Create: `templates/dashboard.html`, `templates/index.html`

- [ ] **Step 1: 实现 `src/handlers/dashboard.rs`**

```rust
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::models::Share;
use crate::AppState;
use askama::Template;
use axum::{
    extract::State,
    response::{Html, Redirect},
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

pub async fn index() -> IndexTemplate {
    IndexTemplate
}

pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<DashboardTemplate>, AppError> {
    let shares: Vec<Share> = sqlx::query_as(
        "SELECT id, user_id, slug, encrypted_payload, created_at FROM shares WHERE user_id = ? ORDER BY created_at DESC"
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

    Ok(Html(DashboardTemplate { shares }))
}
```

- [ ] **Step 2: 创建 `templates/index.html`**

```html
{% extends "base.html" %}

{% block title %}Share Secret{% endblock %}

{% block content %}
<h1>Share Secret</h1>
<p>安全地分享密文。服务端无法读取您的内容。</p>
<p>
    <a href="/register">注册</a> |
    <a href="/login">登录</a>
</p>
{% endblock %}
```

- [ ] **Step 3: 创建 `templates/dashboard.html`**

```html
{% extends "base.html" %}

{% block title %}我的分享 - Share Secret{% endblock %}

{% block content %}
<h1>我的分享</h1>
<p><a href="/shares/new">创建新分享</a></p>

{% if shares.is_empty() %}
<p>暂无分享</p>
{% else %}
<ul style="list-style: none; padding: 0;">
    {% for share in shares %}
    <li style="background: white; padding: 1rem; border-radius: 8px; margin-bottom: 0.75rem; box-shadow: 0 1px 3px rgba(0,0,0,0.1); display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem;">
        <code style="word-break: break-all;">/s/{{ share.slug }}</code>
        <span style="color: #666; font-size: 0.875rem;">创建于 {{ share.created_at }}</span>
        <form method="post" action="/api/shares/{{ share.id }}/delete" style="margin-left: auto;" onsubmit="return confirm('确定删除？');">
            <button type="submit" style="width: auto;">删除</button>
        </form>
    </li>
    {% endfor %}
</ul>
{% endif %}

<form method="post" action="/logout" style="margin-top: 2rem;">
    <button type="submit">登出</button>
</form>
{% endblock %}
```

- [ ] **Step 4: 在 `src/handlers/share.rs` 添加删除处理器**

```rust
pub async fn delete_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Redirect, AppError> {
    let result = sqlx::query("DELETE FROM shares WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user.id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Forbidden);
    }

    Ok(Redirect::to("/dashboard"))
}
```

- [ ] **Step 5: 编译检查**

Run: `cargo check`
Expected: 通过

- [ ] **Step 6: Commit**

```bash
git add src/handlers/dashboard.rs templates/dashboard.html templates/index.html src/handlers/share.rs
git commit -m "feat: add dashboard and share deletion"
```

---

## Task 6: 集成测试

**Files:**
- Create: `tests/integration_test.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: 暴露 `build_app` 函数用于测试 `src/main.rs`**

```rust
pub fn build_app(db: SqlitePool) -> Router {
    let state = Arc::new(AppState { db });
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store);

    Router::new()
        .route("/", get(handlers::dashboard::index))
        .route("/register", get(handlers::auth::register_page).post(handlers::auth::register))
        .route("/login", get(handlers::auth::login_page).post(handlers::auth::login))
        .route("/logout", post(handlers::auth::logout))
        .route("/dashboard", get(handlers::dashboard::dashboard))
        .route("/shares/new", get(handlers::share::new_share_page))
        .route("/api/shares", post(handlers::share::create_share))
        .route("/api/shares/:id/delete", post(handlers::share::delete_share))
        .route("/s/:slug", get(handlers::share::view_share))
        .route("/api/shares/:slug", get(handlers::share::get_share_payload))
        .layer(session_layer)
        .with_state(state)
}
```

`main` 中调用 `build_app(db::init_db().await)`。

- [ ] **Step 2: 编写集成测试 `tests/integration_test.rs`**

```rust
use share_secret::{build_app, db::init_db_memory};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_register_and_login() {
    let db = init_db_memory().await;
    let app = build_app(db);

    let register_req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=alice&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(register_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=alice&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn test_create_and_fetch_share() {
    let db = init_db_memory().await;
    let app = build_app(db);

    // register
    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=bob&password=secret"))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    // login
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=bob&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let cookie = res.headers().get("set-cookie").unwrap().clone();

    // create share
    let payload = r#"{"slug":"abc123","encrypted_payload":"testpayload"}"#;
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(payload))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // fetch payload
    let req = Request::builder()
        .uri("/api/shares/abc123")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
```

- [ ] **Step 3: 运行测试**

Run: `cargo test`
Expected: 全部通过

- [ ] **Step 4: Commit**

```bash
git add tests/integration_test.rs src/main.rs
git commit -m "test: add integration tests for auth and share flow"
```

---

## Task 7: 收尾与验证

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`

- [ ] **Step 1: 确认所有依赖正确**

Run: `cargo tree | head -50`
Expected: 无冲突，依赖正常

- [ ] **Step 2: 运行完整测试与编译**

Run: `cargo test && cargo check && cargo clippy -- -D warnings`
Expected: 全部通过

- [ ] **Step 3: 手动启动验证**

Run: `cargo run`
Expected: 服务监听 0.0.0.0:3000

- [ ] **Step 4: Final commit**

```bash
git commit -m "feat: complete share-secret web service" --allow-empty
```

---

## Spec Coverage Review

| 需求 | 实现任务 |
|---|---|
| 用户注册/登录 | Task 2 |
| 创建公共分享链接 | Task 3 |
| 支持标题和自定义字段 | Task 3（前端 JSON 序列化） |
| 客户端加密所有信息 | Task 3（AES-GCM 加密完整 payload） |
| 查看页面一键复制字段值 | Task 4 |
| 创建者可删除分享 | Task 5 |
| 服务端无法解密 | Task 3/4/6（密钥在 fragment） |

## Placeholder Scan

- 无 TBD/TODO。
- 所有步骤包含完整代码或命令。
- 函数名/类型在全文一致。

## 注意

`CurrentUser` 的 `FromRequestParts` 实现可能需要根据 `tower-sessions` 实际 API 微调。
前端 `crypto.js` 使用 `crypto.subtle`，要求页面通过 HTTPS 或 localhost 访问。
