use sqlx::any::AnyPoolOptions;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use share_secret::security::{CodeStore, LoginGuard};
use share_secret::{build_router, AppState};
use std::sync::Arc;
use tower::ServiceExt;

/// Postgres 测试需显式提供 TEST_DATABASE_URL（postgres://...），否则跳过。
fn pg_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .filter(|u| u.starts_with("postgres://") || u.starts_with("postgresql://"))
}

/// 证明 `$1` 占位符在 Postgres 上经 Any 可用（Any 不会改写 `?`，故统一用 `$1`），
/// 并能正确解码 i64 / String / Option<String>。
#[tokio::test]
async fn spike_any_postgres_placeholders_and_types() {
    let Some(url) = pg_url() else {
        eprintln!("skipping spike_any_postgres_placeholders_and_types: TEST_DATABASE_URL not set");
        return;
    };
    share_secret::db::install_drivers_once();
    let pool = AnyPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect postgres");

    sqlx::query("DROP TABLE IF EXISTS spike_t")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE spike_t (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT NOT NULL, note TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // INSERT with $1/$2 placeholders — the form that works on both backends via Any.
    sqlx::query("INSERT INTO spike_t (name, note) VALUES ($1, $2)")
        .bind("alice")
        .bind(Option::<String>::None)
        .execute(&pool)
        .await
        .expect("insert with $1/$2 placeholders");

    // SELECT back with a $1 placeholder and decode i64 + String + Option<String>.
    let row: (i64, String, Option<String>) =
        sqlx::query_as("SELECT id, name, note FROM spike_t WHERE name = $1")
            .bind("alice")
            .fetch_one(&pool)
            .await
            .expect("select and decode");

    assert!(row.0 >= 1);
    assert_eq!(row.1, "alice");
    assert_eq!(row.2, None);

    sqlx::query("DROP TABLE spike_t").execute(&pool).await.unwrap();
}

async fn body_string(body: Body) -> String {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// 连接 Postgres，重建空表，返回 app 与 state（带可读 state 引用）。
async fn make_app_pg(url: &str) -> (axum::Router, Arc<AppState>) {
    share_secret::db::install_drivers_once();
    let pool = AnyPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .expect("connect postgres");

    // 干净起步：先删表再按 Postgres schema 重建。
    sqlx::query("DROP TABLE IF EXISTS shares").execute(&pool).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS users").execute(&pool).await.unwrap();
    share_secret::db::init_postgres_schema(&pool).await;

    let state = Arc::new(AppState {
        db: pool,
        codes: CodeStore::new(),
        login_guard: LoginGuard::new(),
    });
    (build_router(state.clone()), state)
}

#[tokio::test]
async fn postgres_end_to_end_flow() {
    let Some(url) = pg_url() else {
        eprintln!("skipping postgres_end_to_end_flow: TEST_DATABASE_URL not set");
        return;
    };
    let (app, state) = make_app_pg(&url).await;

    // 注册（走验证码流程）
    let req = Request::builder()
        .method("POST")
        .uri("/register/code")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=pguser"))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();
    let code = state.codes.peek("pguser").expect("code issued");

    let req = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("username=pguser&password=secret&code={code}")))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // 登录
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=pguser&password=secret"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let cookie = res.headers().get("set-cookie").unwrap().clone();

    // 创建分享
    let req = Request::builder()
        .method("POST")
        .uri("/api/shares")
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"pgcipher"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let slug = serde_json::from_str::<serde_json::Value>(&body_string(res.into_body()).await)
        .unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_string();

    // 匿名读取
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["encrypted_payload"].as_str(), Some("pgcipher"));
    assert_eq!(v["is_owner"].as_bool(), Some(false));

    // 所有者更新
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{slug}/update"))
        .header("content-type", "application/json")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::from(r#"{"encrypted_payload":"pgupdated"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&body_string(res.into_body()).await).unwrap();
    assert_eq!(v["encrypted_payload"].as_str(), Some("pgupdated"));

    // 仪表盘列出该分享（验证 created_at TEXT 能解码为 String）
    let req = Request::builder()
        .uri("/dashboard")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_string(res.into_body()).await.contains(&slug));

    // 删除（按 id；从库里取该 slug 的 BIGINT id，可正常解码为 i64）
    let id: i64 = sqlx::query_scalar("SELECT id FROM shares WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&state.db)
        .await
        .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/shares/{id}/delete"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("cookie", cookie.to_str().unwrap())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // 确认已删除
    let req = Request::builder()
        .uri(format!("/api/shares/{slug}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
