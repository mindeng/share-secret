use axum::body::Body;
use axum::http::{Request, StatusCode};
use share_secret::security::{CodeStore, LoginGuard};
use share_secret::{build_router, db::init_db_memory, AppState};
use std::sync::Arc;
use tower::ServiceExt;

async fn body_string(body: Body) -> String {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn make_app() -> (axum::Router, Arc<AppState>) {
    let db = init_db_memory().await;
    let state = Arc::new(AppState {
        db,
        codes: CodeStore::new(),
        login_guard: LoginGuard::new(),
    });
    (build_router(state.clone()), state)
}

/// 走验证码流程注册一个用户，断言成功跳转。
async fn register_user(app: &axum::Router, state: &Arc<AppState>, user: &str, pass: &str) {
    let req = Request::builder()
        .method("POST")
        .uri("/register/code")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username={user}")))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    let code = state.codes.peek(user).expect("code issued");
    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username={user}&password={pass}&code={code}")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

/// 注册并登录，返回会话 cookie。
async fn register_and_login(
    app: &axum::Router,
    state: &Arc<AppState>,
    user: &str,
) -> axum::http::HeaderValue {
    register_user(app, state, user, "secret").await;
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
async fn test_login_cookie_secure_by_default() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "secitest").await;
    let s = cookie.to_str().unwrap().to_lowercase();
    assert!(
        s.contains("secure"),
        "session cookie should be Secure by default: {s}"
    );
}

#[tokio::test]
async fn test_register_and_login() {
    let (app, state) = make_app().await;
    register_user(&app, &state, "alice", "secret").await;

    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=alice&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn test_create_and_fetch_share() {
    let (app, state) = make_app().await;

    register_user(&app, &state, "bob", "secret").await;

    // login
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=bob&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let cookie = res.headers().get("set-cookie").unwrap().clone();

    // create share (slug is generated server-side and returned)
    let payload = r#"{"encrypted_payload":"testpayload"}"#;
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
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let slug = created["slug"].as_str().expect("slug in response");

    // fetch payload by the returned slug
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = body_string(res.into_body()).await;
    let fetched: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(fetched["encrypted_payload"].as_str(), Some("testpayload"));
    assert!(fetched["kdf_salt"].is_null());
}

#[tokio::test]
async fn test_create_share_requires_auth() {
    let (app, _state) = make_app().await;

    let payload = r#"{"encrypted_payload":"testpayload"}"#;
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_fetch_missing_share_returns_404() {
    let (app, _state) = make_app().await;

    let req = Request::builder()
        .uri("/api/shares/doesnotexist")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_password_protected_share_roundtrips_salt() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "carol").await;

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

#[tokio::test]
async fn test_register_requires_valid_code() {
    let (app, _state) = make_app().await;

    // 未获取验证码直接注册 -> 重渲染注册页并提示
    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=eve&password=secret&code=123456"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK); // 非 303 跳转
    let body = body_string(res.into_body()).await;
    assert!(body.contains("请先获取验证码"), "body: {body}");
}

#[tokio::test]
async fn test_register_rejects_wrong_code() {
    let (app, state) = make_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/register/code")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=frank"))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();
    let real = state.codes.peek("frank").expect("code issued");
    // 构造一个保证不同的错误码
    let wrong = if real == "000000" { "111111" } else { "000000" };

    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username=frank&password=secret&code={wrong}")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_string(res.into_body()).await;
    assert!(body.contains("验证码错误"), "body: {body}");
}

#[tokio::test]
async fn test_login_locks_after_failures() {
    let (app, state) = make_app().await;
    register_user(&app, &state, "grace", "secret").await;

    // 连续 5 次错误密码
    for _ in 0..5 {
        let req = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("username=grace&password=wrong"))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK); // 失败重渲染登录页
    }

    // 第 6 次即使密码正确也被锁定
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=grace&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK); // 未跳转 = 被拦
    let body = body_string(res.into_body()).await;
    assert!(body.contains("尝试过于频繁"), "body: {body}");
}

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

async fn create_share_with(app: &axum::Router, cookie: &axum::http::HeaderValue, body: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
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

#[tokio::test]
async fn test_login_trims_username() {
    let (app, state) = make_app().await;
    register_user(&app, &state, "ivan", "secret").await;

    // 登录时用户名带尾随空格（%20），应被 trim 后匹配成功
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=ivan%20&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn test_update_rejects_empty_payload() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "emptyupd").await;
    let slug = create_share_with(&app, &cookie, r#"{"encrypted_payload":"orig"}"#).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":""}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_sets_and_clears_kdf_salt() {
    let (app, state) = make_app().await;
    let cookie = register_and_login(&app, &state, "saltupd").await;
    // start as a link-mode share (no salt)
    let slug = create_share_with(&app, &cookie, r#"{"encrypted_payload":"orig"}"#).await;

    // update -> password mode (sets a salt)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"c1","kdf_salt":"c2FsdHk="}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let req = Request::builder().uri(format!("/api/shares/{slug}")).body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["kdf_salt"].as_str(), Some("c2FsdHk="));

    // update -> back to link mode (clears the salt)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"c2"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let req = Request::builder().uri(format!("/api/shares/{slug}")).body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert!(v["kdf_salt"].is_null());
    assert_eq!(v["encrypted_payload"].as_str(), Some("c2"));
}
