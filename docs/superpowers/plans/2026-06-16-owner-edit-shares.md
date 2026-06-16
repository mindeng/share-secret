# Owner Editing for Shares — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user who created a share edit its content and view-password inline on the share's view page, re-encrypting client-side and saving under the same slug.

**Architecture:** The server stays zero-knowledge — it only stores ciphertext. The payload API gains an `is_owner` flag (from the session) so the view page can reveal an Edit button. A new owner-only `POST /api/shares/:slug/update` endpoint overwrites `encrypted_payload`/`kdf_salt`. All decryption and re-encryption happen in the browser, reusing the key already in memory after the initial decrypt.

**Tech Stack:** Rust (axum 0.7, sqlx/sqlite, askama), tower-sessions, vanilla JS with WebCrypto (AES-GCM / PBKDF2).

---

## File Structure

- `src/auth.rs` — add `current_user_id(&Session) -> Option<i64>` helper (read session without requiring auth).
- `src/handlers/share.rs` — add `is_owner` to `SharePayloadResponse`; read session in `get_share_payload`; add `UpdateShareRequest` + `update_share` handler.
- `src/lib.rs` — register `POST /api/shares/:slug/update`.
- `static/crypto.js` — add `updateShare(slug, payload, password, existing)`.
- `templates/view_share.html` — Edit button + inline edit form + save logic; track mode/key in memory.
- `templates/dashboard.html` — make the slug a link to the view page.
- `tests/integration_test.rs` — cover `is_owner` and the update endpoint.

---

## Task 1: `is_owner` flag in the payload API

**Files:**
- Modify: `src/auth.rs`
- Modify: `src/handlers/share.rs:18-22` (`SharePayloadResponse`) and `src/handlers/share.rs:100-113` (`get_share_payload`)
- Test: `tests/integration_test.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/integration_test.rs`:

```rust
#[tokio::test]
async fn test_is_owner_flag() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "owner1").await;

    // owner creates a share
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"orig"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = body_string(res.into_body()).await;
    let slug = serde_json::from_str::<serde_json::Value>(&body).unwrap()["slug"]
        .as_str().unwrap().to_string();

    // owner fetch -> is_owner true
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["is_owner"].as_bool(), Some(true));

    // anonymous fetch -> is_owner false
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["is_owner"].as_bool(), Some(false));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test integration_test test_is_owner_flag`
Expected: FAIL — response JSON has no `is_owner` field (assertion `Some(true)` fails, got `None`).

- [ ] **Step 3: Add the session helper to `src/auth.rs`**

Add after `logout_user` (around line 45), reusing the existing private `USER_ID_KEY`:

```rust
/// 从 session 读取当前用户 id（未登录返回 None，永不报错）。
pub async fn current_user_id(session: &Session) -> Option<i64> {
    session.get::<i64>(USER_ID_KEY).await.ok().flatten()
}
```

- [ ] **Step 4: Add `is_owner` to the response struct**

In `src/handlers/share.rs`, change `SharePayloadResponse` (lines 18-22):

```rust
#[derive(Serialize)]
pub struct SharePayloadResponse {
    pub encrypted_payload: String,
    pub kdf_salt: Option<String>,
    pub is_owner: bool,
}
```

- [ ] **Step 5: Read the session in `get_share_payload`**

Replace `get_share_payload` (lines 100-113) with:

```rust
pub async fn get_share_payload(
    State(state): State<Arc<AppState>>,
    session: Session,
    Path(slug): Path<String>,
) -> Result<Json<SharePayloadResponse>, AppError> {
    let row: Option<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT user_id, encrypted_payload, kdf_salt FROM shares WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?;

    match row {
        Some((owner_id, encrypted_payload, kdf_salt)) => {
            let is_owner = crate::auth::current_user_id(&session).await == Some(owner_id);
            Ok(Json(SharePayloadResponse { encrypted_payload, kdf_salt, is_owner }))
        }
        None => Err(AppError::NotFound),
    }
}
```

Add the import at the top of `src/handlers/share.rs` (with the other `use` lines):

```rust
use tower_sessions::Session;
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test integration_test`
Expected: PASS — `test_is_owner_flag` plus all existing tests (existing tests ignore the new field).

- [ ] **Step 7: Commit**

```bash
git add src/auth.rs src/handlers/share.rs tests/integration_test.rs
git commit -m "feat: return is_owner flag from share payload API"
```

---

## Task 2: Owner-only update endpoint

**Files:**
- Modify: `src/handlers/share.rs` (add `UpdateShareRequest` + `update_share`)
- Modify: `src/lib.rs:60-63` (add route)
- Test: `tests/integration_test.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration_test.rs`:

```rust
async fn create_share_with(app: &axum::Router, cookie: &axum::http::HeaderValue, body: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = body_string(res.into_body()).await;
    serde_json::from_str::<serde_json::Value>(&body).unwrap()["slug"]
        .as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_owner_can_update_share() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "upowner").await;
    let slug = create_share_with(&app, &cookie, r#"{"encrypted_payload":"orig"}"#).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"updated"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // fetch confirms ciphertext changed
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["encrypted_payload"].as_str(), Some("updated"));
}

#[tokio::test]
async fn test_non_owner_cannot_update_share() {
    let (app, state) = make_app().await;
    let owner = register_and_login(&app, &state, "realowner").await;
    let slug = create_share_with(&app, &owner, r#"{"encrypted_payload":"orig"}"#).await;
    let attacker = register_and_login(&app, &state, "attacker").await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .header("cookie", attacker.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"hacked"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_update_requires_auth() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "needauth").await;
    let slug = create_share_with(&app, &cookie, r#"{"encrypted_payload":"orig"}"#).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"encrypted_payload":"x"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_update_missing_slug_forbidden() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "ghostupd").await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/shares/nosuchslug/update")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"x"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test integration_test test_owner_can_update_share test_non_owner_cannot_update_share test_update_requires_auth test_update_missing_slug_forbidden`
Expected: FAIL — route `/api/shares/:slug/update` does not exist (404 instead of expected statuses).

- [ ] **Step 3: Add the request struct and handler in `src/handlers/share.rs`**

Add the struct after `CreateShareResponse` (around line 34):

```rust
#[derive(Debug, Deserialize)]
pub struct UpdateShareRequest {
    pub encrypted_payload: String,
    #[serde(default)]
    pub kdf_salt: Option<String>,
}
```

Add the handler after `create_share` (around line 76):

```rust
pub async fn update_share(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Path(slug): Path<String>,
    Json(req): Json<UpdateShareRequest>,
) -> Result<StatusCode, AppError> {
    if req.encrypted_payload.is_empty() {
        return Err(AppError::BadRequest("加密内容不能为空"));
    }

    let result = sqlx::query(
        "UPDATE shares SET encrypted_payload = ?, kdf_salt = ? WHERE slug = ? AND user_id = ?",
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
```

Add `StatusCode` to the axum imports at the top of `src/handlers/share.rs`. Change line 6 from:

```rust
use axum::{extract::{Path, State}, response::{Html, Redirect}, Json};
```

to:

```rust
use axum::{extract::{Path, State}, http::StatusCode, response::{Html, Redirect}, Json};
```

- [ ] **Step 4: Register the route in `src/lib.rs`**

After the existing `/api/shares/:slug` GET route (line 63), add:

```rust
        .route("/api/shares/:slug/update", post(handlers::share::update_share))
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test integration_test`
Expected: PASS — all four new update tests plus every existing test.

- [ ] **Step 6: Commit**

```bash
git add src/handlers/share.rs src/lib.rs tests/integration_test.rs
git commit -m "feat: add owner-only share update endpoint"
```

---

## Task 3: Client `updateShare` helper

**Files:**
- Modify: `static/crypto.js` (append new function)

No automated JS test harness exists in this project; verify by `cargo build` (file is static) and manual check in Task 4. This task only adds the helper used by the template.

- [ ] **Step 1: Append `updateShare` to `static/crypto.js`**

Add at the end of the file:

```javascript
// 编辑已有分享：
// - password 非空 → 切换/设置为密码模式（新 salt + 派生密钥）
// - password 为空且原本是链接模式 → 复用原密钥（已分享的链接继续有效）
// - password 为空且原本是密码模式 → 切换为链接模式（生成新密钥）
// existing: { mode: 'link' | 'password', key: rawKeyB64 | null }
async function updateShare(slug, payload, password, existing) {
    let key = null;
    let kdfSalt = null;
    let cryptoKey;

    if (password) {
        kdfSalt = generateSalt();
        cryptoKey = await deriveKeyFromPassword(password, kdfSalt);
    } else if (existing.mode === 'link' && existing.key) {
        key = existing.key;
        cryptoKey = await importRawKey(key);
    } else {
        key = generateKey();
        cryptoKey = await importRawKey(key);
    }

    const encryptedPayload = await encryptWithKey(cryptoKey, payload);

    const res = await fetch(`/api/shares/${slug}/update`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ encrypted_payload: encryptedPayload, kdf_salt: kdfSalt })
    });

    if (!res.ok) {
        const text = await res.text();
        throw new Error(text || '更新失败');
    }

    return { key, passwordProtected: !!password };
}
```

- [ ] **Step 2: Verify the project still builds**

Run: `cargo build`
Expected: PASS (static file change; no Rust impact, but confirms nothing else broke).

- [ ] **Step 3: Commit**

```bash
git add static/crypto.js
git commit -m "feat: add updateShare helper for editing shares"
```

---

## Task 4: Inline edit UI on the view page

**Files:**
- Modify: `templates/view_share.html` (full rewrite — adds Edit button, edit form, save logic, in-memory mode/key tracking)

Note: field rows are built with `document.createElement` (no `innerHTML`), matching the safe pattern of setting user values via `.value` only.

- [ ] **Step 1: Replace `templates/view_share.html` with the full content below**

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
    <button type="button" id="edit-btn" style="display:none; width:auto; margin-top:1rem;">编辑</button>
</div>

<div id="password-prompt" style="display:none; background:white; padding:1rem; border-radius:8px;">
    <p>此分享受密码保护，请输入查看密码：</p>
    <label><input type="password" id="view-password" autocomplete="off"></label>
    <button id="unlock-btn">解锁</button>
    <p id="password-error" style="color:red; display:none;">密码错误，请重试</p>
</div>

<form id="edit-form" style="display:none; margin-top:1rem;">
    <label>标题 <input type="text" id="edit-title" required></label>
    <div id="edit-fields"></div>
    <button type="button" id="edit-add-field" style="margin-bottom:1rem;">添加字段</button>
    <label>查看密码（可选，留空则用带密钥的链接分享）
        <input type="password" id="edit-view-password" autocomplete="new-password">
    </label>
    <button type="submit">保存修改</button>
    <button type="button" id="edit-cancel" style="width:auto;">取消</button>
</form>

<div id="edit-result" style="display:none; margin-top:1rem; background:white; padding:1rem; border-radius:8px;">
    <p>已保存。分享链接：</p>
    <input type="text" id="edit-link" readonly>
    <button type="button" id="edit-copy-link">复制链接</button>
    <p id="edit-password-hint" style="display:none; color:#b45309; margin-top:0.75rem;">
        ⚠ 此分享受密码保护。请通过其它安全渠道把查看密码告诉对方，链接本身不含密钥。
    </p>
</div>

<script src="/static/crypto.js"></script>
<script>
    const slug = window.location.pathname.split('/').pop();
    let currentPayload = null;
    let currentMode = null;   // 'link' | 'password'
    let currentKey = null;    // rawKeyB64 when link mode
    let isOwner = false;

    function showError(msg) {
        document.getElementById('loading').style.display = 'none';
        const el = document.getElementById('error');
        el.textContent = msg;
        el.style.display = 'block';
    }

    function renderPayload(payload) {
        currentPayload = payload;
        document.getElementById('loading').style.display = 'none';
        document.getElementById('password-prompt').style.display = 'none';
        document.getElementById('edit-form').style.display = 'none';
        document.getElementById('content').style.display = 'block';
        document.getElementById('title').textContent = payload.title;

        const tbody = document.getElementById('fields');
        tbody.replaceChildren();
        (payload.fields || []).forEach(field => {
            const tr = document.createElement('tr');

            const labelTd = document.createElement('td');
            labelTd.textContent = field.label;

            const valueTd = document.createElement('td');
            const valueInput = document.createElement('input');
            valueInput.type = 'text';
            valueInput.value = field.value;
            valueInput.readOnly = true;
            valueTd.appendChild(valueInput);

            const btnTd = document.createElement('td');
            const copyBtn = document.createElement('button');
            copyBtn.className = 'copy-btn';
            copyBtn.textContent = '复制';
            copyBtn.addEventListener('click', () => {
                navigator.clipboard.writeText(valueInput.value).then(() => {
                    copyBtn.textContent = '已复制';
                    setTimeout(() => { copyBtn.textContent = '复制'; }, 1500);
                });
            });
            btnTd.appendChild(copyBtn);

            tr.appendChild(labelTd);
            tr.appendChild(valueTd);
            tr.appendChild(btnTd);
            tbody.appendChild(tr);
        });

        document.getElementById('edit-btn').style.display = isOwner ? 'inline-block' : 'none';
    }

    // 用 DOM API 构造可编辑字段行（不使用 innerHTML，避免 XSS）
    function addEditFieldRow(label, value) {
        const div = document.createElement('div');
        div.className = 'edit-field';

        const labelWrap = document.createElement('label');
        labelWrap.append('字段名 ');
        const labelInput = document.createElement('input');
        labelInput.type = 'text';
        labelInput.className = 'edit-label';
        labelInput.required = true;
        labelInput.value = label || '';
        labelWrap.appendChild(labelInput);

        const valueWrap = document.createElement('label');
        valueWrap.append('值 ');
        const valueInput = document.createElement('input');
        valueInput.type = 'text';
        valueInput.className = 'edit-value';
        valueInput.required = true;
        valueInput.value = value || '';
        valueWrap.appendChild(valueInput);

        div.appendChild(labelWrap);
        div.appendChild(valueWrap);
        document.getElementById('edit-fields').appendChild(div);
    }

    function openEditForm() {
        document.getElementById('content').style.display = 'none';
        document.getElementById('edit-result').style.display = 'none';
        document.getElementById('edit-title').value = currentPayload.title || '';
        const ef = document.getElementById('edit-fields');
        ef.replaceChildren();
        const fields = currentPayload.fields || [];
        if (fields.length === 0) {
            addEditFieldRow('', '');
        } else {
            fields.forEach(f => addEditFieldRow(f.label, f.value));
        }
        document.getElementById('edit-view-password').value = '';
        document.getElementById('edit-form').style.display = 'block';
    }

    document.getElementById('edit-btn').addEventListener('click', openEditForm);
    document.getElementById('edit-add-field').addEventListener('click', () => addEditFieldRow('', ''));
    document.getElementById('edit-cancel').addEventListener('click', () => {
        document.getElementById('edit-form').style.display = 'none';
        document.getElementById('content').style.display = 'block';
    });

    document.getElementById('edit-form').addEventListener('submit', async (e) => {
        e.preventDefault();
        const title = document.getElementById('edit-title').value;
        const password = document.getElementById('edit-view-password').value;
        const fields = [];
        document.querySelectorAll('#edit-fields .edit-field').forEach(el => {
            fields.push({
                label: el.querySelector('.edit-label').value,
                value: el.querySelector('.edit-value').value,
            });
        });
        const payload = { title, fields };

        try {
            const { key, passwordProtected } = await updateShare(
                slug, payload, password, { mode: currentMode, key: currentKey }
            );

            if (passwordProtected) {
                currentMode = 'password';
                currentKey = null;
                history.replaceState(null, '', `/s/${slug}`);
            } else {
                currentMode = 'link';
                currentKey = key;
                history.replaceState(null, '', `/s/${slug}#key=${key}`);
            }

            renderPayload(payload);

            const link = passwordProtected
                ? `${window.location.origin}/s/${slug}`
                : `${window.location.origin}/s/${slug}#key=${key}`;
            document.getElementById('edit-link').value = link;
            document.getElementById('edit-password-hint').style.display = passwordProtected ? 'block' : 'none';
            document.getElementById('edit-result').style.display = 'block';
        } catch (err) {
            alert('保存失败：' + err.message);
        }
    });

    document.getElementById('edit-copy-link').addEventListener('click', () => {
        const input = document.getElementById('edit-link');
        input.select();
        navigator.clipboard.writeText(input.value).then(() => {
            const btn = document.getElementById('edit-copy-link');
            btn.textContent = '已复制';
            setTimeout(() => btn.textContent = '复制链接', 1500);
        });
    });

    (async () => {
        let data;
        try {
            const res = await fetch(`/api/shares/${slug}`);
            if (!res.ok) throw new Error('分享不存在');
            data = await res.json();
        } catch (e) {
            showError('分享不存在或已被删除');
            return;
        }

        isOwner = !!data.is_owner;

        if (data.kdf_salt) {
            document.getElementById('loading').style.display = 'none';
            document.getElementById('password-prompt').style.display = 'block';

            const unlock = async () => {
                const password = document.getElementById('view-password').value;
                document.getElementById('password-error').style.display = 'none';
                try {
                    const cryptoKey = await deriveKeyFromPassword(password, data.kdf_salt);
                    const payload = await decryptWithKey(cryptoKey, data.encrypted_payload);
                    currentMode = 'password';
                    currentKey = null;
                    renderPayload(payload);
                } catch (e) {
                    document.getElementById('password-error').style.display = 'block';
                }
            };

            document.getElementById('unlock-btn').addEventListener('click', unlock);
            document.getElementById('view-password').addEventListener('keydown', (ev) => {
                if (ev.key === 'Enter') unlock();
            });
            return;
        }

        const keyMatch = window.location.hash.match(/^#key=(.+)$/);
        if (!keyMatch) {
            showError('链接不完整，缺少解密密钥');
            return;
        }
        try {
            const cryptoKey = await importRawKey(keyMatch[1]);
            const payload = await decryptWithKey(cryptoKey, data.encrypted_payload);
            currentMode = 'link';
            currentKey = keyMatch[1];
            renderPayload(payload);
        } catch (e) {
            showError('无法解密，请检查链接是否完整');
        }
    })();
</script>
{% endblock %}
```

- [ ] **Step 2: Build and verify the template compiles**

Run: `cargo build`
Expected: PASS — askama compiles `view_share.html` at build time, so a template syntax error would fail here.

- [ ] **Step 3: Manual verification**

Run: `SECURE_COOKIES=false cargo run` then in a browser:
1. Register + log in, create a link-mode share (no password). Copy the `/s/<slug>#key=...` link.
2. Open that link while logged in → 编辑 button appears. Click it, change the title and a field value, leave password empty, 保存修改.
3. Confirm the displayed content updates and the shown link is unchanged (`#key=` identical). Reopen the original link in a fresh tab → shows updated content.
4. While logged in, edit again and this time set a view-password, 保存修改. Confirm the result box shows the password hint and the URL bar drops the `#key=` fragment. Open `/s/<slug>` in a private window → password prompt; entering the password shows updated content.
5. Open the share's view URL while logged out → no 编辑 button.

Expected: all steps behave as described.

- [ ] **Step 4: Commit**

```bash
git add templates/view_share.html
git commit -m "feat: inline edit form for share owners on view page"
```

---

## Task 5: Dashboard link to view page

**Files:**
- Modify: `templates/dashboard.html:15`

- [ ] **Step 1: Make the slug a clickable link**

In `templates/dashboard.html`, change line 15 from:

```html
        <code style="word-break: break-all;">/s/{{ share.slug }}</code>
```

to:

```html
        <a href="/s/{{ share.slug }}" style="word-break: break-all;"><code>/s/{{ share.slug }}</code></a>
```

- [ ] **Step 2: Build and verify the template compiles**

Run: `cargo build`
Expected: PASS.

- [ ] **Step 3: Manual verification**

Run: `SECURE_COOKIES=false cargo run`, log in, open `/dashboard`. Confirm each share's `/s/<slug>` is a link that opens the view page. (Link-mode shares will show "链接不完整，缺少解密密钥" without the `#key=` fragment — expected; password-mode shares show the password prompt.)

- [ ] **Step 4: Commit**

```bash
git add templates/dashboard.html
git commit -m "feat: link dashboard shares to their view page"
```

---

## Final verification

- [ ] Run the full suite: `cargo test`
  Expected: all tests pass, including the five new ones (`test_is_owner_flag`, `test_owner_can_update_share`, `test_non_owner_cannot_update_share`, `test_update_requires_auth`, `test_update_missing_slug_forbidden`).
- [ ] Run `cargo build --release` to confirm a clean production build.
