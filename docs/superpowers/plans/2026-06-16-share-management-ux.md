# 分享管理 UX 改进 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 改善分享的编辑/创建/导航体验：编辑时自动填充查看密码、密码框可切换显示、创建后可一键进入分享、登录用户落到 dashboard、dashboard 创建入口更醒目。

**Architecture:** 几乎全是前端改动（askama 模板 + 一个新的静态 JS helper）。唯一后端改动是 `index` handler 改为可选鉴权并对已登录用户重定向到 `/dashboard`。askama 在编译期校验模板，所以 `cargo build` 能抓出模板语法错误；客户端行为以浏览器手动验证为主。

**Tech Stack:** Rust + axum 0.7.9 + askama 模板；原生浏览器 JS（Web Crypto，已有 `static/crypto.js`）。

---

## File Structure

- `src/handlers/dashboard.rs`（修改）：`index` handler 改用 `Option<CurrentUser>`，已登录重定向 `/dashboard`。
- `tests/integration_test.rs`（修改）：新增 `index` 重定向的两条集成测试。
- `static/ui.js`（新增）：`attachPasswordToggle(input)` —— 给密码框加眼睛切换按钮。单一职责，复用于多个模板。
- `templates/view_share.html`（修改）：编辑时密码自动填充；引入 `ui.js` 并对两处密码框接入切换。
- `templates/new_share.html`（修改）：引入 `ui.js` 接入切换；结果面板加「查看 / 编辑此分享」按钮。
- `templates/dashboard.html`（修改）：「创建新分享」改为右对齐醒目按钮。

各任务自成一体，可独立提交。

---

## Task 1: `index` 已登录重定向到 `/dashboard`

**Files:**
- Modify: `src/handlers/dashboard.rs:18-20`（`index` 函数）及顶部 `use`
- Test: `tests/integration_test.rs`（文件末尾追加两条测试）

- [ ] **Step 1: 写失败测试**

在 `tests/integration_test.rs` 末尾追加（复用文件已有的 `make_app` / `register_and_login` / `body_string` 辅助函数）：

```rust
#[tokio::test]
async fn test_index_redirects_logged_in_to_dashboard() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "homeuser").await;
    let req = Request::builder()
        .method("GET")
        .uri("/")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers().get("location").unwrap(), "/dashboard");
}

#[tokio::test]
async fn test_index_anonymous_shows_landing() {
    let (app, _state) = make_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = body_string(res.into_body()).await;
    assert!(html.contains("注册"), "匿名首页应包含注册链接");
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test --test integration_test test_index_ -- --nocapture`
Expected: `test_index_redirects_logged_in_to_dashboard` FAIL（当前 `/` 对已登录用户返回 200 而非 303）。

- [ ] **Step 3: 改 `index` handler**

把 `src/handlers/dashboard.rs` 顶部的：

```rust
use axum::{extract::State, response::Html};
```

改为：

```rust
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
};
```

把现有的 `index` 函数：

```rust
pub async fn index() -> Result<Html<String>, AppError> {
    Ok(Html(IndexTemplate.render()?))
}
```

改为：

```rust
pub async fn index(user: Option<CurrentUser>) -> Result<Response, AppError> {
    if user.is_some() {
        return Ok(Redirect::to("/dashboard").into_response());
    }
    Ok(Html(IndexTemplate.render()?).into_response())
}
```

说明：`CurrentUser` 已在该文件顶部 `use crate::auth::CurrentUser;` 导入。axum 0.7 对 `Option<T>`（`T: FromRequestParts`）的 blanket 实现会把提取失败（含未登录的 `AppError::Auth`）映射为 `None`，因此无需改 `CurrentUser` 本身。

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test --test integration_test test_index_ -- --nocapture`
Expected: 两条测试 PASS。

- [ ] **Step 5: 跑全量测试确保无回归**

Run: `cargo test`
Expected: 全部 PASS。

- [ ] **Step 6: 提交**

```bash
git add src/handlers/dashboard.rs tests/integration_test.rs
git commit -m "feat: redirect logged-in users from / to /dashboard"
```

---

## Task 2: 新增 `static/ui.js` —— 密码显示/隐藏切换 helper

**Files:**
- Create: `static/ui.js`

无 JS 测试框架，本任务以浏览器手动验证（在 Task 3/4 接入后整体验证）；本步先保证文件存在且语法正确。图标用静态 SVG 常量，并通过 `DOMParser` 构造节点（不使用 `innerHTML`，避免 XSS 风险，也通过仓库的安全检查 hook）。

- [ ] **Step 1: 创建 `static/ui.js`**

```js
// 给一个密码输入框旁边加一个"显示/隐藏"切换按钮（眼睛图标）。
// input 处于 password 时按钮显示睁眼图标（aria-label「显示密码」），点击切到明文并显示闭眼图标。

// 静态、可信的 SVG 字符串；用 DOMParser 解析成节点，避免使用 innerHTML。
const _EYE_OPEN_SVG =
    '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>';
const _EYE_OFF_SVG =
    '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>';

function _svgIcon(markup) {
    const doc = new DOMParser().parseFromString(markup, 'image/svg+xml');
    return document.importNode(doc.documentElement, true);
}

function attachPasswordToggle(input) {
    if (!input) return;

    // 用 flex 容器包裹 input，让输入框与按钮并排
    const wrap = document.createElement('div');
    wrap.style.cssText = 'display:flex; gap:0.5rem; align-items:stretch;';
    input.parentNode.insertBefore(wrap, input);
    wrap.appendChild(input);
    input.style.flex = '1';
    input.style.marginTop = '0';

    const btn = document.createElement('button');
    btn.type = 'button';
    btn.style.cssText =
        'width:auto; padding:0.4rem 0.8rem; display:inline-flex; align-items:center;';
    btn.setAttribute('aria-label', '显示密码');
    btn.replaceChildren(_svgIcon(_EYE_OPEN_SVG));

    btn.addEventListener('click', () => {
        const show = input.type === 'password';
        input.type = show ? 'text' : 'password';
        btn.replaceChildren(_svgIcon(show ? _EYE_OFF_SVG : _EYE_OPEN_SVG));
        btn.setAttribute('aria-label', show ? '隐藏密码' : '显示密码');
    });

    wrap.appendChild(btn);
}
```

- [ ] **Step 2: 提交**

```bash
git add static/ui.js
git commit -m "feat: add attachPasswordToggle helper (static/ui.js)"
```

---

## Task 3: `view_share.html` —— 编辑密码自动填充 + 接入切换按钮

**Files:**
- Modify: `templates/view_share.html`

- [ ] **Step 1: 引入 ui.js**

在 `templates/view_share.html` 中现有的：

```html
<script src="/static/crypto.js"></script>
```

改为（下面新增一行）：

```html
<script src="/static/crypto.js"></script>
<script src="/static/ui.js"></script>
```

- [ ] **Step 2: 新增 `currentPassword` 状态变量**

把现有：

```js
    const slug = window.location.pathname.split('/').filter(Boolean).pop();
    let currentPayload = null;
    let currentMode = null;   // 'link' | 'password'
    let currentKey = null;    // rawKeyB64 when link mode
    let isOwner = false;
```

改为（追加一行 `currentPassword`）：

```js
    const slug = window.location.pathname.split('/').filter(Boolean).pop();
    let currentPayload = null;
    let currentMode = null;       // 'link' | 'password'
    let currentKey = null;        // rawKeyB64 when link mode
    let currentPassword = null;   // 密码模式下解锁/保存时用过的查看密码（仅内存）
    let isOwner = false;
```

- [ ] **Step 3: 解锁成功后记住密码**

在底部 IIFE 的 `unlock` 函数里，把：

```js
                    const cryptoKey = await deriveKeyFromPassword(password, data.kdf_salt);
                    const payload = await decryptWithKey(cryptoKey, data.encrypted_payload);
                    currentMode = 'password';
                    currentKey = null;
                    renderPayload(payload);
```

改为（新增 `currentPassword = password;`）：

```js
                    const cryptoKey = await deriveKeyFromPassword(password, data.kdf_salt);
                    const payload = await decryptWithKey(cryptoKey, data.encrypted_payload);
                    currentMode = 'password';
                    currentKey = null;
                    currentPassword = password;
                    renderPayload(payload);
```

- [ ] **Step 4: 打开编辑表单时回填密码**

在 `openEditForm()` 里，把：

```js
        document.getElementById('edit-view-password').value = '';
        document.getElementById('edit-form').style.display = 'block';
```

改为：

```js
        document.getElementById('edit-view-password').value =
            currentMode === 'password' ? (currentPassword || '') : '';
        document.getElementById('edit-form').style.display = 'block';
```

- [ ] **Step 5: 保存成功后同步 `currentPassword`**

在 `edit-form` 的 submit handler 里，把：

```js
            if (passwordProtected) {
                currentMode = 'password';
                currentKey = null;
                history.replaceState(null, '', `/s/${slug}`);
            } else {
                currentMode = 'link';
                currentKey = key;
                history.replaceState(null, '', `/s/${slug}#key=${key}`);
            }
```

改为（同步 `currentPassword`）：

```js
            if (passwordProtected) {
                currentMode = 'password';
                currentKey = null;
                currentPassword = password;
                history.replaceState(null, '', `/s/${slug}`);
            } else {
                currentMode = 'link';
                currentKey = key;
                currentPassword = null;
                history.replaceState(null, '', `/s/${slug}#key=${key}`);
            }
```

- [ ] **Step 6: 接入切换按钮**

在底部 IIFE 的启动行 `(async () => {` 之前插入两行 helper 调用：

```js
    // 给两处密码输入框加显示/隐藏切换
    attachPasswordToggle(document.getElementById('view-password'));
    attachPasswordToggle(document.getElementById('edit-view-password'));

    (async () => {
```

（`#edit-view-password` 在初始隐藏的编辑表单内，但 DOM 元素已存在，绑定一次即可。）

- [ ] **Step 7: 编译校验模板**

Run: `cargo build`
Expected: 编译通过（askama 校验模板无误）。

- [ ] **Step 8: 浏览器手动验证**

Run: `cargo run`（按 README 启动；用浏览器打开站点）
验证：
1. 创建一个带查看密码的分享 → 打开 `/s/<slug>` → 输入密码解锁 → 点「编辑」→ 密码框已回填刚才的密码。
2. 不动密码直接「保存修改」→ 结果链接为 `/s/<slug>`（无 `#key=`），密码提示出现；地址栏无 `#key=`。
3. 重新进入编辑 → 手动清空密码框 → 保存 → 结果链接出现 `#key=`（切到链接模式）。
4. 解锁框与编辑密码框旁均有眼睛按钮，点击可在明文/掩码间切换，图标随之变化。
停止：`Ctrl-C`。

- [ ] **Step 9: 提交**

```bash
git add templates/view_share.html
git commit -m "feat: autofill view password on edit + password toggle in view_share"
```

---

## Task 4: `new_share.html` —— 接入切换按钮 + 创建后跳转按钮

**Files:**
- Modify: `templates/new_share.html`

- [ ] **Step 1: 引入 ui.js**

把：

```html
<script src="/static/crypto.js"></script>
```

改为：

```html
<script src="/static/crypto.js"></script>
<script src="/static/ui.js"></script>
```

- [ ] **Step 2: 结果面板加跳转按钮**

把结果面板：

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

改为（新增 `go-share` 按钮）：

```html
<div id="result" style="display:none; margin-top: 1rem; background: white; padding: 1rem; border-radius: 8px;">
    <p>分享链接：</p>
    <input type="text" id="share-link" readonly>
    <button id="copy-link">复制链接</button>
    <button type="button" id="go-share" style="width:auto; margin-top:0.5rem;">查看 / 编辑此分享</button>
    <p id="password-hint" style="display:none; color:#b45309; margin-top:0.75rem;">
        ⚠ 此分享受密码保护。请通过其它安全渠道把查看密码告诉对方，链接本身不含密钥。
    </p>
</div>
```

- [ ] **Step 3: submit handler 里接上跳转按钮**

在 submit handler 中，把：

```js
            document.getElementById('share-link').value = fullUrl;
            document.getElementById('password-hint').style.display = passwordProtected ? 'block' : 'none';
            document.getElementById('result').style.display = 'block';
```

改为（新增 `go-share` 的点击行为）：

```js
            document.getElementById('share-link').value = fullUrl;
            document.getElementById('password-hint').style.display = passwordProtected ? 'block' : 'none';
            document.getElementById('go-share').onclick = () => { window.location.href = fullUrl; };
            document.getElementById('result').style.display = 'block';
```

- [ ] **Step 4: 接入切换按钮**

在 `<script>` 块内、`document.getElementById('add-field')...` 那段事件绑定之前（脚本顶部），新增：

```js
    attachPasswordToggle(document.getElementById('view-password'));
```

- [ ] **Step 5: 编译校验模板**

Run: `cargo build`
Expected: 编译通过。

- [ ] **Step 6: 浏览器手动验证**

Run: `cargo run`
验证：
1. 创建页密码框旁有眼睛按钮，可切换显示。
2. 创建链接模式分享 → 结果面板出现「查看 / 编辑此分享」→ 点击进入 `/s/<slug>#key=...` 并显示内容。
3. 创建密码模式分享 → 点击「查看 / 编辑此分享」→ 进入 `/s/<slug>` 弹出密码提示。
停止：`Ctrl-C`。

- [ ] **Step 7: 提交**

```bash
git add templates/new_share.html
git commit -m "feat: post-create open-share button + password toggle in new_share"
```

---

## Task 5: `dashboard.html` —— 「创建新分享」改为右对齐醒目按钮

**Files:**
- Modify: `templates/dashboard.html`

- [ ] **Step 1: 调整顶部操作栏**

把：

```html
<p style="display: flex; gap: 1rem; flex-wrap: wrap; align-items: center;">
    <a href="/shares/new">创建新分享</a>
    <a href="/api/shares/export" download="share-secret-export.json">导出全部</a>
    <button type="button" id="import-btn" style="width: auto;">导入</button>
    <input type="file" id="import-file" accept="application/json,.json" style="display: none;">
</p>
```

改为（导出/导入留左侧，创建按钮右对齐且醒目）：

```html
<p style="display: flex; gap: 1rem; flex-wrap: wrap; align-items: center;">
    <a href="/api/shares/export" download="share-secret-export.json">导出全部</a>
    <button type="button" id="import-btn" style="width: auto;">导入</button>
    <input type="file" id="import-file" accept="application/json,.json" style="display: none;">
    <a href="/shares/new" style="margin-left:auto; background:#2563eb; color:#fff; padding:0.5rem 1rem; border-radius:4px; text-decoration:none;">创建新分享</a>
</p>
```

说明：`<a>` 不会继承 `base.html` 里只针对 `<button>` 的样式，所以用内联样式复刻蓝色按钮外观；`margin-left:auto` 把它推到右侧；`input[type=file]` 为 `display:none` 不影响布局。

- [ ] **Step 2: 编译校验模板**

Run: `cargo build`
Expected: 编译通过。

- [ ] **Step 3: 浏览器手动验证**

Run: `cargo run`（登录后访问 `/` 应跳到 `/dashboard`）
验证：「创建新分享」显示为右对齐的蓝色醒目按钮，点击进入创建页；导出/导入仍在左侧、功能正常。
停止：`Ctrl-C`。

- [ ] **Step 4: 提交**

```bash
git add templates/dashboard.html
git commit -m "feat: make dashboard create-share a prominent right-aligned button"
```

---

## 最终验证

- [ ] **跑全量测试**

Run: `cargo test`
Expected: 全部 PASS。

- [ ] **完整手动回归**（对照 spec 测试清单 1–8）

Run: `cargo run`，依次验证：
1. 解锁密码分享 → 编辑 → 密码已回填。
2. 不动密码保存 → 仍密码模式，无 `#key=`，密码提示在。
3. 清空密码保存 → 切链接模式，出现 `#key=`。
4. 三处密码框眼睛按钮可切换明文/掩码，图标与 aria-label 同步。
5. 链接模式分享编辑 → 密码框为空，行为不变。
6. 创建（两种模式）→ 结果面板有「查看 / 编辑此分享」，点击进入对应查看页。
7. 已登录访问 `/` → 跳 `/dashboard`；登出后访问 `/` → 显示首页。
8. dashboard「创建新分享」为右对齐醒目按钮。
