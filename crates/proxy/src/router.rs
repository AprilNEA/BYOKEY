//! Axum router construction and route registration.
//!
//! All traffic — the REST AI proxy and `ConnectRPC` management — is served
//! from a single router on one port. The `ConnectRPC` management services
//! are mounted as the router's `fallback_service`, so POST requests to
//! `/byokey.status.StatusService/{Method}` or
//! `/byokey.accounts.AccountsService/{Method}` land there while the named
//! REST routes take priority.

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

use crate::AppState;
use crate::handler::{count_tokens, management, messages, models};

fn common_layers(router: Router) -> Router {
    // Sentry layers are added as the outermost wrapping, so a hub is bound
    // for every request before any other instrumentation runs. For axum,
    // `.layer()` applies bottom-up; the *last* `.layer()` call is the
    // outermost, so `NewSentryLayer` goes after `SentryHttpLayer`.
    // `SentryHttpLayer::new()` does NOT enable transactions — we only want
    // request context attached to error events, not performance spans that
    // could capture AI proxy traffic.
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
                // `ApiError` logs each failure with its cause, at the level
                // that says who failed; this only notes the classification.
                .on_failure(
                    |err: ServerErrorsFailureClass, latency: Duration, _span: &Span| {
                        tracing::debug!(error = %err, ?latency, "response classified as a failure");
                    },
                ),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(middleware::from_fn(
            crate::middleware::dump::dump_middleware,
        ))
        .layer(sentry::integrations::tower::SentryHttpLayer::new())
        .layer(sentry::integrations::tower::NewSentryLayer::<
            http::Request<axum::body::Body>,
        >::new_from_top())
}

/// Build the unified byokey router.
///
/// Routes served:
/// - `/v1/messages`, `/v1/messages/count_tokens`, `/v1/models` — the
///   Anthropic Messages API.
/// - `/byokey.status.StatusService/{Method}`,
///   `/byokey.accounts.AccountsService/{Method}` — local byokey management
///   over `ConnectRPC` (fallback service).
pub fn make_router(state: Arc<AppState>) -> Router {
    let rest_routes = Router::new()
        .route("/v1/messages", post(messages::anthropic_messages))
        .route(
            "/v1/messages/count_tokens",
            post(count_tokens::count_tokens),
        )
        .route("/v1/models", get(models::list_models));

    // `ConnectRPC` management service (served as the fallback).
    let connect_service = management::build_router(state.clone()).into_axum_service();

    let router = rest_routes
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
        let http = reqwest::Client::new();
        let auth = Arc::new(AuthManager::new(store, http.clone()));
        let config = Arc::new(arc_swap::ArcSwap::from_pointee(
            byokey_config::Config::default(),
        ));
        AppState::new(
            config,
            auth,
            http,
            None,
            byokey_provider::CopilotIdentity::default(),
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
        // Nothing is signed in or keyed, so nothing is usable.
        assert!(json["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn messages_without_a_login_fail_in_the_anthropic_envelope() {
        use serde_json::json;

        let app = make_router(make_state());
        let body = json!({"model": "claude-sonnet-5", "max_tokens": 1, "messages": []});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
        let json = body_json(resp).await;
        assert_eq!(json["type"], "error");
        assert_eq!(json["error"]["type"], "authentication_error");
    }

    #[tokio::test]
    async fn chat_completions_is_gone() {
        let app = make_router(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
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
                    .uri("/byokey.status.StatusService/GetStatus")
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
