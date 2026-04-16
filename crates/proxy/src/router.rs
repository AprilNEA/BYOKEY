//! Axum router construction and route registration.
//!
//! All traffic — standard REST AI proxy, Amp CLI compatibility, and
//! `ConnectRPC` management — is served from a single router on one port.
//! The `ConnectRPC` management service is mounted as the router's
//! `fallback_service`, so POST requests to
//! `/byokey.management.ManagementService/{Method}` land there while
//! named routes (amp, REST AI) take priority.

use axum::extract::DefaultBodyLimit;
use axum::{
    Router, http, middleware,
    routing::{any, get, post},
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

fn common_layers(router: Router) -> Router {
    router
        .layer(DefaultBodyLimit::max(200 * 1024 * 1024))
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
        .layer(middleware::from_fn(
            crate::middleware::dump::dump_middleware,
        ))
}

/// Build the unified byokey router.
///
/// Routes served:
/// - `/v1/chat/completions`, `/v1/responses`, `/v1/messages`, `/v1/models`
///   — `OpenAI` / Anthropic compatible REST AI.
/// - `/openapi.json` — REST `OpenAPI` spec (AI endpoints only).
/// - `/auth/cli-login`, `/v1/login` — amp CLI login redirects to
///   `ampcode.com`.
/// - `/api/provider/*` — amp CLI's provider-namespaced AI endpoints.
/// - `/api/{*path}`, `/v0/management/{*path}` — catch-all proxies to
///   `ampcode.com` used by the amp CLI.
/// - `/byokey.management.ManagementService/{Method}` — local byokey
///   management over `ConnectRPC` (fallback service).
///
/// The amp routes are wrapped in [`forward_headers_middleware`] to strip
/// client auth and inject the amp upstream token. The middleware is
/// scoped to that sub-router only via `.layer()` before `.merge()`, so
/// REST and `ConnectRPC` routes are unaffected.
pub fn make_router(state: Arc<AppState>) -> Router {
    // Amp-specific routes with forward_headers_middleware scoped to them.
    let amp_routes = Router::new()
        .route("/auth/cli-login", get(amp::cli_login_redirect))
        .route("/v1/login", get(amp::login_redirect))
        .route("/v0/management/{*path}", any(amp::provider::ampcode_proxy))
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
            post(amp::provider::codex_responses_passthrough),
        )
        .route(
            "/api/provider/google/v1beta/models/{action}",
            post(amp::provider::gemini_native_passthrough),
        )
        .route("/api/{*path}", any(amp::provider::ampcode_proxy))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::forward::forward_headers_middleware,
        ));

    // REST AI proxy routes.
    let rest_routes = Router::new()
        .route("/v1/chat/completions", post(chat::chat_completions))
        .route(
            "/v1/responses",
            post(amp::provider::codex_responses_passthrough),
        )
        .route("/v1/messages", post(messages::anthropic_messages))
        .route("/v1/models", get(models::list_models))
        .route("/openapi.json", get(openapi::openapi_json));

    // `ConnectRPC` management service (served as the fallback).
    let connect_service = management::build_router(state.clone()).into_axum_service();

    let router = rest_routes
        .merge(amp_routes)
        .with_state(state)
        .fallback_service(connect_service);

    common_layers(router)
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
        AppState::with_thread_index(
            config,
            auth,
            None,
            byokey_provider::VersionStore::empty(),
            Arc::new(crate::AmpThreadIndex::empty()),
        )
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
                    .uri("/v1/login")
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
                    .uri("/auth/cli-login?authToken=abc123&callbackPort=35789")
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

    /// Basic sanity check that the `ConnectRPC` management service is
    /// reachable at the expected fallback path.
    #[tokio::test]
    async fn test_management_get_status_reachable() {
        let app = make_router(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/byokey.management.ManagementService/GetStatus")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Whatever the service returns, it should be handled — not a 404.
        assert_ne!(
            resp.status(),
            axum::http::StatusCode::NOT_FOUND,
            "`ConnectRPC` fallback should serve management requests"
        );
    }
}
