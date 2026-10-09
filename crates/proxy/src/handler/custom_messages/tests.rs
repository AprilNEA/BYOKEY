use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
    response::Response,
};
use byokey_auth::AuthManager;
use byokey_config::Config;
use byokey_store::InMemoryTokenStore;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::mpsc;
use tower::ServiceExt as _;

use crate::AppState;

type Captured = (String, HeaderMap, Value);

struct Upstream {
    url: String,
    received: mpsc::UnboundedReceiver<Captured>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

const MESSAGE: &str = r#"{"id":"msg_1","type":"message","content":[{"type":"thinking","thinking":"check","signature":"opaque-router-signature"},{"type":"text","text":"OK"}],"stop_reason":"end_turn","usage":{"input_tokens":17,"output_tokens":3},"future_response":{"kept":true}}"#;

impl Upstream {
    async fn start(status: StatusCode, body: &'static str) -> Self {
        let (tx, received) = mpsc::unbounded_channel();
        let router = Router::new().fallback(move |request: Request<Body>| {
            let tx = tx.clone();
            async move {
                let (parts, body_bytes) = request.into_parts();
                let bytes = to_bytes(body_bytes, usize::MAX).await.unwrap();
                let payload = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
                let path = parts.uri.to_string();
                let result = match path.as_str() {
                    "/copilot/models" => r#"{"data":[{"id":"claude-opus-5.5","name":"Opus","model_picker_enabled":true,"supported_endpoints":["/v1/messages"]}]}"#,
                    "/router/models" => r#"{"data":[{"id":"claude-opus-5-5"},{"id":"claude-opus-5.5"},{"id":"claude-sonnet-4-6-stable-max","display_name":"Sonnet Stable"},{"id":"gpt-example"}]}"#,
                    "/api/v1/messages/count_tokens" => r#"{"input_tokens":23}"#,
                    _ => body,
                };
                tx.send((path, parts.headers, payload)).unwrap();
                Response::builder().status(status)
                    .header("content-type", if result.starts_with("event:") { "text/event-stream" } else { "application/json" })
                    .header("retry-after", "17")
                    .body(Body::from(result)).unwrap()
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
        serde_json::from_value(json!({
            "anthropic": {"routes": {"default": "copilot"}},
            "providers": {
                "claude": {"api_key": "private-claude-key"},
                "copilot": {"base_url": format!("{}/copilot", self.url), "api_key": uuid::Uuid::new_v4().to_string()},
                "llm-router": {
                    "base_url": format!("{}/openai/v1", self.url),
                    "api_key": "private-responses-key",
                    "headers": {"x-specified-llm-provider-name": "openai"},
                    "display_name": "LLM Router",
                    "anthropic": {
                        "base_url": format!("{}/api/", self.url),
                        "models_url": format!("{}/router/models", self.url),
                        "api_key": "default-message-key",
                        "headers": {
                            "X-Api-Key": "configured-message-key",
                            "x-request-resource-group": "5",
                            "x-request-task-type": "test",
                            "x-request-task-uid": {"uuid_prefix": "message-"},
                            "x-request-product-name": "byokey-test",
                            "x-request-options": "{\"account_details\":\"1\"}",
                            "x-from-environment": {"env": "PATH"},
                            "Connection": "x-hop",
                            "x-hop": "must-not-leak"
                        }
                    }
                }
            }
        })).unwrap()
    }
}

fn state(config: Config) -> Arc<AppState> {
    config.validate().unwrap();
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
        .header("authorization", "Bearer private-client-token")
        .header("x-api-key", "private-client-key")
        .header("cookie", "private-cookie")
        .header("anthropic-beta", "client-beta")
        .header("x-request-resource-group", "attacker-group")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn custom_catalogs_show_separate_models_without_duplicating_the_copilot_default() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let app = crate::make_router(state(upstream.config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let models: Vec<_> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| {
            (
                model["id"].as_str().unwrap(),
                model["display_name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        models,
        [
            ("claude-opus-5-5", "Claude Opus 5.5 · Copilot"),
            (
                "claude-opus-5-5[llm-router]",
                "Claude Opus 5.5 · LLM Router"
            ),
            (
                "claude-opus-5.5[llm-router]",
                "Claude Opus 5.5 · LLM Router"
            ),
            (
                "claude-sonnet-4-6-stable-max[llm-router]",
                "Sonnet Stable · LLM Router"
            )
        ]
    );
    let requests = [
        upstream.received.recv().await.unwrap(),
        upstream.received.recv().await.unwrap(),
    ];
    let (_, headers, _) = requests
        .iter()
        .find(|(path, _, _)| path == "/router/models")
        .unwrap();
    assert_eq!(headers["x-request-resource-group"], "5");
    assert_eq!(headers["x-api-key"], "configured-message-key");
    assert!(!headers.contains_key("authorization"));
    assert!(!headers.contains_key("x-specified-llm-provider-name"));
}

#[tokio::test]
async fn custom_catalog_allowlists_match_exact_ids_without_filtering_copilot() {
    let upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = serde_json::to_value(upstream.config()).unwrap();
    config["providers"]["llm-router"]["anthropic"]["enabled_models"] =
        json!(["claude-opus-5-5", "claude-not-discovered"]);
    let app = crate::make_router(state(serde_json::from_value(config).unwrap()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let ids: Vec<_> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["claude-opus-5-5", "claude-opus-5-5[llm-router]"]);
}

#[tokio::test]
async fn configured_models_without_discovery_keep_raw_ids_and_copilot() {
    let upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = serde_json::to_value(upstream.config()).unwrap();
    let connection = &mut config["providers"]["llm-router"]["anthropic"];
    connection["models_url"] = Value::Null;
    connection["base_url"] = json!("http://127.0.0.1:1/api");
    connection["enabled_models"] = json!(["claude-opus-5-5", "claude-private-max", "gpt-example"]);
    let app = crate::make_router(state(serde_json::from_value(config).unwrap()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let ids: Vec<_> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "claude-opus-5-5",
            "claude-opus-5-5[llm-router]",
            "claude-private-max[llm-router]"
        ]
    );
}

#[tokio::test]
async fn an_empty_custom_catalog_allowlist_hides_only_that_provider() {
    let upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = serde_json::to_value(upstream.config()).unwrap();
    config["providers"]["llm-router"]["anthropic"]["enabled_models"] = json!([]);
    let app = crate::make_router(state(serde_json::from_value(config).unwrap()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"][0]["id"], "claude-opus-5-5");
    assert_eq!(body["data"][0]["owned_by"], "copilot");
}

#[tokio::test]
async fn native_messages_keep_signatures_and_unknown_fields_with_only_configured_credentials() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let state = state(upstream.config());
    let app = crate::make_router(state.clone());
    let response = app.oneshot(request("/v1/messages", &json!({
        "model":"llm-router/claude-opus-5-5", "max_tokens":32,
        "messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"prior","signature":"opaque-router-signature"}]}],
        "future_request":{"kept":true}, "tools":[{"name":"Read","input_schema":{"type":"object"}}]
    }))).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await,
        serde_json::from_str::<Value>(MESSAGE).unwrap()
    );
    let (path, headers, payload) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/api/v1/messages");
    assert_eq!(
        payload,
        json!({
            "model":"claude-opus-5-5", "max_tokens":32,
            "messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"prior","signature":"opaque-router-signature"}]}],
            "future_request":{"kept":true}, "tools":[{"name":"Read","input_schema":{"type":"object"}}]
        })
    );
    assert_eq!(headers["x-api-key"], "configured-message-key");
    assert_eq!(headers.get_all("x-api-key").iter().count(), 1);
    assert_eq!(headers["x-request-resource-group"], "5");
    assert_eq!(headers["x-request-task-type"], "test");
    assert_eq!(headers["x-request-product-name"], "byokey-test");
    assert_eq!(headers["x-request-options"], "{\"account_details\":\"1\"}");
    assert_eq!(
        headers["x-from-environment"],
        std::env::var("PATH").unwrap()
    );
    assert_eq!(headers["anthropic-beta"], "client-beta");
    assert!(!headers.contains_key("authorization"));
    assert!(!headers.contains_key("cookie"));
    assert!(!headers.contains_key("x-specified-llm-provider-name"));
    assert!(!headers.contains_key("x-hop"));
    assert_eq!(state.usage.snapshot().input_tokens, 17);
}

#[tokio::test]
async fn custom_messages_and_counting_omit_unrequested_betas() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let app = crate::make_router(state(upstream.config()));
    let body = json!({
        "model":"claude-opus-5-5[llm-router]", "max_tokens":32,
        "messages":[{"role":"user","content":"hi"}]
    });
    let mut message = request("/v1/messages", &body);
    message.headers_mut().remove("anthropic-beta");
    let mut count = request("/v1/messages/count_tokens", &body);
    count.headers_mut().remove("anthropic-beta");

    let response = app.clone().oneshot(message).await.unwrap();
    let counted = app.oneshot(count).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counted.status(), StatusCode::OK);
    let (path, headers, _) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/api/v1/messages");
    assert!(!headers.contains_key("anthropic-beta"));
    let (path, headers, _) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/api/v1/messages/count_tokens");
    assert!(!headers.contains_key("anthropic-beta"));
}

#[tokio::test]
async fn configured_beta_header_replaces_client_and_long_context_betas() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = upstream.config();
    config
        .providers
        .get_mut("llm-router")
        .unwrap()
        .anthropic
        .as_mut()
        .unwrap()
        .headers
        .insert(
            "Anthropic-Beta".into(),
            byokey_config::ConfigValue::Literal("gateway-beta".into()),
        );
    let app = crate::make_router(state(config));

    let response = app
        .oneshot(request(
            "/v1/messages",
            &json!({
                "model":"claude-opus-5-5[llm-router][1m]", "max_tokens":32,
                "messages":[{"role":"user","content":"hi"}], "betas":["body-beta"]
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let (_, headers, body) = upstream.received.recv().await.unwrap();
    assert_eq!(headers["anthropic-beta"], "gateway-beta");
    assert_eq!(headers.get_all("anthropic-beta").iter().count(), 1);
    assert!(body.get("betas").is_none());
}

#[tokio::test]
async fn tagged_models_keep_effort_and_fast_separate_from_the_copilot_default() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let app = crate::make_router(state(upstream.config()));
    let copilot = app
        .clone()
        .oneshot(request(
            "/v1/messages",
            &json!({
                "model":"claude-opus-5-5", "max_tokens":32,
                "thinking":{"type":"adaptive"}, "output_config":{"effort":"low"},
                "messages":[{"role":"user","content":"hi"}]
            }),
        ))
        .await
        .unwrap();
    let router = app
        .oneshot(request(
            "/v1/messages",
            &json!({
                "model":"claude-opus-5-5[llm-router]", "max_tokens":32,
                "thinking":{"type":"adaptive"}, "output_config":{"effort":"max"},
                "speed":"fast", "betas":["fast-mode-2026-02-01"],
                "messages":[{"role":"user","content":"hi"}]
            }),
        ))
        .await
        .unwrap();

    assert_eq!(copilot.status(), StatusCode::OK);
    assert_eq!(router.status(), StatusCode::OK);
    let (path, headers, body) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/copilot/v1/messages");
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["output_config"]["effort"], "low");
    assert!(body.get("speed").is_none());
    assert!(
        headers["anthropic-beta"]
            .to_str()
            .unwrap()
            .split(',')
            .any(|beta| beta == "oauth-2025-04-20")
    );
    assert!(!headers.contains_key("x-request-resource-group"));
    let (path, headers, body) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/api/v1/messages");
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["thinking"], json!({"type":"adaptive"}));
    assert_eq!(body["output_config"]["effort"], "max");
    assert_eq!(body["speed"], "fast");
    assert_eq!(
        headers["anthropic-beta"]
            .to_str()
            .unwrap()
            .split(',')
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["client-beta", "fast-mode-2026-02-01"])
    );
    assert!(body.get("betas").is_none());
    assert_eq!(headers["x-request-resource-group"], "5");
    assert_eq!(headers["x-api-key"], "configured-message-key");
    assert!(!headers.contains_key("authorization"));
}

#[tokio::test]
async fn invalid_provider_suffixes_never_fall_back_to_copilot() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let app = crate::make_router(state(upstream.config()));

    for model in [
        "claude-opus-5-5[removed-router]",
        "claude-opus-5-5[]",
        "[copilot]",
        "[1m][copilot]",
    ] {
        for path in ["/v1/messages", "/v1/messages/count_tokens"] {
            let response = app
                .clone()
                .oneshot(request(path, &json!({"model": model, "messages": []})))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{path}: {model}"
            );
        }
    }
    assert!(upstream.received.try_recv().is_err());
}

#[tokio::test]
async fn hidden_models_keep_their_ids_and_messages_headers_for_generation_and_counting() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = upstream.config();
    config
        .providers
        .get_mut("llm-router")
        .unwrap()
        .anthropic
        .as_mut()
        .unwrap()
        .enabled_models = Some(BTreeSet::new());
    let app = crate::make_router(state(config));
    let body = json!({"model":"claude-sonnet-4-6-stable-max[llm-router]","messages":[{"role":"user","content":"hi"}]});
    let first = app
        .clone()
        .oneshot(request("/v1/messages", &body))
        .await
        .unwrap();
    let count = app
        .oneshot(request("/v1/messages/count_tokens", &body))
        .await
        .unwrap();

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(count.status(), StatusCode::OK);
    assert_eq!(json_body(count).await, json!({"input_tokens":23}));
    let (_, first_headers, first_payload) = upstream.received.recv().await.unwrap();
    let (path, count_headers, payload) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/api/v1/messages/count_tokens");
    let expected =
        json!({"model":"claude-sonnet-4-6-stable-max","messages":[{"role":"user","content":"hi"}]});
    assert_eq!(first_payload, expected);
    assert_eq!(payload, expected);
    assert_eq!(count_headers["x-request-resource-group"], "5");
    assert_eq!(count_headers["x-api-key"], "configured-message-key");
    let first_id = first_headers["x-request-task-uid"]
        .to_str()
        .unwrap()
        .strip_prefix("message-")
        .unwrap();
    let count_id = count_headers["x-request-task-uid"]
        .to_str()
        .unwrap()
        .strip_prefix("message-")
        .unwrap();
    assert!(uuid::Uuid::parse_str(first_id).is_ok());
    assert!(uuid::Uuid::parse_str(count_id).is_ok());
    assert_ne!(first_id, count_id);
}

#[tokio::test]
async fn header_only_upstreams_do_not_fall_back_to_claude_or_responses_keys() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = upstream.config();
    let connection = config
        .providers
        .get_mut("llm-router")
        .unwrap()
        .anthropic
        .as_mut()
        .unwrap();
    connection.api_key = None;
    connection.headers.remove("X-Api-Key");
    config.anthropic.routes.default = Some("llm-router".into());
    let app = crate::make_router(state(config));
    let response = app
        .oneshot(request(
            "/v1/messages",
            &json!({"model":"claude-opus-5-5","messages":[]}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let (_, headers, _) = upstream.received.recv().await.unwrap();
    assert!(!headers.contains_key("authorization"));
    assert!(!headers.contains_key("x-api-key"));
    assert_eq!(headers["x-request-resource-group"], "5");
}

#[tokio::test]
async fn qualified_models_resolve_before_auto_thinking_and_long_context() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let app = crate::make_router(state(upstream.config()));
    let body = json!({
        "model":"claude-opus-5-5[llm-router][1m]", "thinking":{"type":"auto"}, "messages":[]
    });
    let response = app
        .clone()
        .oneshot(request("/v1/messages", &body))
        .await
        .unwrap();
    let count = app
        .oneshot(request("/v1/messages/count_tokens", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(count.status(), StatusCode::OK);
    let (_, headers, payload) = upstream.received.recv().await.unwrap();
    assert_eq!(payload["model"], "claude-opus-5-5");
    assert_eq!(payload["thinking"], json!({"type":"adaptive"}));
    assert_eq!(
        headers["anthropic-beta"]
            .to_str()
            .unwrap()
            .split(',')
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["context-1m-2025-08-07", "client-beta"])
    );
    let (path, headers, payload) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/api/v1/messages/count_tokens");
    assert_eq!(payload["model"], "claude-opus-5-5");
    assert_eq!(
        headers["anthropic-beta"]
            .to_str()
            .unwrap()
            .split(',')
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["context-1m-2025-08-07", "client-beta"])
    );
}

#[tokio::test]
async fn disabled_custom_providers_reject_requests_and_disappear_after_reload() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let config = upstream.config();
    let state = state(config.clone());
    let app = crate::make_router(state.clone());
    let body = json!({"model":"claude-opus-5-5[llm-router]","messages":[]});
    let before = app
        .clone()
        .oneshot(request("/v1/messages", &body))
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK);
    upstream.received.recv().await.unwrap();

    let mut disabled = config;
    disabled.providers.get_mut("llm-router").unwrap().enabled = false;
    state.config.store(Arc::new(disabled));
    let after = app
        .clone()
        .oneshot(request("/v1/messages", &body))
        .await
        .unwrap();
    let count = app
        .clone()
        .oneshot(request("/v1/messages/count_tokens", &body))
        .await
        .unwrap();
    let catalog = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(after.status(), StatusCode::BAD_REQUEST);
    assert_eq!(count.status(), StatusCode::BAD_REQUEST);
    let models = json_body(catalog).await;
    assert_eq!(models["data"].as_array().unwrap().len(), 1);
    assert_eq!(models["data"][0]["id"], "claude-opus-5-5");
    let (path, _, _) = upstream.received.recv().await.unwrap();
    assert_eq!(path, "/copilot/models");
    assert!(upstream.received.try_recv().is_err());
}

#[tokio::test]
async fn native_streams_keep_signature_deltas_unknown_fields_and_the_terminal_event() {
    const SSE: &str = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":13,\"output_tokens\":0}}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"opaque-signature\"},\"future\":true}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    );
    let upstream = Upstream::start(StatusCode::OK, SSE).await;
    let state = state(upstream.config());
    let app = crate::make_router(state.clone());
    let response = app
        .oneshot(request(
            "/v1/messages",
            &json!({"model":"claude-opus-5-5[llm-router]","stream":true,"messages":[]}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        SSE
    );
    let usage = state.usage.snapshot();
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.success_requests
        ),
        (13, 7, 1)
    );
}

#[tokio::test]
async fn upstream_rejections_keep_the_status_body_and_retry_delay() {
    const ERROR: &str = r#"{"type":"error","error":{"type":"rate_limit_error","message":"resource group exhausted"}}"#;
    let upstream = Upstream::start(StatusCode::TOO_MANY_REQUESTS, ERROR).await;
    let app = crate::make_router(state(upstream.config()));
    let response = app
        .oneshot(request(
            "/v1/messages",
            &json!({"model":"llm-router/claude-opus-5-5","messages":[]}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "17");
    assert_eq!(
        json_body(response).await,
        serde_json::from_str::<Value>(ERROR).unwrap()
    );
}

#[tokio::test]
async fn a_custom_model_route_lists_bare_ids_and_keeps_other_models_qualified() {
    let upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = upstream.config();
    config
        .anthropic
        .routes
        .models
        .insert("claude-opus-5-5".parse().unwrap(), "llm-router".into());
    let app = crate::make_router(state(config));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let ids: Vec<_> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "claude-opus-5-5",
            "claude-opus-5.5",
            "claude-sonnet-4-6-stable-max[llm-router]"
        ]
    );
    assert_eq!(body["data"][0]["owned_by"], "llm-router");
}

#[tokio::test]
async fn catalog_qualifiers_do_not_overwrite_upstream_aliases() {
    let mut upstream = Upstream::start(StatusCode::OK, MESSAGE).await;
    let mut config = upstream.config();
    config.anthropic.routes.default = Some("llm-router".into());
    config.providers.get_mut("copilot").unwrap().enabled = false;
    let connection = config
        .providers
        .get_mut("llm-router")
        .unwrap()
        .anthropic
        .as_mut()
        .unwrap();
    connection.models_url = None;
    connection.enabled_models = Some(BTreeSet::from([
        "claude-private/preview".into(),
        "claude-private[preview]".into(),
    ]));
    let app = crate::make_router(state(config));

    let catalog = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(catalog.status(), StatusCode::OK);
    let models = json_body(catalog).await;
    assert_eq!(models["data"][0]["id"], "llm-router/claude-private/preview");
    assert_eq!(
        models["data"][1]["id"],
        "claude-private[preview][llm-router]"
    );
    for (id, upstream_id) in [
        (
            "llm-router/claude-private/preview",
            "claude-private/preview",
        ),
        (
            "claude-private[preview][llm-router]",
            "claude-private[preview]",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request("/v1/messages", &json!({"model":id,"messages":[]})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let (path, _, body) = upstream.received.recv().await.unwrap();
        assert_eq!(path, "/api/v1/messages");
        assert_eq!(body["model"], upstream_id);
    }
}

#[tokio::test]
async fn custom_discovery_failure_does_not_return_a_partial_catalog() {
    let upstream = Upstream::start(StatusCode::SERVICE_UNAVAILABLE, MESSAGE).await;
    let mut config = serde_json::to_value(upstream.config()).unwrap();
    config["providers"]["llm-router"]["anthropic"]["enabled_models"] = json!(["claude-opus-5-5"]);
    let app = crate::make_router(state(serde_json::from_value(config).unwrap()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["retry-after"], "17");
}
