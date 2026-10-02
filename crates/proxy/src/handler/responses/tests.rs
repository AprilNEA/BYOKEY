use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
    routing::get,
};
use byokey_auth::AuthManager;
use byokey_config::{
    Config, ProviderConfig,
    schema::responses::{ConfigValue, ResponseModel},
};
use byokey_store::InMemoryTokenStore;
use serde_json::json;
use tokio::sync::mpsc;
use tower::ServiceExt as _;

struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(router: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server { url, task }
}

type Captured = (String, HeaderMap, Value);

fn capture(status: StatusCode, body: &'static str) -> (Router, mpsc::UnboundedReceiver<Captured>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let router = Router::new().fallback(move |request: Request<Body>| {
        let tx = tx.clone();
        async move {
            let (parts, bytes) = request.into_parts();
            let bytes = to_bytes(bytes, usize::MAX).await.unwrap();
            let payload = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            tx.send((parts.uri.to_string(), parts.headers, payload))
                .unwrap();
            Response::builder()
                .status(status)
                .header(
                    "content-type",
                    if body.starts_with("data:") {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                )
                .header("retry-after", "Fri, 02 Oct 2026 12:00:00 GMT")
                .header("x-codex-turn-state", "opaque-state")
                .body(Body::from(body))
                .unwrap()
        }
    });
    (router, rx)
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
}

fn request(body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .header("authorization", "Bearer client-chatgpt-token")
        .header("chatgpt-account-id", "client-account")
        .header("cookie", "private-cookie")
        .header("x-oai-attestation", "private-attestation")
        .header("x-codex-turn-state", "previous-turn-state")
        .header("x-client-request-id", "client-request")
        .body(Body::from(body.to_string()))
        .unwrap()
}

const COMPLETED: &str = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":13,\"output_tokens\":7}}}\n\n";

#[tokio::test]
async fn chatgpt_keeps_auth_and_unknown_request_fields_and_counts_responses_usage() {
    let (router, mut received) = capture(StatusCode::OK, COMPLETED);
    let upstream = serve(router).await;
    let mut config = Config::default();
    config.responses.chatgpt_base_url = format!("{}/backend-api/codex/", upstream.url);
    let state = state(config);
    let body = json!({"model":"chatgpt/gpt-example", "stream":true,
        "input":[{"type":"function_call_output","call_id":"call-opaque","output":"42"}],
        "reasoning":{"effort":"high"}, "future_field":{"preserved":true}});
    let response = crate::make_router(state.clone())
        .oneshot(request(&body))
        .await
        .unwrap();
    assert_eq!(response.headers()["x-codex-turn-state"], "opaque-state");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, COMPLETED);
    let (path, headers, payload) = received.recv().await.unwrap();
    assert_eq!(path, "/backend-api/codex/responses");
    assert_eq!(headers["authorization"], "Bearer client-chatgpt-token");
    assert_eq!(headers["chatgpt-account-id"], "client-account");
    assert_eq!(headers["x-oai-attestation"], "private-attestation");
    assert!(!headers.contains_key("cookie"));
    assert_eq!(
        payload,
        json!({"model":"gpt-example", "stream":true,
        "input":[{"type":"function_call_output","call_id":"call-opaque","output":"42"}],
        "reasoning":{"effort":"high"}, "future_field":{"preserved":true}})
    );
    let usage = state.usage.snapshot();
    assert_eq!(
        (
            usage.success_requests,
            usage.failure_requests,
            usage.input_tokens,
            usage.output_tokens
        ),
        (1, 0, 13, 7)
    );
}

#[tokio::test]
async fn custom_upstreams_replace_credentials_and_resolve_environment_headers() {
    let (router, mut received) = capture(StatusCode::OK, COMPLETED);
    let upstream = serve(router).await;
    let mut config = Config::default();
    config.responses.upstreams.insert(
        "company".into(),
        ResponsesUpstream {
            base_url: format!("{}/team/v1", upstream.url),
            api_key: Some(ConfigValue::Literal("company-key".into())),
            headers: [
                (
                    "X-Special".into(),
                    ConfigValue::Literal("special-value".into()),
                ),
                (
                    "X-From-Environment".into(),
                    ConfigValue::Environment { env: "PATH".into() },
                ),
            ]
            .into(),
        },
    );
    config.responses.models.insert(
        "fast-alias".into(),
        ResponseModel {
            upstream: "company".into(),
            model: "vendor/gpt-fast".into(),
            catalog_model: None,
            catalog: None,
        },
    );
    let response = crate::make_router(state(config))
        .oneshot(request(
            &json!({"model":"fast-alias","input":"hi","stream":true}),
        ))
        .await
        .unwrap();
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        COMPLETED
    );
    let (path, headers, payload) = received.recv().await.unwrap();
    assert_eq!(path, "/team/v1/responses");
    assert_eq!(payload["model"], "vendor/gpt-fast");
    assert_eq!(headers["authorization"], "Bearer company-key");
    assert_eq!(headers["x-special"], "special-value");
    assert_eq!(headers["x-codex-turn-state"], "previous-turn-state");
    assert_eq!(
        headers["x-from-environment"],
        std::env::var("PATH").unwrap()
    );
    assert!(!headers.contains_key("chatgpt-account-id"));
    assert!(!headers.contains_key("cookie"));
    assert!(!headers.contains_key("x-oai-attestation"));
}

#[tokio::test]
async fn upstream_auth_errors_are_returned_without_retry_or_rewriting() {
    const ERROR: &str = "{ \"error\": {\"message\":\"expired\",\"code\":\"token_expired\"} }";
    let (router, mut received) = capture(StatusCode::UNAUTHORIZED, ERROR);
    let upstream = serve(router).await;
    let mut config = Config::default();
    config.responses.chatgpt_base_url = upstream.url.clone();
    let state = state(config);
    let response = crate::make_router(state.clone())
        .oneshot(request(&json!({"model":"gpt-example","input":"hi"})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()["retry-after"],
        "Fri, 02 Oct 2026 12:00:00 GMT"
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        ERROR
    );
    received.recv().await.unwrap();
    assert!(received.try_recv().is_err());
    assert_eq!(state.usage.snapshot().failure_requests, 1);
}

#[tokio::test]
async fn copilot_uses_its_own_credential_and_marks_tool_results_as_agent_requests() {
    let (router, mut received) = capture(StatusCode::OK, COMPLETED);
    let router = router.route("/models", get(|| async { Json(json!({"data":[
        {"id":"gpt-example","model_picker_enabled":true,"supported_endpoints":["/responses"]},
        {"id":"messages-only","model_picker_enabled":true,"supported_endpoints":["/v1/messages"]}
    ]})) }));
    let upstream = serve(router).await;
    let key = uuid::Uuid::new_v4().to_string();
    let mut config = Config::default();
    config.providers.insert(
        ProviderId::Copilot,
        ProviderConfig {
            api_key: Some(key.clone()),
            base_url: Some(upstream.url.clone()),
            ..Default::default()
        },
    );
    let app = crate::make_router(state(config));
    let response = app.clone().oneshot(request(&json!({"model":"copilot/gpt-example", "stream":true,
        "input":[{"role":"user","content":"compute"},{"type":"function_call_output","call_id":"opaque","output":"42"}]}))).await.unwrap();
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        COMPLETED
    );
    let (path, headers, payload) = received.recv().await.unwrap();
    assert_eq!(path, "/responses");
    assert_eq!(headers["authorization"], format!("Bearer {key}"));
    assert_eq!(headers["x-initiator"], "agent");
    assert_eq!(headers["x-codex-turn-state"], "previous-turn-state");
    assert!(!headers.contains_key("chatgpt-account-id"));
    assert!(!headers.contains_key("x-oai-attestation"));
    assert_eq!(payload["input"][1]["call_id"], "opaque");
    let rejected = app
        .oneshot(request(
            &json!({"model":"copilot/messages-only","input":"hi"}),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn aliases_keep_catalog_capabilities_but_disable_upstream_migrations() {
    let (router, mut received) = capture(
        StatusCode::OK,
        r#"{"models":[{"slug":"gpt-original","display_name":"Original","base_instructions":"original instructions","context_window":400000,"upgrade":{"model":"outside-gateway"},"future_field":17}]}"#,
    );
    let upstream = serve(router).await;
    let mut config = Config::default();
    config.responses.chatgpt_base_url = upstream.url.clone();
    config.responses.models.insert(
        "fast".into(),
        ResponseModel {
            upstream: "chatgpt".into(),
            model: "gpt-original".into(),
            catalog_model: None,
            catalog: None,
        },
    );
    let response = crate::make_router(state(config))
        .oneshot(
            Request::builder()
                .uri("/codex/models?client_version=1.2.3")
                .header("authorization", "Bearer catalog-token")
                .header("if-none-match", "stale")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        result["models"][0],
        json!({"slug":"fast","display_name":"fast","base_instructions":"original instructions","context_window":400_000,"upgrade":null,"future_field":17})
    );
    let (path, headers, _) = received.recv().await.unwrap();
    assert_eq!(path, "/models?client_version=1.2.3");
    assert_eq!(headers["authorization"], "Bearer catalog-token");
    assert!(!headers.contains_key("if-none-match"));
}

#[tokio::test]
async fn malformed_and_compressed_requests_fail_in_the_openai_envelope() {
    let app = crate::make_router(state(Config::default()));
    let bad = app
        .clone()
        .oneshot(request(&json!({"model":42})))
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let error: Value =
        serde_json::from_slice(&to_bytes(bad.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(error["error"]["code"], "invalid_request");
    let compressed = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/responses")
                .header("content-type", "application/json")
                .header("content-encoding", "zstd")
                .body(Body::from("compressed bytes"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(compressed.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(compressed.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("enable_request_compression"));
}

#[tokio::test]
async fn redirects_do_not_disclose_credentials_to_a_second_server() {
    let (router, mut received) = capture(StatusCode::OK, COMPLETED);
    let other = serve(router).await;
    let destination = other.url.clone();
    let redirect = serve(Router::new().fallback(move || {
        let destination = destination.clone();
        async move { (StatusCode::TEMPORARY_REDIRECT, [("location", destination)]) }
    }))
    .await;
    let mut config = Config::default();
    config.responses.chatgpt_base_url = redirect.url.clone();
    let response = crate::make_router(state(config))
        .oneshot(request(&json!({"model":"gpt-example","input":"hi"})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn explicit_custom_catalogs_work_without_chatgpt_credentials() {
    let config = Config::from_yaml(
        r"
responses:
  default: company
  upstreams:
    company:
      base_url: https://example.com/v1
  models:
    custom:
      upstream: company
      model: deployment
      catalog:
        display_name: Company model
        base_instructions: Deployment instructions
        context_window: 64000
",
    )
    .unwrap();
    let response = crate::make_router(state(config))
        .oneshot(
            Request::builder()
                .uri("/codex/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        json!({"models":[{"slug":"custom","display_name":"Company model",
        "base_instructions":"Deployment instructions","context_window":64000,"upgrade":null}]})
    );
}

#[tokio::test]
async fn copilot_catalogs_cap_context_without_inflating_smaller_models() {
    let upstream = serve(Router::new()
        .route("/chatgpt/models", get(|| async { Json(json!({"models":[
            {"slug":"gpt-small","context_window":64_000},
            {"slug":"gpt-large","context_window":400_000}
        ]})) }))
        .route("/models", get(|| async { Json(json!({"data":[
            {"id":"gpt-small","model_picker_enabled":true,"supported_endpoints":["/responses"],
             "capabilities":{"limits":{"max_context_window_tokens":128_000}}},
            {"id":"gpt-large","model_picker_enabled":true,"supported_endpoints":["/responses"],
             "capabilities":{"limits":{"max_context_window_tokens":128_000}}}
        ]})) }))).await;
    let mut config = Config::default();
    config.responses.default = "copilot".into();
    config.responses.chatgpt_base_url = format!("{}/chatgpt", upstream.url);
    config.providers.insert(
        ProviderId::Copilot,
        ProviderConfig {
            api_key: Some(uuid::Uuid::new_v4().to_string()),
            base_url: Some(upstream.url.clone()),
            ..Default::default()
        },
    );
    let response = crate::make_router(state(config))
        .oneshot(
            Request::builder()
                .uri("/codex/models")
                .header("authorization", "Bearer catalog-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let limits = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["slug"].as_str().unwrap(),
                m["context_window"].as_u64().unwrap(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        limits,
        [
            ("copilot/gpt-large", 128_000),
            ("copilot/gpt-small", 64_000),
            ("gpt-large", 128_000),
            ("gpt-small", 64_000)
        ]
        .into()
    );
}
