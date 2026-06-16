# 注册验证码 + 登录限流 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 注册必须输入服务端打印到控制台的验证码（错误次数受限），登录密码错误次数受限并触发临时锁定。

**Architecture:** 新增内存模块 `src/security.rs`，含 `CodeStore`（验证码：6 位数字、10 分钟有效、错 5 次作废、60 秒冷却）与 `LoginGuard`（登录：连错 5 次锁 15 分钟，按用户名维度），均为 `Mutex<HashMap>` + `Instant` 计时，挂在 `AppState` 上。Handler 只调用其方法；时长参数可注入以便单测。

**Tech Stack:** Rust 2024、axum 0.7、askama 0.13、sqlx(sqlite)、argon2、rand 0.8、tower-sessions（现有），`std::sync::Mutex` + `std::time::{Instant, Duration}`（新增使用）。

---

## File Structure

- **Create** `src/security.rs` — `CodeStore`、`LoginGuard` 两个内存限流结构体 + 单元测试。
- **Modify** `src/lib.rs` — 注册 `security` 模块；`AppState` 增 `codes`/`login_guard` 字段；新增 `build_router(state)`；`build_app(db)` 复用之；加路由 `POST /register/code`。
- **Modify** `src/models.rs` — `RegisterForm` 增 `code`；新增 `CodeForm`。
- **Modify** `src/handlers/auth.rs` — 新增 `register_code` handler 与 `CodeResponse`；`register`/`login` 接入限流。
- **Modify** `templates/register.html` — 验证码输入框 + "获取验证码"按钮 + 内联 JS。
- **Modify** `tests/integration_test.rs` — 改造辅助函数走验证码流程；新增限流用例。

> `templates/login.html` 无需改动（锁定提示复用现有 `error` 字段）。

---

## Task 1: `CodeStore`（验证码内存存储）

**Files:**
- Create: `src/security.rs`
- Test: `src/security.rs`（内联 `#[cfg(test)]`）

- [ ] **Step 1: 写失败测试**

新建 `src/security.rs`，写入：

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::Rng;

// ---- 默认参数 ----
const CODE_TTL: Duration = Duration::from_secs(600); // 10 分钟
const CODE_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_CODE_ATTEMPTS: u32 = 5;

/// 验证码校验失败原因。
#[derive(Debug, PartialEq, Eq)]
pub enum CodeError {
    NoCode,
    Expired,
    Wrong,
    TooManyAttempts,
}

struct CodeEntry {
    code: String,
    created_at: Instant,
    expires_at: Instant,
    attempts: u32,
}

/// 注册验证码的内存存储（按用户名）。
pub struct CodeStore {
    ttl: Duration,
    cooldown: Duration,
    max_attempts: u32,
    inner: Mutex<HashMap<String, CodeEntry>>,
}

#[cfg(test)]
mod code_tests {
    use super::*;

    fn store() -> CodeStore {
        // 短时长便于测试过期/冷却
        CodeStore::with_params(Duration::from_millis(50), Duration::from_millis(50), 3)
    }

    #[test]
    fn correct_code_passes_and_is_consumed() {
        let s = store();
        let code = s.issue("alice").expect("first issue ok");
        assert_eq!(s.verify("alice", &code), Ok(()));
        // 已消费：再验证应为 NoCode
        assert_eq!(s.verify("alice", &code), Err(CodeError::NoCode));
    }

    #[test]
    fn wrong_code_invalidated_after_max_attempts() {
        let s = store();
        let _ = s.issue("bob").unwrap();
        assert_eq!(s.verify("bob", "000000"), Err(CodeError::Wrong));
        assert_eq!(s.verify("bob", "000000"), Err(CodeError::Wrong));
        // 第 3 次达到上限 -> 作废
        assert_eq!(s.verify("bob", "000000"), Err(CodeError::TooManyAttempts));
        // 作废后码已删除
        assert_eq!(s.verify("bob", "000000"), Err(CodeError::NoCode));
    }

    #[test]
    fn issue_respects_cooldown() {
        let s = store();
        let _ = s.issue("carol").unwrap();
        assert!(s.issue("carol").is_err(), "second issue within cooldown rejected");
    }

    #[test]
    fn verify_without_issue_is_no_code() {
        let s = store();
        assert_eq!(s.verify("dave", "123456"), Err(CodeError::NoCode));
    }
}
```

- [ ] **Step 2: 跑测试确认失败（编译失败）**

Run: `cargo test --lib security::code_tests`
Expected: 编译错误，`CodeStore::with_params`/`issue`/`verify` 未定义。

- [ ] **Step 3: 实现 `CodeStore`**

在 `src/security.rs` 的 `code_tests` 模块**之前**插入实现：

```rust
fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    let n: u32 = rng.gen_range(0..1_000_000);
    format!("{n:06}")
}

impl CodeStore {
    pub fn new() -> Self {
        Self::with_params(CODE_TTL, CODE_COOLDOWN, MAX_CODE_ATTEMPTS)
    }

    pub fn with_params(ttl: Duration, cooldown: Duration, max_attempts: u32) -> Self {
        Self {
            ttl,
            cooldown,
            max_attempts,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 生成并存储新验证码；冷却期内返回 `Err(剩余时长)`。
    /// 返回明文码，由调用方负责打印到控制台。
    pub fn issue(&self, username: &str) -> Result<String, Duration> {
        let now = Instant::now();
        let mut map = self.inner.lock().expect("codes lock");
        if let Some(entry) = map.get(username) {
            let elapsed = now.duration_since(entry.created_at);
            if elapsed < self.cooldown {
                return Err(self.cooldown - elapsed);
            }
        }
        let code = generate_code();
        map.insert(
            username.to_string(),
            CodeEntry {
                code: code.clone(),
                created_at: now,
                expires_at: now + self.ttl,
                attempts: 0,
            },
        );
        Ok(code)
    }

    /// 校验验证码。成功消费该码；错误累计到上限则作废。
    pub fn verify(&self, username: &str, code: &str) -> Result<(), CodeError> {
        let now = Instant::now();
        let mut map = self.inner.lock().expect("codes lock");
        let entry = match map.get_mut(username) {
            Some(e) => e,
            None => return Err(CodeError::NoCode),
        };
        if now >= entry.expires_at {
            map.remove(username);
            return Err(CodeError::Expired);
        }
        if entry.code == code {
            map.remove(username);
            return Ok(());
        }
        entry.attempts += 1;
        if entry.attempts >= self.max_attempts {
            map.remove(username);
            return Err(CodeError::TooManyAttempts);
        }
        Err(CodeError::Wrong)
    }

    /// 仅供测试/调试：查看当前明文码。
    pub fn peek(&self, username: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("codes lock")
            .get(username)
            .map(|e| e.code.clone())
    }
}

impl Default for CodeStore {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib security::code_tests`
Expected: 4 个测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/security.rs
git commit -m "feat: add CodeStore for registration verification codes"
```

---

## Task 2: `LoginGuard`（登录锁定）

**Files:**
- Modify: `src/security.rs`
- Test: `src/security.rs`（内联 `#[cfg(test)]`）

- [ ] **Step 1: 写失败测试**

在 `src/security.rs` 末尾追加：

```rust
#[cfg(test)]
mod login_tests {
    use super::*;

    fn guard() -> LoginGuard {
        LoginGuard::with_params(3, Duration::from_secs(900))
    }

    #[test]
    fn locks_after_max_failures() {
        let g = guard();
        assert!(g.check("alice").is_ok());
        g.record_failure("alice");
        g.record_failure("alice");
        assert!(g.check("alice").is_ok(), "still ok below threshold");
        g.record_failure("alice"); // 第 3 次 -> 锁定
        assert!(g.check("alice").is_err(), "locked at threshold");
    }

    #[test]
    fn success_clears_failures() {
        let g = guard();
        g.record_failure("bob");
        g.record_failure("bob");
        g.record_success("bob");
        // 计数清零后再失败一次不应锁定
        g.record_failure("bob");
        assert!(g.check("bob").is_ok());
    }

    #[test]
    fn expired_lock_resets() {
        // 锁定时长设为 0，check 时立即视为已过期并复位
        let g = LoginGuard::with_params(1, Duration::from_millis(0));
        g.record_failure("carol"); // 立即锁定，但 lock_duration=0
        assert!(g.check("carol").is_ok(), "zero-duration lock already expired");
    }
}
```

- [ ] **Step 2: 跑测试确认失败（编译失败）**

Run: `cargo test --lib security::login_tests`
Expected: 编译错误，`LoginGuard` 未定义。

- [ ] **Step 3: 实现 `LoginGuard`**

在 `src/security.rs` 中、`code_tests` 模块**之前**（紧接 `CodeStore` 的实现之后）插入：

```rust
const MAX_LOGIN_FAILURES: u32 = 5;
const LOGIN_LOCK: Duration = Duration::from_secs(900); // 15 分钟

struct Attempt {
    failures: u32,
    locked_until: Option<Instant>,
}

/// 登录失败锁定（按 key，当前传 username；将来可换 IP）。
pub struct LoginGuard {
    max_failures: u32,
    lock_duration: Duration,
    inner: Mutex<HashMap<String, Attempt>>,
}

impl LoginGuard {
    pub fn new() -> Self {
        Self::with_params(MAX_LOGIN_FAILURES, LOGIN_LOCK)
    }

    pub fn with_params(max_failures: u32, lock_duration: Duration) -> Self {
        Self {
            max_failures,
            lock_duration,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 锁定中返回 `Err(剩余时长)`；锁定已过期则复位并放行。
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let mut map = self.inner.lock().expect("login lock");
        if let Some(a) = map.get_mut(key) {
            if let Some(until) = a.locked_until {
                if now < until {
                    return Err(until - now);
                }
                a.failures = 0;
                a.locked_until = None;
            }
        }
        Ok(())
    }

    /// 记一次失败；达上限则设置锁定截止时间。
    pub fn record_failure(&self, key: &str) {
        let now = Instant::now();
        let mut map = self.inner.lock().expect("login lock");
        let a = map.entry(key.to_string()).or_insert(Attempt {
            failures: 0,
            locked_until: None,
        });
        a.failures += 1;
        if a.failures >= self.max_failures {
            a.locked_until = Some(now + self.lock_duration);
        }
    }

    /// 登录成功：清除该 key 的失败记录。
    pub fn record_success(&self, key: &str) {
        self.inner.lock().expect("login lock").remove(key);
    }
}

impl Default for LoginGuard {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib security`
Expected: `code_tests` + `login_tests` 全部 PASS（7 个）。

- [ ] **Step 5: 提交**

```bash
git add src/security.rs
git commit -m "feat: add LoginGuard for login lockout"
```

---

## Task 3: 接入 `AppState` 与路由

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: 修改 `src/lib.rs`**

将文件顶部模块声明区（第 1-6 行）替换为新增 `security` 模块：

```rust
pub mod auth;
pub mod crypto;
pub mod db;
pub mod error;
pub mod handlers;
pub mod models;
pub mod security;
```

将 `use` 区与 `AppState`/`build_app`（第 8-47 行）整体替换为：

```rust
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};

use crate::security::{CodeStore, LoginGuard};

pub struct AppState {
    pub db: sqlx::SqlitePool,
    pub codes: CodeStore,
    pub login_guard: LoginGuard,
}

pub fn build_app(db: sqlx::SqlitePool) -> Router {
    let state = Arc::new(AppState {
        db,
        codes: CodeStore::new(),
        login_guard: LoginGuard::new(),
    });
    build_router(state)
}

/// 用已构造的 state 组装路由（测试可注入并保留对 state 的引用）。
pub fn build_router(state: Arc<AppState>) -> Router {
    let session_store = MemoryStore::default();
    // Session cookies are marked `Secure` by default (safe for HTTPS deployments).
    // For plain-HTTP local development, set SECURE_COOKIES=false — otherwise browsers
    // drop the cookie on non-localhost HTTP and the session is lost right after login.
    let secure_cookies = std::env::var("SECURE_COOKIES")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);
    let session_layer = SessionManagerLayer::new(session_store).with_secure(secure_cookies);

    Router::new()
        .nest_service("/static", ServeDir::new("static"))
        .route("/", get(handlers::dashboard::index))
        .route(
            "/register",
            get(handlers::auth::register_page).post(handlers::auth::register),
        )
        .route("/register/code", post(handlers::auth::register_code))
        .route(
            "/login",
            get(handlers::auth::login_page).post(handlers::auth::login),
        )
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

> 注：`#[derive(Clone)]` 已从 `AppState` 移除（`Mutex` 不是 `Clone`，且状态以 `Arc<AppState>` 共享，无需 `Clone`）。

- [ ] **Step 2: 编译确认（此时 handler 尚未加，预期失败）**

Run: `cargo build`
Expected: 编译错误，`handlers::auth::register_code` 未找到 —— 下一个任务补上。先继续。

- [ ] **Step 3: 提交**

```bash
git add src/lib.rs
git commit -m "feat: wire CodeStore/LoginGuard into AppState and add code route"
```

---

## Task 4: 表单模型

**Files:**
- Modify: `src/models.rs`

- [ ] **Step 1: 修改 `RegisterForm` 并新增 `CodeForm`**

将 `src/models.rs` 中 `RegisterForm` 定义（第 20-24 行）替换为：

```rust
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
```

- [ ] **Step 2: 编译确认**

Run: `cargo build`
Expected: 仍因 `register_code` 未定义而失败（下个任务补）。`models.rs` 本身无新错误。

- [ ] **Step 3: 提交**

```bash
git add src/models.rs
git commit -m "feat: add code field to RegisterForm and CodeForm"
```

---

## Task 5: 注册流程 handler

**Files:**
- Modify: `src/handlers/auth.rs`

- [ ] **Step 1: 修改 import 区（第 1-11 行）**

替换为：

```rust
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
```

- [ ] **Step 2: 替换 `register` handler（第 41-64 行）**

替换为新的 `register` + `register_code` + `CodeResponse` + 错误文案辅助：

```rust
#[derive(Serialize)]
pub struct CodeResponse {
    pub ok: bool,
    pub message: String,
}

pub async fn register_code(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CodeForm>,
) -> Json<CodeResponse> {
    if form.username.trim().is_empty() {
        return Json(CodeResponse {
            ok: false,
            message: "请先填写用户名".to_string(),
        });
    }
    match state.codes.issue(&form.username) {
        Ok(code) => {
            println!("[验证码] 用户 {} 的注册验证码: {}", form.username, code);
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
    if form.username.is_empty() || form.password.is_empty() {
        return Err(RegisterTemplate {
            error: Some("用户名和密码不能为空".to_string()),
        });
    }

    if let Err(e) = state.codes.verify(&form.username, &form.code) {
        return Err(RegisterTemplate {
            error: Some(code_error_message(e)),
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
```

- [ ] **Step 3: 编译确认**

Run: `cargo build`
Expected: 成功（login handler 暂未改，但能编译）。若报 `login` 相关无关错误则检查改动范围。

- [ ] **Step 4: 提交**

```bash
git add src/handlers/auth.rs
git commit -m "feat: require verification code on registration"
```

---

## Task 6: 登录限流 handler

**Files:**
- Modify: `src/handlers/auth.rs`

- [ ] **Step 1: 替换 `login` handler（现 `login` 函数整体）**

替换为：

```rust
pub async fn login(
    State(state): State<Arc<AppState>>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Result<Redirect, LoginTemplate> {
    if let Err(remaining) = state.login_guard.check(&form.username) {
        let mins = remaining.as_secs() / 60 + 1;
        return Err(LoginTemplate {
            error: Some(format!("尝试过于频繁，请 {mins} 分钟后再试")),
        });
    }

    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
            .bind(&form.username)
            .fetch_optional(&state.db)
            .await
            .map_err(|_| LoginTemplate {
                error: Some("数据库错误".to_string()),
            })?;

    match row {
        Some((id, hash)) if verify_password(&form.password, &hash) => {
            state.login_guard.record_success(&form.username);
            login_user(&session, id).await;
            Ok(Redirect::to("/dashboard"))
        }
        _ => {
            state.login_guard.record_failure(&form.username);
            Err(LoginTemplate {
                error: Some("用户名或密码错误".to_string()),
            })
        }
    }
}
```

- [ ] **Step 2: 编译确认**

Run: `cargo build`
Expected: 成功，无警告级错误。

- [ ] **Step 3: 提交**

```bash
git add src/handlers/auth.rs
git commit -m "feat: rate-limit login with LoginGuard lockout"
```

---

## Task 7: 注册页模板

**Files:**
- Modify: `templates/register.html`

- [ ] **Step 1: 替换 `{% block content %}` 内容（第 5-16 行）**

替换为：

```html
{% block content %}
<h1>注册</h1>
{% if let Some(error) = error %}
<p style="color: red;">{{ error }}</p>
{% endif %}
<form method="post" action="/register">
    <label>用户名 <input type="text" name="username" id="username" required></label>
    <label>密码 <input type="password" name="password" required></label>
    <label>验证码 <input type="text" name="code" id="code" inputmode="numeric" required></label>
    <button type="button" id="get-code">获取验证码</button>
    <p id="code-msg"></p>
    <button type="submit">注册</button>
</form>
<p>已有账号？<a href="/login">登录</a></p>
<script>
document.getElementById('get-code').addEventListener('click', async function () {
    var username = document.getElementById('username').value;
    var msg = document.getElementById('code-msg');
    var res = await fetch('/register/code', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({ username: username }),
    });
    var data = await res.json();
    msg.style.color = data.ok ? 'green' : 'red';
    msg.textContent = data.message;
});
</script>
{% endblock %}
```

- [ ] **Step 2: 编译确认（Askama 编译期校验模板）**

Run: `cargo build`
Expected: 成功（模板解析通过）。

- [ ] **Step 3: 提交**

```bash
git add templates/register.html
git commit -m "feat: add verification code field and button to register page"
```

---

## Task 8: 集成测试

**Files:**
- Modify: `tests/integration_test.rs`

- [ ] **Step 1: 替换文件顶部 import 与辅助函数（第 1-9 行 + 第 132-149 行的 `register_and_login`）**

将文件顶部（第 1-9 行）替换为：

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use share_secret::security::{CodeStore, LoginGuard};
use share_secret::{build_router, db::init_db_memory, AppState};
use std::sync::Arc;
use tower::ServiceExt;

async fn body_string(body: Body) -> String {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn make_app() -> (axum::Router, Arc<AppState>) {
    let db = init_db_memory().await;
    let state = Arc::new(AppState {
        db,
        codes: CodeStore::new(),
        login_guard: LoginGuard::new(),
    });
    (build_router(state.clone()), state)
}

/// 走验证码流程注册一个用户，断言成功跳转。
async fn register_user(app: &axum::Router, state: &Arc<AppState>, user: &str, pass: &str) {
    let req = Request::builder()
        .method("POST")
        .uri("/register/code")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username={user}")))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let code = state.codes.peek(user).expect("code issued");
    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username={user}&password={pass}&code={code}")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

/// 注册并登录，返回会话 cookie。
async fn register_and_login(
    app: &axum::Router,
    state: &Arc<AppState>,
    user: &str,
) -> axum::http::HeaderValue {
    register_user(app, state, user, "secret").await;
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username={user}&password=secret")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    res.headers().get("set-cookie").unwrap().clone()
}
```

并删除文件原有位于第 132-149 行的旧 `register_and_login` 函数（已被上面替换版本取代——确保文件中只有一个 `register_and_login`）。

- [ ] **Step 2: 替换 `test_login_cookie_secure_by_default`**

```rust
#[tokio::test]
async fn test_login_cookie_secure_by_default() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "secitest").await;
    let s = cookie.to_str().unwrap().to_lowercase();
    assert!(
        s.contains("secure"),
        "session cookie should be Secure by default: {s}"
    );
}
```

- [ ] **Step 3: 替换 `test_register_and_login`**

```rust
#[tokio::test]
async fn test_register_and_login() {
    let (app, state) = make_app().await;
    register_user(&app, &state, "alice", "secret").await;

    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=alice&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}
```

- [ ] **Step 4: 替换 `test_create_and_fetch_share` 的 register+login 段（前两个请求）**

将该测试开头到取得 `cookie` 之前的注册/登录两段替换为：

```rust
#[tokio::test]
async fn test_create_and_fetch_share() {
    let (app, state) = make_app().await;

    register_user(&app, &state, "bob", "secret").await;

    // login
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=bob&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let cookie = res.headers().get("set-cookie").unwrap().clone();
```

（该测试其余部分——创建/获取 share——保持不变。）

- [ ] **Step 5: 替换 `test_create_share_requires_auth` 与 `test_fetch_missing_share_returns_404` 的 app 构造**

两处把 `let db = init_db_memory().await; let app = build_app(db);` 改为：

```rust
    let (app, _state) = make_app().await;
```

（两测试其余逻辑不变。）

- [ ] **Step 6: 替换 `test_password_protected_share_roundtrips_salt` 开头**

```rust
#[tokio::test]
async fn test_password_protected_share_roundtrips_salt() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "carol").await;
```

（其余不变。）

- [ ] **Step 7: 在文件末尾追加新限流用例**

```rust
#[tokio::test]
async fn test_register_requires_valid_code() {
    let (app, _state) = make_app().await;

    // 未获取验证码直接注册 -> 重渲染注册页并提示
    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=eve&password=secret&code=123456"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK); // 非 303 跳转
    let body = body_string(res.into_body()).await;
    assert!(body.contains("请先获取验证码"), "body: {body}");
}

#[tokio::test]
async fn test_register_rejects_wrong_code() {
    let (app, state) = make_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/register/code")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=frank"))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();
    let real = state.codes.peek("frank").expect("code issued");
    // 构造一个保证不同的错误码
    let wrong = if real == "000000" { "111111" } else { "000000" };

    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username=frank&password=secret&code={wrong}")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_string(res.into_body()).await;
    assert!(body.contains("验证码错误"), "body: {body}");
}

#[tokio::test]
async fn test_login_locks_after_failures() {
    let (app, state) = make_app().await;
    register_user(&app, &state, "grace", "secret").await;

    // 连续 5 次错误密码
    for _ in 0..5 {
        let req = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("username=grace&password=wrong"))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK); // 失败重渲染登录页
    }

    // 第 6 次即使密码正确也被锁定
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=grace&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK); // 未跳转 = 被拦
    let body = body_string(res.into_body()).await;
    assert!(body.contains("尝试过于频繁"), "body: {body}");
}
```

- [ ] **Step 8: 跑全部测试**

Run: `cargo test`
Expected: 全部 PASS（含单元 + 集成 + 新增 3 个限流用例）。

- [ ] **Step 9: 提交**

```bash
git add tests/integration_test.rs
git commit -m "test: cover verification code and login lockout flows"
```

---

## 收尾验证

- [ ] **Step 1: 全量构建 + 测试 + lint**

```bash
cargo build && cargo test && cargo clippy -- -D warnings
```
Expected: 构建通过、测试全绿、clippy 无警告。

- [ ] **Step 2: 手动冒烟（可选）**

```bash
SECURE_COOKIES=false cargo run
```
浏览器开 `/register`，点"获取验证码"→ 看终端打印 `[验证码] ...` → 填码注册 → `/login` 连错 5 次看锁定提示。

---

## Self-Review Notes

- **Spec 覆盖**：验证码注册（Task 4/5/7）、验证码错误限次（Task 1 + Task 5 + Task 8 Step 7）、登录限次锁定（Task 2 + Task 6 + Task 8 Step 7）、内存存储（Task 1/2）、控制台打印（Task 5 `println!`）、60s 冷却（Task 1 + Task 5 文案）、用户名维度（Task 6）、不碰 k8s（无相关任务）——全部有任务对应。
- **类型一致**：`CodeStore::{new,with_params,issue,verify,peek}`、`LoginGuard::{new,with_params,check,record_failure,record_success}`、`CodeError` 四分支、`CodeForm{username}`、`RegisterForm{username,password,code}`、`CodeResponse{ok,message}`、`build_router(Arc<AppState>)` 在定义处与调用处签名一致。
- **无占位符**：每步均含完整代码与可执行命令。
