// bole-3xj5
//! HTTP error envelope: `{"error":{"code","message"}}` with a matching status.

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
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "bad_request", message: message.into() }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self { status: StatusCode::UNAUTHORIZED, code: "unauthorized", message: message.into() }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, code: "not_found", message: message.into() }
    }
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
        // Library errors are internal by default; handlers translate the
        // "not found" / "forbidden" cases explicitly before this fallback.
        ApiError::internal(e.to_string())
    }
}

impl From<bole::ParseObjectIdError> for ApiError {
    fn from(_: bole::ParseObjectIdError) -> Self {
        ApiError::bad_request("invalid object id (expected 64 hex chars)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn not_found_renders_envelope() {
        let resp = ApiError::not_found("nope").into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["code"], "not_found");
        assert_eq!(json["error"]["message"], "nope");
    }
}
