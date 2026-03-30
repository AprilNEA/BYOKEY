//! Axum router construction and route registration.

use axum::extract::DefaultBodyLimit;
use axum::{
    Router,
    routing::{any, delete, get, post},
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::handler::{
    accounts, amp, amp_provider, amp_quota, amp_threads, chat, messages, models, ratelimits,
    status, usage,
};
use crate::{AppState, openapi};

/// Build the full axum router.
///
/// Routes:
/// - POST /v1/chat/completions                          OpenAI-compatible
/// - POST /v1/messages                                  Anthropic native passthrough
/// - POST /copilot/v1/messages                          Anthropic via Copilot
/// - GET  /v1/models
/// - GET  /amp/v1/login
/// - ANY  /amp/v0/management/{*path}
/// - POST /amp/v1/chat/completions
///
/// `AmpCode` provider routes:
/// - POST /api/provider/anthropic/v1/messages           Anthropic native (`AmpCode`)
/// - POST /api/provider/openai/v1/chat/completions      `OpenAI`-compatible (`AmpCode`)
/// - POST /api/provider/openai/v1/responses             Codex Responses API (`AmpCode`)
/// - POST /api/provider/google/v1beta/models/{action}   Gemini native (`AmpCode`)
/// - ANY  /api/{*path}                                  `ampcode.com` management proxy
pub fn make_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Standard routes
        .route("/v1/chat/completions", post(chat::chat_completions))
        .route("/v1/messages", post(messages::anthropic_messages))
        .route(
            "/copilot/v1/messages",
            post(messages::copilot_anthropic_messages),
        )
        .route(
            "/copilot/v1/chat/completions",
            post(chat::copilot_chat_completions),
        )
        .route("/v1/models", get(models::list_models))
        // Amp CLI routes
        .route("/amp/auth/cli-login", get(amp::cli_login_redirect))
        .route("/amp/v1/login", get(amp::login_redirect))
        .route("/amp/v0/management/{*path}", any(amp::management_proxy))
        .route("/amp/v1/chat/completions", post(chat::chat_completions))
        // AmpCode provider-specific routes (must be registered before the catch-all)
        .route(
            "/api/provider/anthropic/v1/messages",
            post(messages::anthropic_messages),
        )
        .route(
            "/api/provider/openai/v1/chat/completions",
            post(chat::chat_completions),
        )
        .route(
            "/api/provider/openai/v1/responses",
            post(amp_provider::codex_responses_passthrough),
        )
        .route(
            "/api/provider/google/v1beta/models/{action}",
            post(amp_provider::gemini_native_passthrough),
        )
        // Catch-all: forward remaining /api/* routes to ampcode.com
        .route("/api/{*path}", any(amp_provider::amp_management_proxy))
        // Management API
        .route("/v0/management/status", get(status::status_handler))
        .route("/v0/management/usage", get(usage::usage_handler))
        .route(
            "/v0/management/usage/history",
            get(usage::usage_history_handler),
        )
        .route("/v0/management/accounts", get(accounts::accounts_handler))
        .route(
            "/v0/management/accounts/{provider}/{account_id}",
            delete(accounts::remove_account_handler),
        )
        .route(
            "/v0/management/accounts/{provider}/{account_id}/activate",
            post(accounts::activate_account_handler),
        )
        .route(
            "/v0/management/ratelimits",
            get(ratelimits::ratelimits_handler),
        )
        .route(
            "/v0/management/amp/quota",
            get(amp_quota::amp_quota_handler),
        )
        .route("/v0/management/amp/threads", get(amp_threads::list_threads))
        .route(
            "/v0/management/amp/threads/{id}",
            get(amp_threads::get_thread),
        )
        .route("/openapi.json", get(openapi::openapi_json))
        .with_state(state)
        .layer(DefaultBodyLimit::max(200 * 1024 * 1024)) // 200 MB for image uploads
        .layer(TraceLayer::new_for_http())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use byokey_auth::AuthManager;
    use byokey_store::InMemoryTokenStore;
    use http_body_util::BodyExt as _;
    use serde_json::Value;
    use tower::ServiceExt as _;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store, rquest::Client::new()));
        let config = Arc::new(arc_swap::ArcSwap::from_pointee(
            byokey_config::Config::default(),
        ));
        AppState::new(config, auth, None)
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
        // All providers are enabled by default even without explicit config.
        assert!(!json["data"].as_array().unwrap().is_empty());
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
    async fn test_amp_cli_login_redirect() {
        let app = make_router(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/amp/auth/cli-login?authToken=abc123&callbackPort=35789")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::FOUND);
        assert_eq!(
            resp.headers().get("location").and_then(|v| v.to_str().ok()),
            Some("https://ampcode.com/auth/cli-login?authToken=abc123&callbackPort=35789")
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
    async fn test_chat_missing_model_returns_422() {
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

        // Missing required `model` field → axum JSON rejection → 422
        assert_eq!(resp.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
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
