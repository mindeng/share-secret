use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use askama::Error as AskamaError;

#[derive(Debug)]
pub enum AppError {
    Db(sqlx::Error),
    Template(AskamaError),
    Auth(&'static str),
    NotFound,
    Forbidden,
    BadRequest(&'static str),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, "数据库错误"),
            AppError::Template(_) => (StatusCode::INTERNAL_SERVER_ERROR, "模板渲染错误"),
            AppError::Auth(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::NotFound => (StatusCode::NOT_FOUND, "页面不存在"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "无权操作"),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
        };
        (status, message).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self { AppError::Db(e) }
}

impl From<AskamaError> for AppError {
    fn from(e: AskamaError) -> Self { AppError::Template(e) }
}
