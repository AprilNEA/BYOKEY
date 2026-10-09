use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use byokey_auth::AuthManager;
use byokey_config::{Config, ConfigValue, ProviderConfig};
use byokey_store::InMemoryTokenStore;
use byokey_types::ProviderId;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::mpsc;
use tower::ServiceExt as _;

use crate::AppState;

struct Upstream {
    url: String,
    received: mpsc::UnboundedReceiver<(String, Value)>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Upstream {
    async fn start(status: StatusCode, body: &'static str) -> Self {
        let (tx, received) = mpsc::unbounded_channel();
        let router = Router::new().fallback(move |request: Request<Body>| {
            let tx = tx.clone();
            async move {
                let (parts, body_bytes) = request.into_parts();
                let bytes = to_bytes(body_bytes, usize::MAX).await.unwrap();
                let payload = serde_json::from_slice(&bytes).unwrap();
                tx.send((parts.uri.path().to_owned(), payload)).unwrap();
                Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .header("retry-after", "17")
                    .body(Body::from(body))
                    .unwrap()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url,
            received,
            task,
        }
    }

    fn config(&self) -> Config {
        let mut config = Config::default();
        config.anthropic.routes.default = Some("copilot".into());
        config.providers.insert(
            ProviderId::Copilot.to_string(),
            ProviderConfig {
                api_key: Some(ConfigValue::Literal(uuid::Uuid::new_v4().to_string())),
                base_url: Some(self.url.clone()),
                ..Default::default()
            },
        );
        config
    }
}

fn state(config: Config) -> Arc<AppState> {
    let http = crate::http::upstream_client(None).unwrap();
    AppState::new(
        Arc::new(arc_swap::ArcSwap::from_pointee(config)),
        Arc::new(AuthManager::new(
            Arc::new(InMemoryTokenStore::new()),
            http.clone(),
        )),
        http,
        None,
    )
    .unwrap()
}

fn request(path: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn disabled_routes_reject_generation_and_counting_before_authentication() {
    let cases = [
        (ProviderId::Claude, "", "claude-opus-5-5"),
        (
            ProviderId::Copilot,
            "anthropic:\n  routes:\n    default: copilot",
            "unknown-model",
        ),
        (
            ProviderId::Cursor,
            "anthropic:\n  routes:\n    default: cursor",
            "claude-opus-5-5",
        ),
        (
            ProviderId::Copilot,
            "anthropic:\n  routes:\n    families:\n      opus: copilot",
            "claude-opus-5-5",
        ),
        (
            ProviderId::Cursor,
            "anthropic:\n  routes:\n    models:\n      claude-opus-5-5: cursor",
            "claude-opus-5-5[1m]",
        ),
        (
            ProviderId::Copilot,
            "anthropic:\n  routes:\n    default: cursor",
            "copilot/claude-opus-5.5",
        ),
        (
            ProviderId::Cursor,
            "anthropic:\n  routes:\n    default: copilot",
            "cursor/claude-opus-5-5",
        ),
    ];
    for (provider, routes, model) in cases {
        let mut config = Config::from_yaml(routes).unwrap();
        config.providers.insert(
            provider.to_string(),
            ProviderConfig {
                enabled: false,
                ..Default::default()
            },
        );
        let app = crate::make_router(state(config));
        for path in ["/v1/messages", "/v1/messages/count_tokens"] {
            let response = app
                .clone()
                .oneshot(request(
                    path,
                    &json!({
                        "model": model, "max_tokens": 16,
                        "messages": [{"role": "user", "content": "Explain ownership."}]
                    }),
                ))
                .await
                .unwrap();

            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{provider} {model} {path}"
            );
            assert_eq!(
                json_body(response).await,
                json!({
                    "type": "error",
                    "error": {"type": "invalid_request_error", "message": format!("unsupported provider: {provider} is disabled")}
                })
            );
        }
    }
}

#[tokio::test]
async fn disabling_a_keyed_provider_on_reload_stops_upstream_requests() {
    let mut upstream =
        Upstream::start(StatusCode::OK, r#"{"content":[],"stop_reason":"end_turn"}"#).await;
    let config = upstream.config();
    let state = state(config.clone());
    let app = crate::make_router(state.clone());
    let body = json!({"model": "copilot/claude-opus-5-5", "max_tokens": 16,
        "messages": [{"role": "user", "content": "Explain ownership."}]});
    let first = app
        .clone()
        .oneshot(request("/v1/messages", &body))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    upstream.received.try_recv().unwrap();

    let mut disabled = config;
    disabled.providers.get_mut("copilot").unwrap().enabled = false;
    state.config.store(Arc::new(disabled));
    let response = app
        .clone()
        .oneshot(request("/v1/messages", &body))
        .await
        .unwrap();
    let count = app
        .oneshot(request("/v1/messages/count_tokens", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(count.status(), StatusCode::BAD_REQUEST);
    assert!(upstream.received.try_recv().is_err());
}

#[tokio::test]
async fn copilot_preserves_the_requested_model() {
    let mut upstream =
        Upstream::start(StatusCode::OK, r#"{"content":[],"stop_reason":"end_turn"}"#).await;
    let config = upstream.config();
    let app = crate::make_router(state(config));
    let body = json!({"model": "claude-opus-5-5", "max_tokens": 16, "tools": [],
        "messages": [{"role": "user", "content": "Explain ownership."}]});

    let response = app.oneshot(request("/v1/messages", &body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        upstream.received.try_recv().unwrap(),
        ("/v1/messages".into(), body)
    );
    assert!(upstream.received.try_recv().is_err());
}

#[tokio::test]
async fn copilot_policy_errors_do_not_retry_or_remove_tools_from_later_requests() {
    const ERROR: &str = r#"{"error":{"message":"The use of the web search tool is not supported.","code":"unsupported_value"}}"#;
    let mut upstream = Upstream::start(StatusCode::BAD_REQUEST, ERROR).await;
    let state = state(upstream.config());
    let app = crate::make_router(state.clone());
    let tools = json!([
        {"type": "web_search_20250305", "name": "web_search", "max_uses": 3},
        {"type": "web_fetch_20250910", "name": "web_fetch"},
        {"name": "lookup", "input_schema": {"type": "object"}}
    ]);
    for (path, stream) in [
        ("/v1/messages", false),
        ("/v1/messages", true),
        ("/v1/messages/count_tokens", false),
    ] {
        let body = json!({"model": "claude-opus-5-5", "max_tokens": 16, "stream": stream,
            "messages": [{"role": "user", "content": "Search the documentation."}], "tools": tools});

        let response = app.clone().oneshot(request(path, &body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["retry-after"], "17");
        assert_eq!(
            json_body(response).await,
            serde_json::from_str::<Value>(ERROR).unwrap()
        );
        assert_eq!(upstream.received.try_recv().unwrap(), (path.into(), body));
        assert!(
            upstream.received.try_recv().is_err(),
            "a policy rejection must not trigger a modified retry"
        );
    }
    assert_eq!(state.usage.snapshot().failure_requests, 2);
}
