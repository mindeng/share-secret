use axum::body::Body;
use axum::http::{Request, StatusCode};
use share_secret::{build_app, db::init_db_memory};
use tower::ServiceExt;

async fn body_string(body: Body) -> String {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn test_login_cookie_secure_by_default() {
    // Session cookies are marked `Secure` by default (safe for HTTPS deployments).
    // For plain-HTTP local dev, SECURE_COOKIES=false disables it; see build_app.
    let db = init_db_memory().await;
    let app = build_app(db);
    let cookie = register_and_login(&app, "secitest").await;
    let s = cookie.to_str().unwrap().to_lowercase();
    assert!(
        s.contains("secure"),
        "session cookie should be Secure by default: {s}"
    );
}

#[tokio::test]
async fn test_register_and_login() {
    let db = init_db_memory().await;
    let app = build_app(db);

    let register_req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=alice&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(register_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

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
    let db = init_db_memory().await;
    let app = build_app(db);

    // register
    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=bob&password=secret"))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

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
    let db = init_db_memory().await;
    let app = build_app(db);

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
    let db = init_db_memory().await;
    let app = build_app(db);

    let req = Request::builder()
        .uri("/api/shares/doesnotexist")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

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
