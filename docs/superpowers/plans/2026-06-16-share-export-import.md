# Share Export / Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a logged-in user export all of their own shares to a JSON file and import such a file back, re-creating the shares (slug preserved) under their account.

**Architecture:** Two new JSON API endpoints on the existing axum router — `GET /api/shares/export` (auth, returns the user's ciphertext rows as a downloadable JSON envelope) and `POST /api/shares/import` (auth, inserts non-colliding rows under the current user). The dashboard gets an Export link and an Import button; import JS reads the file client-side and POSTs JSON, matching the codebase's existing `fetch`+`Json<...>` pattern. No decryption anywhere — this only moves ciphertext.

**Tech Stack:** Rust, axum 0.7, sqlx 0.8 (AnyPool: SQLite + Postgres), serde / serde_json, askama templates. Tests: `tokio::test` + `tower::ServiceExt::oneshot` against in-memory SQLite.

---

## Conventions you must follow

- **SQL placeholders are `$1..$n`, never `?`.** Queries run on a sqlx `AnyPool` that may be SQLite or Postgres; `$n` works on both. (This is a hard project rule.)
- Each statement is executed on its own — no multi-statement strings.
- Handlers return `Result<_, AppError>`; `sqlx::Error` auto-converts via `?`. `AppError::BadRequest` takes a `&'static str`.
- Auth is the `CurrentUser(pub User)` extractor; requiring it returns `401` automatically when not logged in. `user.id` is the current user's id.
- Tests use the helpers already in `tests/integration_test.rs`: `make_app()`, `register_and_login()`, `create_share_with()`, `body_string()`. Reuse them; do not re-define them.

## File structure

- **Modify** `src/handlers/share.rs` — add the export/import structs and the two handler functions. (Shares logic already lives here; keep it together.)
- **Modify** `src/lib.rs` — register the two new routes.
- **Modify** `templates/dashboard.html` — add Export link + Import button + the small import script.
- **Modify** `tests/integration_test.rs` — add the export/import integration tests.

---

## Task 1: Export endpoint

**Files:**
- Modify: `src/handlers/share.rs`
- Modify: `src/lib.rs`
- Test: `tests/integration_test.rs`

- [ ] **Step 1: Write the failing test**

Add to the end of `tests/integration_test.rs`:

```rust
#[tokio::test]
async fn test_export_returns_only_own_shares() {
    let (app, state) = make_app().await;

    let alice = register_and_login(&app, &state, "exp_alice").await;
    let bob = register_and_login(&app, &state, "exp_bob").await;

    let alice_slug = create_share_with(&app, &alice, r#"{"encrypted_payload":"alice-cipher"}"#).await;
    let _bob_slug = create_share_with(&app, &bob, r#"{"encrypted_payload":"bob-cipher"}"#).await;

    // alice exports
    let req = Request::builder()
        .uri("/api/shares/export")
        .header("cookie", alice.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"share-secret-export.json\""
    );

    let env: serde_json::Value =
        serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(env["version"].as_i64(), Some(1));
    let shares = env["shares"].as_array().unwrap();
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0]["slug"].as_str(), Some(alice_slug.as_str()));
    assert_eq!(shares[0]["encrypted_payload"].as_str(), Some("alice-cipher"));
    assert!(shares[0]["created_at"].is_string());
}

#[tokio::test]
async fn test_export_requires_auth() {
    let (app, _state) = make_app().await;
    let req = Request::builder()
        .uri("/api/shares/export")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test integration_test test_export -- --nocapture`
Expected: FAIL — route `/api/shares/export` does not exist (404, assertion fails), and `test_export_requires_auth` also fails (404 ≠ 401).

- [ ] **Step 3: Add the export structs and handler**

In `src/handlers/share.rs`, extend the axum import line to bring in `header` and `IntoResponse`. Replace:

```rust
use axum::{extract::{Path, State}, http::StatusCode, response::{Html, Redirect}, Json};
```

with:

```rust
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect},
    Json,
};
```

Then add these structs near the other share structs (after `CreateShareResponse`):

```rust
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
```

Add the handler (append to the file):

```rust
pub async fn export_shares(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, AppError> {
    let shares: Vec<ExportShare> = sqlx::query_as(
        "SELECT slug, encrypted_payload, kdf_salt, created_at FROM shares WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

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
```

- [ ] **Step 4: Register the route**

In `src/lib.rs`, add the route alongside the other `/api/shares` routes (after the `create_share` route line):

```rust
        .route("/api/shares/export", get(handlers::share::export_shares))
```

(`get` is already imported in `src/lib.rs`.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test integration_test test_export -- --nocapture`
Expected: PASS (both `test_export_returns_only_own_shares` and `test_export_requires_auth`).

- [ ] **Step 6: Commit**

```bash
git add src/handlers/share.rs src/lib.rs tests/integration_test.rs
git commit -m "feat: export own shares as downloadable JSON envelope"
```

---

## Task 2: Import endpoint

**Files:**
- Modify: `src/handlers/share.rs`
- Modify: `src/lib.rs`
- Test: `tests/integration_test.rs`

- [ ] **Step 1: Write the failing tests**

Add to the end of `tests/integration_test.rs`. The first test exercises the full round-trip (export from one account, import into a fresh instance, slug + payload + created_at + ownership preserved) and idempotency:

```rust
async fn import_envelope(
    app: &axum::Router,
    cookie: &axum::http::HeaderValue,
    envelope: &str,
) -> serde_json::Value {
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares/import")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(envelope.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    serde_json::from_str(&body_string(res.into_body()).await).unwrap()
}

#[tokio::test]
async fn test_import_roundtrip_preserves_slug_payload_created_at_and_owner() {
    // Source instance: alice creates two shares, then exports.
    let (app1, state1) = make_app().await;
    let alice = register_and_login(&app1, &state1, "rt_alice").await;
    let s1 = create_share_with(&app1, &alice, r#"{"encrypted_payload":"p1","kdf_salt":"c2FsdA=="}"#).await;
    let s2 = create_share_with(&app1, &alice, r#"{"encrypted_payload":"p2"}"#).await;

    let req = Request::builder()
        .uri("/api/shares/export")
        .header("cookie", alice.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app1.clone().oneshot(req).await.unwrap();
    let envelope_json = body_string(res.into_body()).await;
    let env: serde_json::Value = serde_json::from_str(&envelope_json).unwrap();
    // Capture original created_at for s1 from the export itself.
    let orig_created_at = env["shares"]
        .as_array().unwrap().iter()
        .find(|s| s["slug"].as_str() == Some(s1.as_str()))
        .unwrap()["created_at"].as_str().unwrap().to_string();

    // Destination instance (fresh DB = "wiped"): bob imports the envelope.
    let (app2, state2) = make_app().await;
    let bob = register_and_login(&app2, &state2, "rt_bob").await;

    let summary = import_envelope(&app2, &bob, &envelope_json).await;
    assert_eq!(summary["imported"].as_u64(), Some(2));
    assert_eq!(summary["skipped"].as_u64(), Some(0));
    assert_eq!(summary["errors"].as_u64(), Some(0));

    // Payload + salt preserved (fetch by the original slug, anonymous read).
    let req = Request::builder().uri(format!("/api/shares/{s1}")).body(Body::empty()).unwrap();
    let res = app2.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["encrypted_payload"].as_str(), Some("p1"));
    assert_eq!(v["kdf_salt"].as_str(), Some("c2FsdA=="));

    // created_at preserved (query the destination DB directly).
    let row: (String,) = sqlx::query_as("SELECT created_at FROM shares WHERE slug = $1")
        .bind(&s1)
        .fetch_one(&state2.db)
        .await
        .unwrap();
    assert_eq!(row.0, orig_created_at);

    // Ownership: imported rows belong to bob, not to a copied user_id.
    let bob_id: (i64,) = sqlx::query_as("SELECT id FROM users WHERE username = $1")
        .bind("rt_bob")
        .fetch_one(&state2.db)
        .await
        .unwrap();
    let owner: (i64,) = sqlx::query_as("SELECT user_id FROM shares WHERE slug = $1")
        .bind(&s2)
        .fetch_one(&state2.db)
        .await
        .unwrap();
    assert_eq!(owner.0, bob_id.0);

    // Idempotent re-import: nothing new, both skipped.
    let summary2 = import_envelope(&app2, &bob, &envelope_json).await;
    assert_eq!(summary2["imported"].as_u64(), Some(0));
    assert_eq!(summary2["skipped"].as_u64(), Some(2));
}

#[tokio::test]
async fn test_import_rejects_bad_version() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "ver_user").await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/shares/import")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"version":2,"shares":[]}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_import_counts_malformed_entry_as_error_without_aborting() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "mal_user").await;

    // One valid entry + one with an empty encrypted_payload.
    let envelope = r#"{"version":1,"shares":[
        {"slug":"goodslug0001","encrypted_payload":"ok","kdf_salt":null,"created_at":"2026-06-10 09:30:00"},
        {"slug":"badslug00001","encrypted_payload":"","kdf_salt":null,"created_at":"2026-06-10 09:31:00"}
    ]}"#;
    let summary = import_envelope(&app, &cookie, envelope).await;
    assert_eq!(summary["imported"].as_u64(), Some(1));
    assert_eq!(summary["errors"].as_u64(), Some(1));

    // The good one is fetchable; the bad one was never inserted.
    let req = Request::builder().uri("/api/shares/goodslug0001").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let req = Request::builder().uri("/api/shares/badslug00001").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_import_requires_auth() {
    let (app, _state) = make_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares/import")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"version":1,"shares":[]}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test integration_test test_import -- --nocapture`
Expected: FAIL — route `/api/shares/import` does not exist; auth test gets 404 instead of 401.

- [ ] **Step 3: Add the import handler**

Append to `src/handlers/share.rs`:

```rust
pub async fn import_shares(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Json(envelope): Json<ExportEnvelope>,
) -> Result<Json<ImportSummary>, AppError> {
    if envelope.version != 1 {
        return Err(AppError::BadRequest("不支持的导出版本"));
    }

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;

    for share in &envelope.shares {
        if share.encrypted_payload.is_empty() || share.slug.is_empty() {
            errors += 1;
            continue;
        }

        let exists = sqlx::query("SELECT 1 FROM shares WHERE slug = $1")
            .bind(&share.slug)
            .fetch_optional(&state.db)
            .await?
            .is_some();
        if exists {
            skipped += 1;
            continue;
        }

        // created_at is set explicitly to preserve the original timestamp
        // (the create path lets the DB default it; import must not).
        sqlx::query(
            "INSERT INTO shares (user_id, slug, encrypted_payload, kdf_salt, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user.id)
        .bind(&share.slug)
        .bind(&share.encrypted_payload)
        .bind(&share.kdf_salt)
        .bind(&share.created_at)
        .execute(&state.db)
        .await?;
        imported += 1;
    }

    Ok(Json(ImportSummary { imported, skipped, errors }))
}
```

- [ ] **Step 4: Register the route**

In `src/lib.rs`, add after the export route:

```rust
        .route("/api/shares/import", post(handlers::share::import_shares))
```

(`post` is already imported in `src/lib.rs`.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test integration_test test_import -- --nocapture`
Expected: PASS (all four import tests).

- [ ] **Step 6: Run the whole suite for regressions**

Run: `cargo test`
Expected: PASS — all existing tests plus the new export/import tests.

- [ ] **Step 7: Commit**

```bash
git add src/handlers/share.rs src/lib.rs tests/integration_test.rs
git commit -m "feat: import shares from a JSON envelope (slug-preserving, skip collisions)"
```

---

## Task 3: Dashboard Export / Import UI

**Files:**
- Modify: `templates/dashboard.html`
- Test: `tests/integration_test.rs`

- [ ] **Step 1: Write the failing test**

This asserts the dashboard renders the Export/Import controls so the wiring isn't silently dropped. Add to the end of `tests/integration_test.rs`:

```rust
#[tokio::test]
async fn test_dashboard_shows_export_import_controls() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "ui_user").await;

    let req = Request::builder()
        .uri("/dashboard")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_string(res.into_body()).await;
    assert!(body.contains("/api/shares/export"), "export link missing: {body}");
    assert!(body.contains("导入"), "import control missing: {body}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test integration_test test_dashboard_shows_export_import_controls -- --nocapture`
Expected: FAIL — body does not contain `/api/shares/export`.

- [ ] **Step 3: Add the controls to the template**

In `templates/dashboard.html`, replace this line:

```html
<p><a href="/shares/new">创建新分享</a></p>
```

with:

```html
<p style="display: flex; gap: 1rem; flex-wrap: wrap; align-items: center;">
    <a href="/shares/new">创建新分享</a>
    <a href="/api/shares/export" download="share-secret-export.json">导出全部</a>
    <button type="button" id="import-btn" style="width: auto;">导入</button>
    <input type="file" id="import-file" accept="application/json,.json" style="display: none;">
</p>
<p id="import-result" style="color: #555; font-size: 0.875rem;"></p>
```

Then add this script at the very end of the `{% block content %}` (immediately before `{% endblock %}`):

```html
<script>
(function () {
    const btn = document.getElementById('import-btn');
    const fileInput = document.getElementById('import-file');
    const result = document.getElementById('import-result');

    btn.addEventListener('click', () => fileInput.click());

    fileInput.addEventListener('change', async () => {
        const file = fileInput.files[0];
        if (!file) return;
        result.textContent = '导入中…';
        try {
            const envelope = JSON.parse(await file.text());
            const res = await fetch('/api/shares/import', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(envelope)
            });
            if (!res.ok) {
                result.textContent = '导入失败：' + (await res.text());
                return;
            }
            const s = await res.json();
            result.textContent = `导入完成：新增 ${s.imported}，跳过 ${s.skipped}，错误 ${s.errors}`;
            setTimeout(() => location.reload(), 800);
        } catch (e) {
            result.textContent = '导入失败：文件不是有效的 JSON';
        } finally {
            fileInput.value = '';
        }
    });
})();
</script>
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test integration_test test_dashboard_shows_export_import_controls -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Manual smoke check (optional but recommended)**

Run the server and exercise the round-trip in a browser:

```bash
SECURE_COOKIES=false cargo run
```

Then: register/login → create a share → click **导出全部** (a `share-secret-export.json` downloads) → click **导入** and pick that file → confirm the result line shows `新增 0，跳过 1` (the share already exists) and the page reloads.

- [ ] **Step 6: Commit**

```bash
git add templates/dashboard.html tests/integration_test.rs
git commit -m "feat: dashboard export link and import button"
```

---

## Done criteria

- `cargo test` is green, including the new export/import tests.
- A user can download all their shares as JSON and re-import them; re-importing the same file is idempotent (everything skipped), and imported shares keep their original slug and `created_at` while belonging to the importing account.
