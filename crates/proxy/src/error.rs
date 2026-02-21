//! API error type that maps [`ByokError`] variants to HTTP status codes.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_types::ByokError;
use serde_json::json;

/// Wrapper around [`ByokError`] that implements [`IntoResponse`].
pub struct ApiError(pub ByokError);

impl ApiError {
    /// Returns `(status, error_type, error_code)` for the wrapped error.
    fn classify(&self) -> (StatusCode, &'static str, &'static str) {
        match &self.0 {
            ByokError::Auth(_) => {
                (StatusCode::UNAUTHORIZED, "authentication_error", "invalid_api_key")
            }
            ByokError::TokenNotFound(_) | ByokError::TokenExpired(_) => {
                (StatusCode::UNAUTHORIZED, "authentication_error", "token_not_found")
            }
            ByokError::UnsupportedModel(_) => {
                (StatusCode::BAD_REQUEST, "invalid_request_error", "model_not_found")
            }
            ByokError::Translation(_) => {
                (StatusCode::BAD_REQUEST, "invalid_request_error", "translation_error")
            }
            ByokError::Http(m) => classify_http(m),
            _ => {
                (StatusCode::INTERNAL_SERVER_ERROR, "server_error", "internal_error")
            }
        }
    }
}

fn classify_http(msg: &str) -> (StatusCode, &'static str, &'static str) {
    if msg.contains("429") {
        (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", "rate_limit_exceeded")
    } else if msg.contains("401") {
        (StatusCode::UNAUTHORIZED, "authentication_error", "invalid_api_key")
    } else if msg.contains("403") {
        (StatusCode::FORBIDDEN, "permission_error", "insufficient_quota")
    } else {
        (StatusCode::BAD_GATEWAY, "server_error", "upstream_error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_type, error_code) = self.classify();
        let msg = self.0.to_string();
        (
            status,
            Json(json!({
                "error": {
                    "message": msg,
                    "type": error_type,
                    "code": error_code,
                }
            })),
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
    use http_body_util::BodyExt as _;

    async fn extract_error_body(err: ApiError) -> (StatusCode, serde_json::Value) {
        let resp = err.into_response();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, body)
    }

    #[tokio::test]
    async fn test_auth_error() {
        let (status, body) = extract_error_body(ApiError(ByokError::Auth("bad creds".into()))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["type"], "authentication_error");
        assert_eq!(body["error"]["code"], "invalid_api_key");
    }

    #[tokio::test]
    async fn test_token_not_found_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::TokenNotFound(ProviderId::Claude))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["type"], "authentication_error");
        assert_eq!(body["error"]["code"], "token_not_found");
    }

    #[tokio::test]
    async fn test_unsupported_model_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::UnsupportedModel("xyz".into()))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["code"], "model_not_found");
    }

    #[tokio::test]
    async fn test_translation_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::Translation("bad format".into()))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["code"], "translation_error");
    }

    #[tokio::test]
    async fn test_http_429_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::Http("status 429 too many".into()))).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error"]["type"], "rate_limit_error");
        assert_eq!(body["error"]["code"], "rate_limit_exceeded");
    }

    #[tokio::test]
    async fn test_http_401_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::Http("status 401 unauthorized".into()))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["type"], "authentication_error");
        assert_eq!(body["error"]["code"], "invalid_api_key");
    }

    #[tokio::test]
    async fn test_http_403_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::Http("status 403 forbidden".into()))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["type"], "permission_error");
        assert_eq!(body["error"]["code"], "insufficient_quota");
    }

    #[tokio::test]
    async fn test_http_other_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::Http("upstream error".into()))).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"]["type"], "server_error");
        assert_eq!(body["error"]["code"], "upstream_error");
    }

    #[tokio::test]
    async fn test_internal_error() {
        let (status, body) =
            extract_error_body(ApiError(ByokError::Config("bad config".into()))).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["type"], "server_error");
        assert_eq!(body["error"]["code"], "internal_error");
    }
}
