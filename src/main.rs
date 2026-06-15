use share_secret::{build_app, db};

#[tokio::main]
async fn main() {
    let db = db::init_db().await;
    let app = build_app(db);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
