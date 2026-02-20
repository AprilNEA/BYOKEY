//! API error type that maps [`ByokError`] variants to HTTP status codes.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_types::ByokError;
use serde_json::json;

/// Wrapper around [`ByokError`] that implements [`IntoResponse`].
///
/// Maps error variants to appropriate HTTP status codes:
/// - `UnsupportedModel` -> 400
/// - `TokenNotFound` / `TokenExpired` -> 401
/// - `Http` -> 502
/// - Everything else -> 500
pub struct ApiError(pub ByokError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, msg) = match &self.0 {
            ByokError::UnsupportedModel(m) => {
                (StatusCode::BAD_REQUEST, format!("unsupported model: {m}"))
            }
            ByokError::TokenNotFound(_) | ByokError::TokenExpired(_) => {
                (StatusCode::UNAUTHORIZED, self.0.to_string())
            }
            ByokError::Http(m) => (StatusCode::BAD_GATEWAY, m.clone()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()),
        };
        (
            code,
            Json(json!({"error": {"message": msg, "type": "byokey_error"}})),
        )
            .into_response()
    }
}

impl From<ByokError> for ApiError {
    fn from(e: ByokError) -> Self {
        Self(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_types::ProviderId;

    #[test]
    fn test_unsupported_model_is_bad_request() {
        let err = ApiError(ByokError::UnsupportedModel("xyz".into()));
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_token_not_found_is_unauthorized() {
        let err = ApiError(ByokError::TokenNotFound(ProviderId::Claude));
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_http_error_is_bad_gateway() {
        let err = ApiError(ByokError::Http("upstream error".into()));
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }
}
