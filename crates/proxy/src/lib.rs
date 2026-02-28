//! HTTP proxy layer — axum router, route handlers, and error mapping.
//!
//! Exposes an OpenAI-compatible `/v1/chat/completions` endpoint, a `/v1/models`
//! listing, and an Amp CLI compatibility layer under `/amp/*`.

mod amp;
mod amp_provider;
mod chat;
mod error;
mod messages;
mod models;
pub mod usage;

pub use error::ApiError;
pub use usage::UsageStats;

use arc_swap::ArcSwap;
use axum::{
    Json, Router,
    extract::State,
    routing::{any, get, post},
};
use byokey_auth::AuthManager;
use byokey_config::Config;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

/// Shared application state passed to all route handlers.
pub struct AppState {
    /// Server configuration (providers, listen address, etc.).
    /// Atomically swappable for hot-reloading.
    pub config: Arc<ArcSwap<Config>>,
    /// Token manager for OAuth-based providers.
    pub auth: Arc<AuthManager>,
    /// HTTP client for upstream requests.
    pub http: rquest::Client,
    /// In-memory usage statistics.
    pub usage: Arc<UsageStats>,
}

impl AppState {
    /// Creates a new shared application state wrapped in an `Arc`.
    ///
    /// If the config specifies a `proxy_url`, the HTTP client is built with that proxy.
    pub fn new(config: Arc<ArcSwap<Config>>, auth: Arc<AuthManager>) -> Arc<Self> {
        let snapshot = config.load();
        let http = build_http_client(snapshot.proxy_url.as_deref());
        Arc::new(Self {
            config,
            auth,
            http,
            usage: Arc::new(UsageStats::new()),
        })
    }
}

/// Build an HTTP client, optionally configured with a proxy URL.
fn build_http_client(proxy_url: Option<&str>) -> rquest::Client {
    if let Some(url) = proxy_url {
        match rquest::Proxy::all(url) {
            Ok(proxy) => {
                return rquest::Client::builder()
                    .proxy(proxy)
                    .build()
                    .unwrap_or_else(|_| rquest::Client::new());
            }
            Err(e) => {
                tracing::warn!(url = url, error = %e, "invalid proxy_url, using direct connection");
            }
        }
    }
    rquest::Client::new()
}

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
        .route("/v0/management/usage", get(usage_handler))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn usage_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let snap = state.usage.snapshot();
    Json(serde_json::to_value(snap).unwrap_or_default())
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
        let auth = Arc::new(AuthManager::new(store, rquest::Client::new()));
        let config = Arc::new(ArcSwap::from_pointee(Config::default()));
        AppState::new(config, auth)
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
