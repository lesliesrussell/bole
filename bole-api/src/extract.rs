// bole-rvyl
//! Envelope-preserving wrappers around axum's `Path`/`Query` extractors: their
//! built-in rejections return bare text bodies, which would be the only
//! non-JSON error surface in the API. These delegate and map any rejection to
//! the standard `ApiError` envelope.

use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;

use crate::error::ApiError;

pub struct ApiPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: serde::de::DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(v)) => Ok(ApiPath(v)),
            // bole-i8zl: keep the rejection's own status (MissingPathParams is a
            // 500-class server/config error, not a client 400) and use a
            // generic message so serde-internal detail is not surfaced.
            Err(rej) => Err(ApiError::from_status(rej.status(), "invalid path parameter")),
        }
    }
}

pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: serde::de::DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(v)) => Ok(ApiQuery(v)),
            // bole-i8zl: preserve the rejection status; generic message.
            Err(rej) => Err(ApiError::from_status(rej.status(), "invalid query parameter")),
        }
    }
}
