//! Axum router construction and route registration.

use axum::extract::DefaultBodyLimit;
use axum::{
    Router, http, middleware,
    routing::{get, post},
};
use std::sync::Arc;
use std::time::Duration;
use tower_http::classify::ServerErrorsFailureClass;
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::trace::TraceLayer;
use tracing::{Span, info_span};

use crate::handler::{amp, chat, management, messages, models};
use crate::{AppState, openapi};

/// Build the full axum router.
///
/// Top-level routes:
/// - POST /v1/chat/completions                          OpenAI-compatible
/// - POST /v1/messages                                  Anthropic native passthrough
/// - POST /copilot/v1/messages                          Anthropic via Copilot
/// - POST /copilot/v1/chat/completions                  `OpenAI` via Copilot
/// - GET  /v1/models
///
/// Sub-routers:
/// - [`amp::router`]        — Amp CLI / `AmpCode` compatibility (`/amp/*`, `/api/*`).
/// - [`management::router`] — BYOKEY management API, nested at `/v0/management`.
pub fn make_router(state: Arc<AppState>) -> Router {
    Router::new()
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
        .merge(amp::router())
        .nest("/v0/management", management::router())
        .route("/openapi.json", get(openapi::openapi_json))
        .with_state(state)
        .layer(DefaultBodyLimit::max(200 * 1024 * 1024)) // 200 MB for image uploads
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &http::Request<_>| {
                    let request_id = req
                        .extensions()
                        .get::<RequestId>()
                        .and_then(|id| id.header_value().to_str().ok())
                        .unwrap_or("-");
                    info_span!(
                        "http",
                        method = %req.method(),
                        uri = %req.uri(),
                        request_id = request_id,
                    )
                })
                .on_request(|_req: &http::Request<_>, _span: &Span| {
                    tracing::debug!("request received");
                })
                .on_response(
                    |resp: &http::Response<_>, latency: Duration, _span: &Span| {
                        tracing::info!(status = resp.status().as_u16(), ?latency, "response sent");
                    },
                )
                .on_failure(
                    |err: ServerErrorsFailureClass, latency: Duration, _span: &Span| {
                        tracing::error!(
                            error = %err,
                            ?latency,
                            "request failed"
                        );
                    },
                ),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(middleware::from_fn(crate::dump::dump_middleware))
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
        AppState::with_thread_index(config, auth, None, Arc::new(crate::AmpThreadIndex::empty()))
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
