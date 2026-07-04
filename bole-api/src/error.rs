// bole-3xj5
//! HTTP error envelope. Expanded in Task 2; this minimal version lets Task 1
//! handlers compile and return a JSON body on failure.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, code: "internal", message: message.into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": { "code": self.code, "message": self.message } }));
        (self.status, body).into_response()
    }
}

impl From<bole::Error> for ApiError {
    fn from(e: bole::Error) -> Self {
        ApiError::internal(e.to_string())
    }
}
