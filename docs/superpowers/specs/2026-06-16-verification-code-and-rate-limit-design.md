# 注册验证码 + 登录限流 设计

日期：2026-06-16

## 目标

1. 注册必须通过**验证码**才能完成，验证码由服务端生成并**打印到控制台**（无邮箱/短信通道）。
2. 验证码错误**限制重试次数**。
3. 登录密码错误**限制重试次数**。

## 决策摘要（已与用户确认）

| 项 | 决策 |
|---|---|
| 注册流程 | 单页 + "获取验证码"按钮（JS `fetch` 调用获取码接口） |
| 存储 | 内存 `Mutex<HashMap<..>>`，与现有 `MemoryStore` session 一致；重启即清空 |
| 限流策略 | 锁定一段时间（验证码错满即作废需重取；登录错满锁定 15 分钟） |
| 锁定维度 | **用户名**（非 IP）。`LoginGuard` 的 key 用泛化 `String`，留 IP 扩展点 |
| 为何不锁 IP | 流量经 L7 Gateway 代理，Pod 看不到真实客户端 IP（在 XFF 头里），且 Service 是 ClusterIP，`externalTrafficPolicy` 不适用。读 XFF 需处理伪造/可信代理，对邀请制小工具成本偏高 |

## 默认参数（常量，集中定义便于调整）

| 参数 | 值 |
|---|---|
| 验证码长度 | 6 位数字 |
| 验证码有效期 | 10 分钟 |
| 验证码错误上限 | 5 次（达到即作废，需重新获取） |
| 验证码请求冷却 | 同一用户名 60 秒内不可重复请求（防刷控制台） |
| 登录失败上限 | 连续 5 次 |
| 登录锁定时长 | 15 分钟 |

## 架构

新增内存安全模块 `src/security.rs`，含两个独立、可单测的结构体，挂到 `AppState`：

```rust
pub struct AppState {
    pub db: sqlx::SqlitePool,
    pub codes: CodeStore,        // 注册验证码
    pub login_guard: LoginGuard, // 登录锁定
}
```

- 计时用 `std::time::Instant`（单调时钟）。
- 锁用 `std::sync::Mutex`，**不跨 `.await` 持锁**（临界区只做内存读写）。

### `CodeStore`

```rust
struct CodeEntry { code: String, expires_at: Instant, created_at: Instant, attempts: u32 }
pub struct CodeStore { inner: Mutex<HashMap<String, CodeEntry>> }

impl CodeStore {
    /// 生成并存储新码；冷却期内返回 Err(秒数)。
    /// 返回明文码，由 handler 负责 println! 到控制台。
    fn issue(&self, username: &str) -> Result<String, CooldownRemaining>;
    /// 校验：不存在/过期/错误均失败；错误时 attempts+1，达上限删除条目。
    /// 成功时删除条目。
    fn verify(&self, username: &str, code: &str) -> Result<(), CodeError>;
}
```

`CodeError` 区分：`NoCode`（请先获取验证码）、`Expired`、`Wrong`（验证码错误）、`TooManyAttempts`（错误过多，已作废）。

随机码用现有 `rand` crate 生成 6 位数字（`000000`–`999999`，左填零）。

### `LoginGuard`

```rust
struct Attempt { failures: u32, locked_until: Option<Instant> }
pub struct LoginGuard { inner: Mutex<HashMap<String, Attempt>> }

impl LoginGuard {
    /// 锁定中返回 Err(剩余秒数)，否则 Ok。
    fn check(&self, key: &str) -> Result<(), LockRemaining>;
    /// 记一次失败；达上限设置 locked_until。
    fn record_failure(&self, key: &str);
    /// 成功：清除该 key 的计数。
    fn record_success(&self, key: &str);
}
```

`key` 当前传 `username`；将来换 IP 只改 handler 取 key 的那一行。

## 数据流

### 获取验证码：`POST /register/code`
- 入参：`username`（form 或 JSON）。
- `CodeStore.issue(username)`：
  - 冷却中 → 返回 JSON `{ ok: false, message: "请 N 秒后再获取" }`。
  - 否则生成码，`println!("[验证码] 用户 {username} 的注册验证码: {code}")`，返回 `{ ok: true, message: "验证码已打印到服务器控制台" }`。
- 用户名为空 → `{ ok: false, message: "请先填写用户名" }`。

### 注册提交：`POST /register`
- 入参新增 `code` 字段（`RegisterForm` 加 `code: String`）。
- 校验顺序：
  1. username/password 非空（沿用现有校验）。
  2. `CodeStore.verify(username, code)` → 失败按 `CodeError` 渲染对应中文错误。
  3. 通过 → `INSERT users`；唯一冲突 → "用户名已存在"（此时验证码已被 verify 消费/删除，用户需重新获取——可接受，因为用户名冲突应改名重来）。
  4. 成功 → `Redirect::to("/login")`。

### 登录：`POST /login`
- `LoginGuard.check(username)`：
  - 锁定中 → 渲染 `LoginTemplate { error: "尝试过于频繁，请 N 分钟后再试" }`。
- 查用户 + 验密码：
  - 成功 → `record_success` + 登录跳转。
  - 失败（用户不存在或密码错）→ `record_failure`，渲染"用户名或密码错误"。
    - 注：用户不存在也记失败，避免泄露用户是否存在；锁定按输入的 username 维度。

## 模板改动

### `register.html`
- 加验证码输入框 + "获取验证码"按钮。
- 内联 JS：点击按钮 → `fetch('/register/code', {POST, body: username})` → 弹出/内联显示返回 `message`。
- 表单保留 username/password/code，整体走原生 `POST /register`（无需 JS 提交）。
- 与现有 `static/crypto.js` 一致，使用少量内联 JS 即可。

### `login.html`
- 无结构改动；错误信息复用现有 `error` 字段（锁定提示走同一通道）。

## 错误处理

- 注册/登录失败：重渲染对应 Askama 模板并填 `error`（沿用现有模式）。
- `/register/code`：始终返回 `200` + JSON `{ ok, message }`，由 JS 展示。

## 测试

### 单元测试（`src/security.rs`）
- `CodeStore`：
  - 正确码通过且条目被消费。
  - 错误码累计到 5 次后条目作废（再验证返回 `TooManyAttempts`/`NoCode`）。
  - 冷却：连续两次 `issue` 第二次返回冷却错误。
  - 过期：用很短的有效期常量或可注入时长验证过期路径。
- `LoginGuard`：
  - 连续 5 次 `record_failure` 后 `check` 返回锁定。
  - `record_success` 清零，`check` 恢复。
- 为可测试性，时长以常量定义；对过期/锁定时间的测试采用短时长或在结构上允许构造条目。

### 集成测试（`tests/integration_test.rs`）
- 现有 `register_and_login` 等辅助：注册前先 `POST /register/code`，再从 `CodeStore` 取出当前码注入注册请求。
- 为此 `CodeStore` 暴露一个查询当前明文码的方法（`pub fn peek(&self, username) -> Option<String>`，仅供测试/调试使用，文档注明）。
- 新增用例：
  - 无验证码注册被拒。
  - 错误验证码注册被拒。
  - 登录连错 5 次后第 6 次返回锁定（用很短锁定时长或验证状态）。

## 非目标 / 取舍

- 不持久化验证码与登录计数（重启清空，与现有内存 session 一致）。
- 不实现 IP 维度限流（留扩展点）。
- 不改 k8s（`externalTrafficPolicy` 在当前 L7 Gateway + ClusterIP 拓扑下不适用）。
- 不引入日志框架，"控制台打印"用 `println!`。
