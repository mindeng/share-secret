# View Password Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为分享增加可选的「查看密码」：创建者可设一个密码，访问链接的人必须输入正确密码才能解密查看；不设密码时保持现有 `#key=` 链接行为。

**Architecture:** 维持零知识架构——服务端永远拿不到明文。密码模式下，AES-GCM 密钥由密码经 PBKDF2(SHA-256, 200000 轮) 派生，随机 16 字节 salt 存服务端（salt 非机密）。链接里不再带密钥（`/s/slug`），对方需通过其它渠道得知密码。AES-GCM 的认证标签天然用于校验密码是否正确：密码错误时解密抛异常，前端据此提示「密码错误」。非密码模式不变：随机 32 字节密钥放 URL fragment。

**Tech Stack:** 后端 `axum` + `sqlx`(sqlite)；前端 Web Crypto API（`crypto.subtle.deriveKey` PBKDF2 + AES-GCM）。

---

## Data Model Change

`shares` 表新增一列 `kdf_salt TEXT`（可空）：
- `NULL` → 链接模式（密钥在 fragment），现有行为。
- 非空 → 密码模式（密钥由密码 + 此 salt 派生）。

## File Structure

| 文件 | 职责 | 改动 |
|---|---|---|
| `src/db.rs` | 建表 / 迁移 | CREATE 语句加列；`init_db` 加幂等 `ALTER TABLE` 迁移旧库 |
| `src/models.rs` | `Share` 结构 | 加 `kdf_salt: Option<String>` |
| `src/handlers/share.rs` | 创建/读取分享 API | 请求/响应结构加 `kdf_salt`；INSERT/SELECT 带该列 |
| `src/handlers/dashboard.rs` | dashboard 查询 | SELECT 增加 `kdf_salt` 列以匹配 `Share` |
| `tests/integration_test.rs` | 集成测试 | 加密码分享往返测试；链接模式断言 `kdf_salt` 为 null |
| `static/crypto.js` | 前端加解密 | 重构出 `importRawKey`/`deriveKeyFromPassword`/`encryptWithKey`/`decryptWithKey`；`createShare(payload, password)` |
| `templates/new_share.html` | 创建页 | 加可选密码输入；按模式拼接链接 |
| `templates/view_share.html` | 查看页 | 有 salt → 密码表单 + 派生解密；无 salt → fragment 密钥；改用安全 DOM 构造 |

---

## Task 1: 后端持久化与返回 kdf_salt

**Files:**
- Modify: `src/db.rs`
- Modify: `src/models.rs`
- Modify: `src/handlers/share.rs`
- Modify: `src/handlers/dashboard.rs`
- Test: `tests/integration_test.rs`

- [ ] **Step 1: 在 `tests/integration_test.rs` 末尾加一个测试 helper 和失败测试**

在文件末尾追加：

```rust
async fn register_and_login(app: &axum::Router, user: &str) -> axum::http::HeaderValue {
    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username={user}&password=secret")))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username={user}&password=secret")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    res.headers().get("set-cookie").unwrap().clone()
}

#[tokio::test]
async fn test_password_protected_share_roundtrips_salt() {
    let db = init_db_memory().await;
    let app = build_app(db);
    let cookie = register_and_login(&app, "carol").await;

    let payload = r#"{"encrypted_payload":"cipher","kdf_salt":"c2FsdHNhbHQ="}"#;
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(payload))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_string(res.into_body()).await;
    let slug = serde_json::from_str::<serde_json::Value>(&body).unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_string();

    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_string(res.into_body()).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["encrypted_payload"].as_str(), Some("cipher"));
    assert_eq!(v["kdf_salt"].as_str(), Some("c2FsdHNhbHQ="));
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test test_password_protected_share_roundtrips_salt 2>&1 | tail -20`
Expected: 编译失败或断言失败（`CreateShareRequest` 还没有 `kdf_salt` 字段 / 响应没有该字段）。

- [ ] **Step 3: `src/db.rs` 建表语句加列，并在 `init_db` 加幂等迁移**

把 `init_db` 里 `shares` 的 CREATE 语句改成包含 `kdf_salt TEXT`：

```rust
        CREATE TABLE IF NOT EXISTS shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
```

在 `init_db` 中 `.execute(&pool).await.expect("failed to create tables");` 之后、`pool` 之前，加一行幂等迁移（旧库无此列时补上；新库已存在则忽略报错）：

```rust
    // 迁移旧数据库：若 kdf_salt 列不存在则补上（已存在会报错，忽略即可）
    let _ = sqlx::query("ALTER TABLE shares ADD COLUMN kdf_salt TEXT")
        .execute(&pool)
        .await;
```

同样把 `init_db_memory` 里 `shares` 的 CREATE 语句加上 `kdf_salt TEXT`（位置同上，`encrypted_payload TEXT NOT NULL,` 之后）：

```rust
        CREATE TABLE shares (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            slug TEXT UNIQUE NOT NULL,
            encrypted_payload TEXT NOT NULL,
            kdf_salt TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users(id)
        );
```

- [ ] **Step 4: `src/models.rs` 的 `Share` 加字段**

把 `Share` 结构改为（在 `encrypted_payload` 之后加 `kdf_salt`）：

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Share {
    pub id: i64,
    pub user_id: i64,
    pub slug: String,
    pub encrypted_payload: String,
    pub kdf_salt: Option<String>,
    pub created_at: String,
}
```

- [ ] **Step 5: `src/handlers/dashboard.rs` 的 SELECT 增加 kdf_salt 列**

把 dashboard 查询的 SQL 改为（否则 `FromRow` 找不到 `kdf_salt` 列会报错）：

```rust
    let shares: Vec<Share> = sqlx::query_as(
        "SELECT id, user_id, slug, encrypted_payload, kdf_salt, created_at FROM shares WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
```

- [ ] **Step 6: `src/handlers/share.rs` 的请求/响应结构与 SQL 加 kdf_salt**

把 `SharePayloadResponse` 与 `CreateShareRequest` 改为：

```rust
#[derive(Serialize)]
pub struct SharePayloadResponse {
    pub encrypted_payload: String,
    pub kdf_salt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateShareRequest {
    pub encrypted_payload: String,
    #[serde(default)]
    pub kdf_salt: Option<String>,
}
```

把 `create_share` 里的 INSERT 改成带 `kdf_salt`：

```rust
    sqlx::query("INSERT INTO shares (user_id, slug, encrypted_payload, kdf_salt) VALUES (?, ?, ?, ?)")
        .bind(user.id)
        .bind(&slug)
        .bind(&req.encrypted_payload)
        .bind(&req.kdf_salt)
        .execute(&state.db)
        .await?;
```

把 `get_share_payload` 改成查询并返回 `kdf_salt`：

```rust
pub async fn get_share_payload(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<Json<SharePayloadResponse>, AppError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT encrypted_payload, kdf_salt FROM shares WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?;

    match row {
        Some((encrypted_payload, kdf_salt)) => {
            Ok(Json(SharePayloadResponse { encrypted_payload, kdf_salt }))
        }
        None => Err(AppError::NotFound),
    }
}
```

- [ ] **Step 7: 更新现有链接模式测试，断言 kdf_salt 为 null**

在 `tests/integration_test.rs` 的 `test_create_and_fetch_share` 中，最后一段 `assert_eq!(fetched["encrypted_payload"]...` 之后加一行：

```rust
    assert!(fetched["kdf_salt"].is_null());
```

- [ ] **Step 8: 运行全部测试**

Run: `cargo test 2>&1 | tail -10`
Expected: 全部通过（含新 `test_password_protected_share_roundtrips_salt`）。

- [ ] **Step 9: clippy 与 check**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 | tail -5`
Expected: No issues found

- [ ] **Step 10: Commit**

```bash
git add src/db.rs src/models.rs src/handlers/share.rs src/handlers/dashboard.rs tests/integration_test.rs
git commit -m "feat: persist optional kdf_salt for password-protected shares"
```

---

## Task 2: 前端 crypto.js 支持密码派生

**Files:**
- Modify: `static/crypto.js`

> 说明：本项目无 JS 测试框架，crypto.js 的验证放在 Task 5 的浏览器手动验证。本任务只做纯函数重构 + 新增密码模式，不破坏 Rust 测试（保持 `cargo test` 绿）。

- [ ] **Step 1: 用以下完整内容覆盖 `static/crypto.js`**

```javascript
function bytesToB64(bytes) {
    return btoa(String.fromCharCode(...bytes));
}

function b64ToBytes(b64) {
    return Uint8Array.from(atob(b64), c => c.charCodeAt(0));
}

async function importRawKey(rawKeyB64) {
    return crypto.subtle.importKey(
        'raw',
        b64ToBytes(rawKeyB64),
        { name: 'AES-GCM' },
        false,
        ['encrypt', 'decrypt']
    );
}

async function deriveKeyFromPassword(password, saltB64) {
    const baseKey = await crypto.subtle.importKey(
        'raw',
        new TextEncoder().encode(password),
        'PBKDF2',
        false,
        ['deriveKey']
    );
    return crypto.subtle.deriveKey(
        { name: 'PBKDF2', salt: b64ToBytes(saltB64), iterations: 200000, hash: 'SHA-256' },
        baseKey,
        { name: 'AES-GCM', length: 256 },
        false,
        ['encrypt', 'decrypt']
    );
}

function generateKey() {
    return bytesToB64(crypto.getRandomValues(new Uint8Array(32)));
}

function generateSalt() {
    return bytesToB64(crypto.getRandomValues(new Uint8Array(16)));
}

async function encryptWithKey(cryptoKey, payload) {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const encoded = new TextEncoder().encode(JSON.stringify(payload));
    const ciphertext = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, cryptoKey, encoded);
    const combined = new Uint8Array(iv.length + ciphertext.byteLength);
    combined.set(iv);
    combined.set(new Uint8Array(ciphertext), iv.length);
    return bytesToB64(combined);
}

async function decryptWithKey(cryptoKey, encrypted) {
    const combined = b64ToBytes(encrypted);
    const iv = combined.slice(0, 12);
    const ciphertext = combined.slice(12);
    const decrypted = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, cryptoKey, ciphertext);
    return JSON.parse(new TextDecoder().decode(decrypted));
}

// password 为空/undefined → 链接模式（随机密钥放 fragment）
// password 非空 → 密码模式（密钥由密码 + salt 派生，链接不含密钥）
async function createShare(payload, password) {
    let key = null;
    let kdfSalt = null;
    let cryptoKey;

    if (password) {
        kdfSalt = generateSalt();
        cryptoKey = await deriveKeyFromPassword(password, kdfSalt);
    } else {
        key = generateKey();
        cryptoKey = await importRawKey(key);
    }

    const encryptedPayload = await encryptWithKey(cryptoKey, payload);

    const res = await fetch('/api/shares', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ encrypted_payload: encryptedPayload, kdf_salt: kdfSalt })
    });

    if (!res.ok) {
        const text = await res.text();
        throw new Error(text || '创建失败');
    }

    const { slug } = await res.json();
    return { slug, key, passwordProtected: !!password };
}
```

- [ ] **Step 2: 确认 Rust 测试不受影响**

Run: `cargo test 2>&1 | tail -5`
Expected: 全部通过（纯前端改动，不影响后端）。

- [ ] **Step 3: Commit**

```bash
git add static/crypto.js
git commit -m "feat: add password-derived key support in crypto.js"
```

---

## Task 3: 创建页支持设置查看密码

**Files:**
- Modify: `templates/new_share.html`

- [ ] **Step 1: 在表单中「添加字段」按钮之后、「创建分享」按钮之前，加入可选密码输入**

把现有：

```html
    <button type="button" id="add-field" style="margin-bottom: 1rem;">添加字段</button>

    <button type="submit">创建分享</button>
```

替换为：

```html
    <button type="button" id="add-field" style="margin-bottom: 1rem;">添加字段</button>

    <label>查看密码（可选，留空则用带密钥的链接分享）
        <input type="password" id="view-password" autocomplete="new-password">
    </label>

    <button type="submit">创建分享</button>
```

- [ ] **Step 2: 在结果区加入密码提示元素**

把现有结果区：

```html
<div id="result" style="display:none; margin-top: 1rem; background: white; padding: 1rem; border-radius: 8px;">
    <p>分享链接：</p>
    <input type="text" id="share-link" readonly>
    <button id="copy-link">复制链接</button>
</div>
```

替换为：

```html
<div id="result" style="display:none; margin-top: 1rem; background: white; padding: 1rem; border-radius: 8px;">
    <p>分享链接：</p>
    <input type="text" id="share-link" readonly>
    <button id="copy-link">复制链接</button>
    <p id="password-hint" style="display:none; color:#b45309; margin-top:0.75rem;">
        ⚠ 此分享受密码保护。请通过其它安全渠道把查看密码告诉对方，链接本身不含密钥。
    </p>
</div>
```

- [ ] **Step 3: 更新提交逻辑，读取密码并按模式拼接链接**

把 `<script>` 中的 submit 处理函数：

```javascript
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
        const { slug, key } = await createShare(payload);

        const fullUrl = `${window.location.origin}/s/${slug}#key=${key}`;
        document.getElementById('share-link').value = fullUrl;
        document.getElementById('result').style.display = 'block';
    });
```

替换为：

```javascript
    document.getElementById('share-form').addEventListener('submit', async (e) => {
        e.preventDefault();
        const title = document.getElementById('title').value;
        const password = document.getElementById('view-password').value;
        const fields = [];
        document.querySelectorAll('.field').forEach(el => {
            const label = el.querySelector('.label').value;
            const value = el.querySelector('.value').value;
            fields.push({ label, value });
        });

        const payload = { title, fields };
        const { slug, key, passwordProtected } = await createShare(payload, password);

        const fullUrl = passwordProtected
            ? `${window.location.origin}/s/${slug}`
            : `${window.location.origin}/s/${slug}#key=${key}`;
        document.getElementById('share-link').value = fullUrl;
        document.getElementById('password-hint').style.display = passwordProtected ? 'block' : 'none';
        document.getElementById('result').style.display = 'block';
    });
```

- [ ] **Step 4: 编译检查（模板内联在 HTML，Rust 侧无变化）**

Run: `cargo check 2>&1 | tail -3`
Expected: 通过。

- [ ] **Step 5: Commit**

```bash
git add templates/new_share.html
git commit -m "feat: add optional view password field to create page"
```

---

## Task 4: 查看页支持密码解锁

**Files:**
- Modify: `templates/view_share.html`

> 本任务顺带把字段渲染从字符串拼接（`innerHTML` + `escapeHtml`）改为安全 DOM 构造（`createElement` + `textContent`），杜绝 XSS 拼接面，并移除不再需要的 `escapeHtml`。

- [ ] **Step 1: 在 `#content` 之后、`<script src=...>` 之前加入密码表单块**

把现有：

```html
<div id="content" style="display:none;">
    <h2 id="title"></h2>
    <table>
        <tbody id="fields"></tbody>
    </table>
</div>

<script src="/static/crypto.js"></script>
```

替换为：

```html
<div id="content" style="display:none;">
    <h2 id="title"></h2>
    <table>
        <tbody id="fields"></tbody>
    </table>
</div>

<div id="password-prompt" style="display:none; background:white; padding:1rem; border-radius:8px;">
    <p>此分享受密码保护，请输入查看密码：</p>
    <label><input type="password" id="view-password" autocomplete="off"></label>
    <button id="unlock-btn">解锁</button>
    <p id="password-error" style="color:red; display:none;">密码错误，请重试</p>
</div>

<script src="/static/crypto.js"></script>
```

- [ ] **Step 2: 用以下完整 `<script>` 块替换查看页的内联脚本**

把 `<script src="/static/crypto.js"></script>` 之后那段 `<script> ... </script>`（整段 IIFE 加 `escapeHtml`）整体替换为下面这段（注意：不再使用 `escapeHtml`，改用 DOM 构造）：

```html
<script>
    const slug = window.location.pathname.split('/').pop();

    function showError(msg) {
        document.getElementById('loading').style.display = 'none';
        const el = document.getElementById('error');
        el.textContent = msg;
        el.style.display = 'block';
    }

    function renderPayload(payload) {
        document.getElementById('loading').style.display = 'none';
        document.getElementById('password-prompt').style.display = 'none';
        document.getElementById('content').style.display = 'block';
        document.getElementById('title').textContent = payload.title;

        const tbody = document.getElementById('fields');
        tbody.replaceChildren();
        payload.fields.forEach(field => {
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
    }

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

        if (data.kdf_salt) {
            // 密码模式：展示密码表单，提交时派生密钥解密
            document.getElementById('loading').style.display = 'none';
            document.getElementById('password-prompt').style.display = 'block';

            const unlock = async () => {
                const password = document.getElementById('view-password').value;
                document.getElementById('password-error').style.display = 'none';
                try {
                    const cryptoKey = await deriveKeyFromPassword(password, data.kdf_salt);
                    const payload = await decryptWithKey(cryptoKey, data.encrypted_payload);
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

        // 链接模式：密钥在 fragment
        const keyMatch = window.location.hash.match(/^#key=(.+)$/);
        if (!keyMatch) {
            showError('链接不完整，缺少解密密钥');
            return;
        }
        try {
            const cryptoKey = await importRawKey(keyMatch[1]);
            const payload = await decryptWithKey(cryptoKey, data.encrypted_payload);
            renderPayload(payload);
        } catch (e) {
            showError('无法解密，请检查链接是否完整');
        }
    })();
</script>
```

- [ ] **Step 3: 编译检查**

Run: `cargo check 2>&1 | tail -3`
Expected: 通过。

- [ ] **Step 4: Commit**

```bash
git add templates/view_share.html
git commit -m "feat: add password unlock flow to view page"
```

---

## Task 5: 全量验证（后端 + 浏览器手动）

**Files:** 无（仅验证）

- [ ] **Step 1: 后端验证门禁**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: 测试全过；clippy 无告警。

- [ ] **Step 2: 启动服务（端口 3000 可能被占用，用空闲端口 + 临时库）**

Run: `rm -f /tmp/ss-vp.db; DATABASE_URL="sqlite:/tmp/ss-vp.db" BIND_ADDR="127.0.0.1:3048" cargo run`
（后台运行；验证完用 Ctrl-C 或 kill 结束）
Expected: 服务启动，监听 127.0.0.1:3048。

- [ ] **Step 3: 浏览器手动验证密码模式**

1. 打开 `http://127.0.0.1:3048/register` 注册并登录。
2. 进入 `/shares/new`，填标题与字段，**填写「查看密码」**（如 `pw123`），创建。
3. 确认结果链接形如 `http://127.0.0.1:3048/s/<slug>`（**不含 `#key=`**），并显示密码提示。
4. 新开隐身窗口打开该链接：应看到密码输入框。
5. 输入错误密码 → 显示「密码错误，请重试」。
6. 输入正确密码 `pw123` → 正确显示标题与字段，复制按钮可用。

Expected: 上述行为全部符合。

- [ ] **Step 4: 浏览器手动验证链接模式未回归**

1. 再创建一个分享，**「查看密码」留空**。
2. 确认链接形如 `.../s/<slug>#key=...`。
3. 打开链接：无需密码，直接显示字段。

Expected: 链接模式行为与之前一致。

- [ ] **Step 5: 结束服务，Final commit**

```bash
git commit -m "feat: complete view password support" --allow-empty
```

---

## Spec Coverage Review

| 需求 | 实现任务 |
|---|---|
| 创建分享时可选设置查看密码 | Task 3（前端输入）+ Task 1（存 salt） |
| 访问受保护链接需输入密码 | Task 4（密码表单） |
| 密码正确才解密显示 | Task 2（PBKDF2 派生）+ Task 4（解密） |
| 密码错误给出提示 | Task 4（捕获 GCM 认证失败 → 「密码错误」） |
| 服务端零知识（拿不到明文/密码） | Task 1（只存 salt 与密文）+ Task 2（密钥前端派生） |
| 不设密码时保持原链接行为 | Task 2/3/4（`kdf_salt` 为 null 分支） |

## Placeholder Scan

- 无 TBD/TODO；每个代码步骤含完整代码。
- 函数名一致：`importRawKey` / `deriveKeyFromPassword` / `encryptWithKey` / `decryptWithKey` / `generateKey` / `generateSalt` / `createShare(payload, password)` 全文一致；查看页用 `deriveKeyFromPassword` + `decryptWithKey`，创建页用 `createShare`。
- 字段名一致：后端 `kdf_salt`（snake_case，serde 默认），前端 JSON body 用 `kdf_salt`，响应读取 `data.kdf_salt`。

## 注意

- PBKDF2 与 AES-GCM 均需安全上下文（HTTPS 或 `localhost`/`127.0.0.1`），与现有 crypto.js 限制一致。
- 迁移用 `ALTER TABLE ... ADD COLUMN`：旧库补列，新库已含列时报错被 `let _ =` 忽略，幂等安全。
- Task 4 移除了 `escapeHtml`：渲染改用 `createElement` + `textContent`，无字符串拼接 HTML。
