use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    BadRequest(String),
    Conflict(String),
    BadGateway(String),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m),
            ApiError::BadGateway(m) => (StatusCode::BAD_GATEWAY, m),
            ApiError::Internal(e) => {
                tracing::error!("{e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
            }
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError::Internal(e.into())
    }
}
