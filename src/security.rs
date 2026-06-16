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
        if let Some(a) = map.get_mut(key)
            && let Some(until) = a.locked_until
        {
            if now < until {
                return Err(until - now);
            }
            a.failures = 0;
            a.locked_until = None;
        }
        Ok(())
    }

    /// 记一次失败；达上限则设置锁定截止时间。
    /// 约定：调用方应先用 `check` 拦截已锁定的 key，故锁定期内不会再调用本方法。
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

    #[test]
    fn expired_code_is_rejected() {
        let s = store();
        let code = s.issue("erin").unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(s.verify("erin", &code), Err(CodeError::Expired));
    }
}

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

    #[test]
    fn locked_check_reports_remaining_time() {
        let g = LoginGuard::with_params(1, Duration::from_secs(900));
        g.record_failure("dave");
        let remaining = g.check("dave").unwrap_err();
        assert!(remaining <= Duration::from_secs(900));
        assert!(remaining > Duration::from_secs(890), "remaining should be close to full lock window");
    }
}
