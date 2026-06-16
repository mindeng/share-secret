# 分享管理 UX 改进：密码自动填充、显示切换、创建后跳转、导航

**日期**: 2026-06-16
**状态**: 设计已确认，待写实现计划

## 背景

分享支持两种模式（见 `static/crypto.js`）：

- **链接模式**：随机密钥放在 URL fragment（`#key=...`），链接本身即可解密。
- **密码模式**：密钥由用户设置的"查看密码" + salt 经 PBKDF2 派生，链接不含密钥。

查看密码保护的分享时（`templates/view_share.html`），用户在 `#view-password` 输入密码解锁。所有者解锁后可点"编辑"打开编辑表单（`#edit-form`）。所有 HTML 页面 extend `templates/base.html`，header 仅有一个指向 `/` 的 "Share Secret" logo 链接。

## 问题

1. **编辑时密码框被清空**：`openEditForm()` 执行 `edit-view-password.value = ''`。解锁时输入的密码只存在于 `unlock()` 闭包内，编辑时无法复用，所有者必须重新输入。
2. **静默退化为链接模式（隐患）**：所有者只想改内容、不重输密码就保存时，`updateShare()` 因 `password` 为空走 `else` 分支——**生成新随机密钥、切换为链接模式，把密钥暴露到 URL**。非预期的安全降级。
3. **密码无法查看**：所有 `type="password"` 输入框无法切换为明文，难以核对。
4. **创建后无法直接进入分享**：创建成功只显示链接，要编辑还得手动回 dashboard 找。
5. **登录用户进入不便**：访问 `/` 总是落在公开首页，没有快捷进入 dashboard 的入口。
6. **dashboard「创建新分享」不醒目**：只是一个和导出/导入并排的普通文字链接。

## 目标

- 编辑密码保护的分享时，自动回填解锁时输入的查看密码，并消除"静默退化为链接模式"的隐患。
- 给三处密码输入框各加一个"显示/隐藏"切换按钮。
- 创建分享后，保留结果面板，并提供进入该分享（查看/编辑）的按钮。
- 已登录用户访问 `/` 时重定向到 `/dashboard`（logo 即 dashboard 入口）。
- dashboard 的「创建新分享」做成右对齐的醒目按钮。

## 非目标

- 不改变两种分享模式的密码学设计。
- 不持久化密码（仅在当前页面会话内存中，与现有 `currentKey` 一致）。
- 不新增独立的"编辑页"路由——编辑仍内联在 `/s/{slug}` 查看页。
- 不给所有模板引入 `authenticated` 标志（用 `/` 重定向替代显式导航链接，避免大面积改动）。

## 设计

### 1. 编辑时密码自动填充（`templates/view_share.html`）

- 新增模块级 `let currentPassword = null;`（与 `currentKey`、`currentMode` 并列）。
- `unlock()` 解密成功后：`currentPassword = password;`。
- `openEditForm()` 把 `edit-view-password.value = '';` 改为：
  - 密码模式（`currentMode === 'password'`）→ 回填 `currentPassword ?? ''`。
  - 否则（链接模式）→ 留空。
- 保存成功后同步 `currentPassword`：密码模式 → `= password`（采用本次输入，支持改密码）；链接模式 → `= null`。

**附带修复**：密码模式下编辑框默认带原密码，直接保存即带密码提交，保持密码模式，不再静默泄露密钥到 URL。用户若想切链接模式，手动清空密码框即可——行为显式可控。

### 2. 显示/隐藏切换按钮（新文件 `static/ui.js`）

```js
function attachPasswordToggle(input) {
  // 1. 用 flex 容器（display:flex; gap; align-items:stretch）包裹 input
  // 2. 容器内 input 旁追加 width:auto 的图标按钮
  // 3. 点击在 input.type 'password' <-> 'text' 间切换
  // 4. 同步切换内联 SVG 图标（睁眼 / 闭眼带斜杠）与 aria-label（"显示密码" / "隐藏密码"）
}
```

- **图标**：内联 SVG，两种状态（睁眼 / 闭眼带斜杠）。无外部依赖，渲染稳定。
- **可访问性**：按钮 `type="button"` + `aria-label`，随状态更新。
- **样式**：按钮 `width:auto`（沿用 `base.html` 表格复制按钮写法），避免被全局 `input, button { width: 100% }` 撑满；flex 容器让输入框与按钮并排。
- **接入三处**：`view_share.html` 的 `#edit-view-password`、`#view-password`；`new_share.html` 的 `#view-password`。两模板各加 `<script src="/static/ui.js"></script>` 并初始化。`#edit-view-password` 在初始隐藏的编辑表单内，元素始终存在，绑定一次即可。

### 3. 创建后跳转（`templates/new_share.html`）

- 创建成功后，结果面板照常显示链接 / 复制 / 密码提示。
- 结果面板内新增「查看 / 编辑此分享」按钮，`href` = 查看 URL（复用已有 `fullUrl`）：
  - 链接模式：`/s/{slug}#key={key}`
  - 密码模式：`/s/{slug}`

### 4. 登录后 `/` 重定向到 `/dashboard`（`src/handlers/dashboard.rs`）

- `index` handler 改用可选提取器 `Option<CurrentUser>`：
  - `Some` → `Redirect::to("/dashboard")`。
  - `None` → 渲染 `IndexTemplate`（公开首页不变）。
- header 现有的 "Share Secret" logo（→ `/`）对已登录用户即成为 dashboard 入口，无需新增导航链接、无需改 base.html。
- 登录成功已重定向到 `/dashboard`（`auth.rs:148`），与本改动一致。

### 5. dashboard「创建新分享」做成醒目按钮（`templates/dashboard.html`）

- 当前顶部 `<p>` 内三个并排元素：`创建新分享`(链接) / `导出全部`(链接) / `导入`(按钮)。
- 调整为：「创建新分享」渲染成醒目按钮（沿用 `base.html` 蓝色按钮样式，如用 `<a>` 则套按钮样式），并 `margin-left:auto` 右对齐；导出/导入留在左侧。
- 纯样式/标记调整，不改导入逻辑。

## 改动文件

- `static/ui.js`（新增）
- `templates/view_share.html`（自动填充 + 引 ui.js + 两处接入）
- `templates/new_share.html`（跳转按钮 + 引 ui.js + 一处接入）
- `templates/dashboard.html`（创建按钮样式 + 右对齐）
- `src/handlers/dashboard.rs`（`index` 改 `Option<CurrentUser>` + 重定向）

## 测试

后端仅 `index` 重定向逻辑，可加一条集成测试（已登录访问 `/` → 302 到 `/dashboard`；未登录 → 200 渲染首页）。其余为纯客户端 UI 行为，现有 Rust 测试覆盖不到，手动验证清单：

1. 解锁密码保护的分享 → 点编辑 → 密码框已回填原密码。
2. 不动密码直接保存 → 仍为密码模式，URL 不含 `#key=`，密码提示仍显示。
3. 手动清空密码框保存 → 切换为链接模式，URL 出现 `#key=`。
4. 三处密码框点切换按钮 → 明文/掩码正确切换，图标与 aria-label 同步。
5. 链接模式分享编辑 → 密码框为空（无回填），行为不变。
6. 创建分享（两种模式）→ 结果面板显示链接 + 「查看/编辑此分享」按钮，点击进入对应查看页。
7. 已登录访问 `/` → 跳转 `/dashboard`；登出后访问 `/` → 显示首页。
8. dashboard「创建新分享」显示为右对齐醒目按钮，点击进入创建页。
