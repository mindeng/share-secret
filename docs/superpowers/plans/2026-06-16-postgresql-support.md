# PostgreSQL Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add PostgreSQL as a runtime-selectable database backend alongside SQLite, chosen by the `DATABASE_URL` scheme, from a single binary.

**Architecture:** Switch `AppState.db` from `SqlitePool` to `sqlx::AnyPool`. Queries keep their portable `?` placeholders (the Any driver rewrites them to `$1` for Postgres). Only the schema DDL branches per backend, selected by URL scheme. A spike proves Any↔Postgres before the app is migrated; SQLite remains the in-memory test backend.

**Tech Stack:** Rust, axum 0.7, sqlx 0.8 (`any` + `sqlite` + `postgres` + `tls-rustls`), tower-sessions.

---

## Background for the implementer

This is a zero-knowledge secret-sharing app. The server stores only ciphertext. The database layer is tiny: 8 runtime queries (`sqlx::query` / `query_as` / `query_scalar`) all using `?` placeholders, spread across `src/auth.rs`, `src/handlers/auth.rs`, `src/handlers/share.rs`, `src/handlers/dashboard.rs`. The models `User` and `Share` (`src/models.rs`) derive `sqlx::FromRow` with field types `i64`, `String`, `Option<String>` — all inside the Any driver's supported type subset. `Share.created_at` is a `String`.

Key facts you must respect:
- The Any driver requires `sqlx::any::install_default_drivers()` to be called **once** per process before any pool connects (a second call errors). We centralize this behind a `std::sync::Once` helper `share_secret::db::install_drivers_once()`.
- Postgres types the SQL literal `1` as `int4`, which fails to decode as `i64` under Any. The existing slug-existence check `query_scalar::<_, i64>("SELECT 1 …")` must become a row-presence check.
- `created_at` must be a `TEXT` column on Postgres (not `TIMESTAMP`) so it keeps decoding into `Share.created_at: String`.
- The Dockerfile builds with `--locked`, so `Cargo.lock` changes must be committed.

---

## File Structure

- `Cargo.toml` — add sqlx features `any`, `postgres`, `tls-rustls`.
- `src/db.rs` — `install_drivers_once()`, `init_db()`/`init_db_memory()` returning `AnyPool`, and `init_sqlite_schema()` / `init_postgres_schema()`.
- `src/lib.rs` — `AppState.db` and `build_app` use `sqlx::AnyPool`.
- `src/handlers/share.rs` — change the slug-existence check to row-presence.
- `tests/postgres_test.rs` — new gated test file (spike + full-flow), skips when `TEST_DATABASE_URL` is unset.
- `README.md` / `k8s/README.md` — document `DATABASE_URL` backends and `TEST_DATABASE_URL`.

---

## Task 1: Add sqlx features + driver helper + Postgres spike (gated)

Proves the Any driver rewrites `?`→`$1` and decodes `i64`/`String`/`Option<String>` against a real Postgres, **before** any app code is migrated. Without `TEST_DATABASE_URL` the spike skips, so the default suite is unaffected.

**Files:**
- Modify: `Cargo.toml:10`
- Modify: `src/db.rs` (add `install_drivers_once`)
- Create: `tests/postgres_test.rs`

- [ ] **Step 1: Add sqlx features**

In `Cargo.toml`, change line 10 from:

```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "migrate"] }
```

to:

```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "sqlite", "postgres", "any", "migrate"] }
```

- [ ] **Step 2: Add the one-time driver-install helper to `src/db.rs`**

At the top of `src/db.rs`, add the import and a `Once`-guarded installer. Add these lines after the existing `use` statements (after line 4):

```rust
use std::sync::Once;

static DRIVERS: Once = Once::new();

/// 进程内只安装一次 Any 驱动（重复安装会报错）。init_db/测试都通过它来安装。
pub fn install_drivers_once() {
    DRIVERS.call_once(|| {
        sqlx::any::install_default_drivers();
    });
}
```

- [ ] **Step 3: Verify it still builds (and Cargo.lock updates)**

Run: `cargo build`
Expected: PASS. New Postgres/TLS crates are compiled and `Cargo.lock` is updated.

- [ ] **Step 4: Write the gated spike test**

Create `tests/postgres_test.rs`:

```rust
use sqlx::any::AnyPoolOptions;

/// Postgres 测试需显式提供 TEST_DATABASE_URL（postgres://...），否则跳过。
fn pg_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .filter(|u| u.starts_with("postgres"))
}

/// 证明 Any 驱动会把 `?` 占位符改写为 `$1`，并能正确解码 i64 / String / Option<String>。
#[tokio::test]
async fn spike_any_postgres_placeholders_and_types() {
    let Some(url) = pg_url() else {
        eprintln!("skipping spike_any_postgres_placeholders_and_types: TEST_DATABASE_URL not set");
        return;
    };
    share_secret::db::install_drivers_once();
    let pool = AnyPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect postgres");

    sqlx::query("DROP TABLE IF EXISTS spike_t")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE spike_t (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT NOT NULL, note TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // INSERT with `?` placeholders — Any must rewrite to $1, $2 for Postgres.
    sqlx::query("INSERT INTO spike_t (name, note) VALUES (?, ?)")
        .bind("alice")
        .bind(Option::<String>::None)
        .execute(&pool)
        .await
        .expect("insert with ? placeholders");

    // SELECT back with a `?` placeholder and decode i64 + String + Option<String>.
    let row: (i64, String, Option<String>) =
        sqlx::query_as("SELECT id, name, note FROM spike_t WHERE name = ?")
            .bind("alice")
            .fetch_one(&pool)
            .await
            .expect("select and decode");

    assert!(row.0 >= 1);
    assert_eq!(row.1, "alice");
    assert_eq!(row.2, None);

    sqlx::query("DROP TABLE spike_t").execute(&pool).await.unwrap();
}
```

- [ ] **Step 5: Run the spike**

Run (no Postgres): `cargo test --test postgres_test`
Expected: PASS — the test prints the skip message and returns (0 failures).

If a Postgres instance is available, run with it to actually prove the Any behavior:
Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test --test postgres_test -- --nocapture`
Expected: PASS. **If this fails on placeholder rewriting or type decoding, STOP and escalate** — the design's fallback is the enum/repository approach, and no app code has been migrated yet.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/db.rs tests/postgres_test.rs
git commit -m "feat: add sqlx postgres/any features and a gated Any<->Postgres spike"
```

---

## Task 2: Migrate the database layer to `AnyPool`

Switches the whole app to `AnyPool` with per-backend DDL. Verified by the existing 27-test SQLite suite staying green (now running through Any).

**Files:**
- Modify: `src/db.rs` (full rewrite)
- Modify: `src/lib.rs:19-23` (`AppState`) and `src/lib.rs:25-32` (`build_app`)
- Modify: `src/handlers/share.rs:53-58` (slug-existence check)
- Test: existing `tests/integration_test.rs` (must stay green)

- [ ] **Step 1: Rewrite `src/db.rs`**

Replace the entire contents of `src/db.rs` with:

```rust
use sqlx::any::AnyPoolOptions;
use sqlx::AnyPool;
use std::env;
use std::sync::Once;

static DRIVERS: Once = Once::new();

/// 进程内只安装一次 Any 驱动（重复安装会报错）。init_db/测试都通过它来安装。
pub fn install_drivers_once() {
    DRIVERS.call_once(|| {
        sqlx::any::install_default_drivers();
    });
}

fn is_postgres(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

pub async fn init_db() -> AnyPool {
    install_drivers_once();
    let mut database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:share_secret.db".to_string());

    let postgres = is_postgres(&database_url);

    // SQLite 默认不会自动建库文件；补上 mode=rwc 以保持原有 create_if_missing 行为。
    if !postgres && !database_url.contains(":memory:") && !database_url.contains("mode=") {
        let sep = if database_url.contains('?') { '&' } else { '?' };
        database_url = format!("{database_url}{sep}mode=rwc");
    }

    let pool = AnyPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to connect to database");

    if postgres {
        init_postgres_schema(&pool).await;
    } else {
        init_sqlite_schema(&pool).await;
    }

    pool
}

pub async fn init_db_memory() -> AnyPool {
    install_drivers_once();
    let pool = AnyPoolOptions::new()
        .max_connections(5)
        .connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory sqlite");

    init_sqlite_schema(&pool).await;
    pool
}

/// 每条语句单独执行：Any 驱动不保证支持单次调用里的多语句。
pub async fn init_sqlite_schema(pool: &AnyPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create users table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create shares table");

    // 迁移旧库：若 kdf_salt 列不存在则补上（已存在会报错，忽略即可）。
    let _ = sqlx::query("ALTER TABLE shares ADD COLUMN kdf_salt TEXT")
        .execute(pool)
        .await;
}

pub async fn init_postgres_schema(pool: &AnyPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (now())::text
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create users table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS shares (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            user_id BIGINT NOT NULL REFERENCES users(id),
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at TEXT NOT NULL DEFAULT (now())::text
        )",
    )
    .execute(pool)
    .await
    .expect("failed to create shares table");
}
```

- [ ] **Step 2: Update `AppState` and `build_app` in `src/lib.rs`**

Change the `AppState` struct field type (line 20) from:

```rust
    pub db: sqlx::SqlitePool,
```

to:

```rust
    pub db: sqlx::AnyPool,
```

And change `build_app`'s signature (line 25) from:

```rust
pub fn build_app(db: sqlx::SqlitePool) -> Router {
```

to:

```rust
pub fn build_app(db: sqlx::AnyPool) -> Router {
```

(No other changes in `lib.rs`. `main.rs` needs no change — `db::init_db()` now returns `AnyPool` and `build_app` accepts it.)

- [ ] **Step 3: Fix the slug-existence check in `src/handlers/share.rs`**

Replace the existence check (lines 53-58) — currently:

```rust
        let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM shares WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?
            .is_some();
```

with a row-presence check (Postgres types `1` as `int4`, which won't decode as `i64` under Any):

```rust
        let exists = sqlx::query("SELECT 1 FROM shares WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?
            .is_some();
```

- [ ] **Step 4: Run the full suite on SQLite-via-Any**

Run: `cargo test`
Expected: PASS — all 27 existing tests pass through `AnyPool`, and `postgres_test` skips. This is the key checkpoint proving Any works for SQLite end-to-end.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/db.rs src/lib.rs src/handlers/share.rs
git commit -m "feat: run the database layer on sqlx AnyPool (sqlite + postgres)"
```

---

## Task 3: Gated end-to-end Postgres integration test

Exercises the real HTTP flows (register → login → create → fetch → update → dashboard → delete) against Postgres, proving the migrated handlers and the `created_at` TEXT decoding work. Skips without `TEST_DATABASE_URL`.

**Files:**
- Modify: `tests/postgres_test.rs` (append helpers + the flow test)

- [ ] **Step 1: Append the helpers and end-to-end test to `tests/postgres_test.rs`**

Add these imports at the very top of `tests/postgres_test.rs` (keep the existing `use sqlx::any::AnyPoolOptions;`):

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use share_secret::security::{CodeStore, LoginGuard};
use share_secret::{build_router, AppState};
use std::sync::Arc;
use tower::ServiceExt;
```

Then append at the end of the file:

```rust
async fn body_string(body: Body) -> String {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// 连接 Postgres，重建空表，返回 app 与 state（带可读 state 引用）。
async fn make_app_pg(url: &str) -> (axum::Router, Arc<AppState>) {
    share_secret::db::install_drivers_once();
    let pool = AnyPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .expect("connect postgres");

    // 干净起步：先删表再按 Postgres schema 重建。
    sqlx::query("DROP TABLE IF EXISTS shares").execute(&pool).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS users").execute(&pool).await.unwrap();
    share_secret::db::init_postgres_schema(&pool).await;

    let state = Arc::new(AppState {
        db: pool,
        codes: CodeStore::new(),
        login_guard: LoginGuard::new(),
    });
    (build_router(state.clone()), state)
}

#[tokio::test]
async fn postgres_end_to_end_flow() {
    let Some(url) = pg_url() else {
        eprintln!("skipping postgres_end_to_end_flow: TEST_DATABASE_URL not set");
        return;
    };
    let (app, state) = make_app_pg(&url).await;

    // 注册（走验证码流程）
    let req = Request::builder()
        .method("POST")
        .uri("/register/code")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=pguser"))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();
    let code = state.codes.peek("pguser").expect("code issued");

    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username=pguser&password=secret&code={code}")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // 登录
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=pguser&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let cookie = res.headers().get("set-cookie").unwrap().clone();

    // 创建分享
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"pgcipher"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let slug = serde_json::from_str::<serde_json::Value>(&body_string(res.into_body()).await)
        .unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_string();

    // 匿名读取
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["encrypted_payload"].as_str(), Some("pgcipher"));
    assert_eq!(v["is_owner"].as_bool(), Some(false));

    // 所有者更新
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"pgupdated"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["encrypted_payload"].as_str(), Some("pgupdated"));

    // 仪表盘列出该分享（验证 created_at TEXT 能解码为 String）
    let req = Request::builder()
        .uri("/dashboard")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_string(res.into_body()).await.contains(&slug));

    // 删除（按 id；从库里取该 slug 的 BIGINT id，可正常解码为 i64）
    let id: i64 = sqlx::query_scalar("SELECT id FROM shares WHERE slug = ?")
        .bind(&slug)
        .fetch_one(&state.db)
        .await
        .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{id}/delete"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // 确认已删除
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run the gated test (skips without Postgres)**

Run: `cargo test --test postgres_test`
Expected: PASS — both `spike_…` and `postgres_end_to_end_flow` print skip messages and return (0 failures), confirming the file compiles and skips cleanly.

If Postgres is available:
Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test --test postgres_test -- --nocapture`
Expected: PASS — full flow runs against Postgres.

- [ ] **Step 3: Run the entire suite once more**

Run: `cargo test`
Expected: PASS — 27 SQLite tests green, Postgres tests skipped.

- [ ] **Step 4: Commit**

```bash
git add tests/postgres_test.rs
git commit -m "test: gated end-to-end Postgres integration test"
```

---

## Task 4: Documentation

**Files:**
- Modify: `README.md` (create the file if it is empty/absent)
- Modify: `k8s/README.md`

- [ ] **Step 1: Document database configuration in `README.md`**

First read `README.md` (it may be empty). Ensure it contains a "Database" section with this content (append it if the file already has other content; otherwise create the file with a title plus this section):

```markdown
## Database

`share-secret` supports two backends, selected at runtime by the `DATABASE_URL`
environment variable:

- **SQLite** (default): `DATABASE_URL=sqlite:share_secret.db`. The file is
  created automatically. Used by the test suite and simple/local deployments.
- **PostgreSQL**: `DATABASE_URL=postgres://user:password@host:5432/dbname`. The
  database must already exist; the app creates its tables on startup. TLS is
  supported (e.g. managed providers) with no extra system libraries.

Notes:
- `created_at` is stored as text on both backends.
- Tables are created automatically at startup; there is no separate migration
  step.

### Running the PostgreSQL tests

The default `cargo test` runs entirely on in-memory SQLite. The Postgres
integration tests in `tests/postgres_test.rs` are skipped unless you point them
at a real database:

```sh
TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres \
  cargo test --test postgres_test -- --nocapture
```
```

- [ ] **Step 2: Note the Postgres option in `k8s/README.md`**

Read `k8s/README.md`, then add a short note documenting that the deployment defaults to SQLite on a PVC and can be switched to Postgres by changing the `DATABASE_URL` env in `k8s/base/deployment.yaml` to a `postgres://…` URL (sourced from a Secret) and removing the SQLite volume. Add this paragraph in a sensible place (e.g. after the existing description of the deployment):

```markdown
## Database backend

The base manifests run on SQLite backed by the PersistentVolumeClaim mounted at
`/data` (`DATABASE_URL=sqlite:/data/share_secret.db`). To use PostgreSQL instead,
point `DATABASE_URL` in `k8s/base/deployment.yaml` at a `postgres://…` URL
(inject the credentials from a Secret) and drop the `/data` volume + PVC. The
application binary supports both backends with no rebuild.
```

- [ ] **Step 3: Verify the build is unaffected**

Run: `cargo build`
Expected: PASS (docs-only change).

- [ ] **Step 4: Commit**

```bash
git add README.md k8s/README.md
git commit -m "docs: document DATABASE_URL backends and Postgres tests"
```

---

## Final verification

- [ ] `cargo test` — 27 SQLite tests pass; Postgres tests skip. Expected: green.
- [ ] `cargo build --release` — clean production build. Expected: success.
- [ ] (If a Postgres instance is available) `TEST_DATABASE_URL=postgres://… cargo test --test postgres_test -- --nocapture` — full Postgres flow passes.
