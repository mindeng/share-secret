use axum::response::Html;

pub async fn index() -> Html<&'static str> {
    Html("")
}

pub async fn dashboard() -> Html<&'static str> {
    Html("")
}
