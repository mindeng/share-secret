# Share Secret 设计文档

## 1. 系统概述

一个 Rust 单服务 Web 应用，使用 Axum 框架、SQLite 持久化、Askama 服务端模板渲染。支持用户注册/登录、创建带客户端加密的密文分享页、查看并一键复制字段值。

核心安全目标：服务端不存储任何可解密的明文内容，所有分享数据（标题、字段 label、字段 value）均在浏览器端使用 Web Crypto API 加密后提交。

## 2. 页面与路由

| 路由 | 说明 | 登录要求 |
|---|---|---|
| `GET /` | 首页，未登录引导注册/登录，已登录跳转 `/dashboard` | 无 |
| `GET /register` / `POST /register` | 注册页/提交 | 无 |
| `GET /login` / `POST /login` | 登录页/提交 | 无 |
| `POST /logout` | 登出 | 是 |
| `GET /dashboard` | 已登录用户主面板，列出自己创建的分享 | 是 |
| `GET /shares/new` / `POST /api/shares` | 创建分享页/提交 | 是 |
| `POST /api/shares/:id/delete` | 删除分享 | 是 |
| `GET /s/:slug` | 公共分享查看页 | 无 |
| `GET /api/shares/:slug` | 返回加密的分享 payload | 无 |

`slug` 使用随机字符串（如 12 位 base62），避免可猜测。

## 3. 数据模型

SQLite 三张表：

### users

| 字段 | 类型 | 说明 |
|---|---|---|
| id | INTEGER PRIMARY KEY | 自增 |
| username | TEXT UNIQUE NOT NULL | 用户名 |
| password_hash | TEXT NOT NULL | argon2 哈希 |
| created_at | DATETIME NOT NULL | 创建时间 |

### shares

| 字段 | 类型 | 说明 |
|---|---|---|
| id | INTEGER PRIMARY KEY | 自增 |
| user_id | INTEGER NOT NULL | 外键，关联 users |
| slug | TEXT UNIQUE NOT NULL | 公共分享标识 |
| encrypted_payload | TEXT NOT NULL | 前端加密的完整分享 JSON |
| created_at | DATETIME NOT NULL | 创建时间 |

删除分享时直接物理删除 `shares` 表中对应记录。创建者可从 `/dashboard` 删除自己的分享。

## 4. 组件划分

| 模块 | 职责 |
|---|---|
| `main.rs` | 启动服务、配置路由、连接数据库 |
| `db.rs` | SQLite 连接池初始化、建表迁移 |
| `auth.rs` | 用户密码 argon2 哈希、session cookie 提取、登录态守卫 |
| `models.rs` | `User`, `Share`, `EncryptedPayload` 等结构体 |
| `handlers/auth.rs` | 注册、登录、登出处理器 |
| `handlers/dashboard.rs` | 首页、dashboard 处理器 |
| `handlers/share.rs` | 创建、删除、查看分享处理器 |
| `templates/` | Askama 模板文件 |
| `static/crypto.js` | 前端加密/解密逻辑（Web Crypto API） |

模块间通过明确函数签名交互，例如 `auth::current_user` 返回 `Option<User>`，处理器据此决定是否重定向。

## 5. 数据流与关键流程

### 5.1 创建分享

1. 已登录用户访问 `/shares/new`。
2. 前端生成随机 Server Key，并预生成分享链接 `/s/:slug#serverKey=<base64url>`。
3. 用户填写标题、自定义字段（多组 label/value）。
4. 前端将内容序列化为 JSON：`{ "title": "...", "fields": [{ "label": "...", "value": "..." }] }`。
5. 使用 Server Key 通过 AES-GCM 加密整个 JSON，得到 `ciphertext + nonce`。
6. 提交到服务端：`slug`, `encrypted_payload`（base64 编码的 ciphertext + nonce）。
7. 服务端验证登录态、保存记录，返回成功。
8. 前端页面显示完整链接并提供"复制链接"按钮。

**注意**：Server Key 只存在于前端和 URL fragment 中，服务端从未接触。创建页面是唯一能获得完整链接的时机。

### 5.2 查看分享

1. 用户打开 `/s/:slug#serverKey=<base64url>`。
2. 浏览器从 URL fragment 中提取 Server Key，不发送给服务器。
3. 页面通过 `/api/shares/:slug` 获取 `encrypted_payload`。
4. 前端使用 Server Key 解密 payload。
5. 渲染标题、字段列表，每个字段旁提供"复制"按钮。

### 5.3 删除分享

1. 登录用户在 `/dashboard` 看到自己创建的分享列表（仅显示 slug 和创建时间，不显示标题等加密内容）。
2. 点击删除后，后端校验 `share.user_id == current_user.id`，然后删除记录。

## 6. 安全设计

- 服务端只保存 `slug` 和 `encrypted_payload`，不保存 Server Key，也不保存任何明文。
- 用户登录密码使用 argon2 哈希存储。
- 分享 URL 中的 Server Key 位于 fragment（`#` 之后），浏览器不会将其发送到服务器。
- 明文仅在浏览器内存和渲染后的 DOM 中出现。
- 如果用户丢失完整链接，服务端无法恢复内容。

## 7. 错误处理

| 场景 | 处理方式 |
|---|---|
| 注册/登录表单校验失败 | 返回同页并显示错误信息 |
| 用户名已存在 | 提示"用户名已被使用" |
| 登录凭据错误 | 模糊提示"用户名或密码错误" |
| 未登录访问 dashboard | 重定向到 `/login` |
| slug 不存在 | 返回 404 页面 |
| URL fragment 中缺少 Server Key | 提示"链接不完整" |
| Server Key 错误或解密失败 | 提示"无法解密，请检查链接是否完整" |
| 删除非自己的分享 | 返回 403 |

采用 `Result<T, AppError>` 统一错误类型，最终映射为对应状态码和模板页面。

## 8. 技术栈

| 用途 | 库 |
|---|---|
| Web 框架 | `axum` |
| 模板引擎 | `askama` |
| 数据库 | `sqlite` + `sqlx` |
| 密码哈希 | `argon2` |
| Session | `tower-sessions` + `tower-cookies` |
| 前端加密 | Web Crypto API（AES-GCM） |
| 工具 | `rand`, `serde`, `base64` |

## 9. 测试策略

| 层级 | 内容 |
|---|---|
| 单元测试 | 加密/解密辅助函数、slug/key 生成、密码哈希校验 |
| 集成测试 | 使用 `axum::TestServer` 测试注册/登录/创建/删除/查看接口 |
| 数据库 | 测试使用内存 SQLite，每个测试独立建表 |
| 端到端 | 使用 Playwright 或 jsdom 验证前端加密解密流程 |

重点验证：服务端无法解密、完整链接能正确解密、错误 key 无法解密。

## 10. 二期规划（待定）

- 可选 View Password：对 Server Key 再进行一层加密，查看时需输入密码。
- 分享过期时间或访问次数限制。
