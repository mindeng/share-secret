use share_secret::{build_app, db};

#[tokio::main]
async fn main() {
    let db = db::init_db().await;
    let app = build_app(db);

    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
