# 编辑分享：密码自动填充 + 显示/隐藏切换

**日期**: 2026-06-16
**状态**: 设计已确认，待写实现计划

## 背景

分享支持两种模式（见 `static/crypto.js`）：

- **链接模式**：随机密钥放在 URL fragment（`#key=...`），链接本身即可解密。
- **密码模式**：密钥由用户设置的"查看密码" + salt 经 PBKDF2 派生，链接不含密钥。

查看密码保护的分享时（`templates/view_share.html`），用户在 `#view-password` 输入密码解锁。所有者解锁后可点"编辑"打开编辑表单（`#edit-form`）。

## 问题

1. **编辑时密码框被清空**：`openEditForm()` 执行 `document.getElementById('edit-view-password').value = '';`。解锁时输入的密码只存在于 `unlock()` 闭包内，编辑时无法复用，所有者必须重新输入。

2. **静默退化为链接模式（隐患）**：若所有者只想改内容、不重输密码就直接保存，`updateShare()` 因 `password` 为空、`currentMode === 'password'` 而走到 `else` 分支——**生成新随机密钥、切换为链接模式，把密钥暴露到 URL**。这是非预期的安全降级。

3. **密码无法查看**：所有密码输入框（`type="password"`）无法切换为明文，用户难以核对输入。

## 目标

- 编辑密码保护的分享时，自动回填解锁时输入的查看密码。
- 给三处密码输入框各加一个"显示/隐藏"切换按钮。
- 顺带消除"静默退化为链接模式"的隐患（自动填充使默认保存保持密码模式）。

## 非目标

- 不改后端、API、数据库。纯前端改动。
- 不改变两种分享模式的密码学设计。
- 不持久化密码（仅在当前页面会话的内存中，与现有 `currentKey` 一致）。

## 设计

### 1. 密码自动填充（`templates/view_share.html`）

- 新增模块级变量 `let currentPassword = null;`（与 `currentKey`、`currentMode` 并列）。
- `unlock()` 解密成功后设置 `currentPassword = password;`。
- `openEditForm()` 把 `document.getElementById('edit-view-password').value = '';` 改为：
  - 密码模式（`currentMode === 'password'`）→ 回填 `currentPassword`（可能为 `null` 时按空串处理）。
  - 否则（链接模式）→ 留空。
- 保存成功后同步 `currentPassword`：
  - 保存为密码模式（`passwordProtected` 为真）→ `currentPassword = password;`（采用用户本次输入，支持改密码）。
  - 切换为链接模式 → `currentPassword = null;`。

**附带修复**：因为密码模式下编辑框默认带着原密码，用户直接保存会带着密码提交，`updateShare()` 走密码分支，保持密码模式，不再静默生成新密钥泄露到 URL。用户若确实想切换为链接模式，手动清空密码框即可——行为显式、可控。

### 2. 显示/隐藏切换按钮（新文件 `static/ui.js`）

新建共享脚本，提供：

```js
function attachPasswordToggle(input) {
  // 1. 用一个 flex 容器（display:flex; gap; align-items:stretch）包裹 input
  // 2. 容器内 input 旁追加一个 width:auto 的图标按钮
  // 3. 点击时在 input.type 'password' <-> 'text' 间切换
  // 4. 同步切换内联 SVG 图标（睁眼 / 闭眼带斜杠）与 aria-label（"显示密码" / "隐藏密码"）
}
```

要点：

- **图标**：内联 SVG，两种状态（睁眼 / 闭眼带斜杠）。不引外部依赖，渲染稳定，优于 emoji。
- **可访问性**：按钮设 `type="button"` 与 `aria-label`（"显示密码" / "隐藏密码"），随状态更新。
- **样式**：按钮 `width:auto`（沿用 `base.html` 中表格复制按钮的写法），避免全局 `input, button { width: 100% }` 把按钮撑满整行。flex 容器让输入框与按钮并排。

### 3. 接入

应用 `attachPasswordToggle` 到三处密码框：

- `templates/view_share.html`：`#edit-view-password`、`#view-password`
- `templates/new_share.html`：`#view-password`

两个模板各加：`<script src="/static/ui.js"></script>`，并在脚本中对相应输入框调用 `attachPasswordToggle`。注意 `#edit-view-password` 在编辑表单中，初始 `display:none`——切换 helper 只需绑定一次（DOM 元素始终存在），不受显隐影响。

## 改动文件

- `static/ui.js`（新增）
- `templates/view_share.html`（自动填充逻辑 + 引入 ui.js + 两处接入）
- `templates/new_share.html`（引入 ui.js + 一处接入）

## 测试

纯客户端 UI 行为，现有 Rust 测试（SQLite/Postgres）覆盖不到。手动验证清单：

1. 解锁密码保护的分享 → 点编辑 → 密码框已回填原密码。
2. 不动密码直接保存 → 仍为密码模式，URL 不含 `#key=`，密码提示仍显示。
3. 手动清空密码框保存 → 切换为链接模式，URL 出现 `#key=`。
4. 三处密码框点击切换按钮 → 明文/掩码正确切换，图标与 aria-label 同步。
5. 链接模式分享编辑 → 密码框为空（无回填），行为不变。

不新增自动化测试（除非后续决定加端到端测试）。
