//! HTTP proxy layer — axum router, route handlers, and error mapping.
//!
//! Exposes an OpenAI-compatible `/v1/chat/completions` endpoint, a `/v1/models`
//! listing, and an Amp CLI compatibility layer under `/amp/*`.

mod amp;
mod chat;
mod error;
mod models;

pub use error::ApiError;

use axum::{
    Router,
    routing::{any, get, post},
};
use byokey_auth::AuthManager;
use byokey_config::Config;
use std::sync::Arc;

/// Shared application state passed to all route handlers.
pub struct AppState {
    /// Server configuration (providers, listen address, etc.).
    pub config: Arc<Config>,
    /// Token manager for OAuth-based providers.
    pub auth: Arc<AuthManager>,
    /// HTTP client for upstream requests.
    pub http: rquest::Client,
}

impl AppState {
    /// Creates a new shared application state wrapped in an `Arc`.
    pub fn new(config: Config, auth: Arc<AuthManager>) -> Arc<Self> {
        Arc::new(Self {
            config: Arc::new(config),
            auth,
            http: rquest::Client::new(),
        })
    }
}

/// Build the full axum router.
///
/// Routes:
/// - POST /v1/chat/completions
/// - GET  /v1/models
/// - GET  /amp/v1/login
/// - ANY  /amp/v0/management/{*path}
/// - POST /amp/v1/chat/completions
pub fn make_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat::chat_completions))
        .route("/v1/models", get(models::list_models))
        .route("/amp/v1/login", get(amp::login_redirect))
        .route("/amp/v0/management/{*path}", any(amp::management_proxy))
        .route("/amp/v1/chat/completions", post(chat::chat_completions))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use byokey_store::InMemoryTokenStore;
    use http_body_util::BodyExt as _;
    use serde_json::Value;
    use tower::ServiceExt as _;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        AppState::new(Config::default(), auth)
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn test_list_models_empty_config() {
        let app = make_router(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["object"], "list");
        assert!(json["data"].is_array());
        assert_eq!(json["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_amp_login_redirect() {
        let app = make_router(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/amp/v1/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::FOUND);
        assert_eq!(
            resp.headers().get("location").and_then(|v| v.to_str().ok()),
            Some("https://ampcode.com/login")
        );
    }

    #[tokio::test]
    async fn test_chat_unknown_model_returns_400() {
        use serde_json::json;

        let app = make_router(make_state());
        let body = json!({"model": "nonexistent-model-xyz", "messages": []});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let json = body_json(resp).await;
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("nonexistent-model-xyz")
        );
    }

    #[tokio::test]
    async fn test_chat_missing_model_returns_400() {
        use serde_json::json;

        let app = make_router(make_state());
        let body = json!({"messages": [{"role": "user", "content": "hi"}]});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Empty model string → UnsupportedModel → 400
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_amp_chat_route_exists() {
        use serde_json::json;

        let app = make_router(make_state());
        let body = json!({"model": "nonexistent", "messages": []});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/amp/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Route exists (not 404), even though model is invalid
        assert_ne!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
