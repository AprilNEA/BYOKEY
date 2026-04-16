//! Integration tests for the `ampcode` crate.
//!
//! Uses a local axum mock server to simulate `ampcode.com` responses
//! and temporary files for local I/O tests.

#![allow(clippy::items_after_statements)] // test functions define handlers inline

use ampcode::{AmpcodeClient, AmpcodeError, Plan};
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::net::SocketAddr;

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Spawn a mock server and return `(base_url, join_handle)`.
async fn spawn_mock(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base_url, handle)
}

/// Build a client pointed at the mock server.
fn mock_client(base_url: &str) -> AmpcodeClient {
    AmpcodeClient::new("test-token".to_string()).with_base_url(base_url)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Balance API
// ═══════════════════════════════════════════════════════════════════════════════

/// Wrap a result in the JSON-RPC response envelope.
fn rpc_ok(result: &Value) -> Value {
    json!({ "ok": true, "result": result })
}

fn balance_router() -> Router {
    async fn rpc_handler(Json(body): Json<Value>) -> impl IntoResponse {
        let method = body["method"].as_str().unwrap_or("");
        match method {
            "userDisplayBalanceInfo" => {
                let display = "Signed in as test@example.com Amp Free: $6.50/$10.00 remaining (replenishes +$0.42/hour)";
                Json(rpc_ok(&json!({ "displayText": display }))).into_response()
            }
            "unknownFormat" => {
                Json(rpc_ok(&json!({ "displayText": "Something entirely new" }))).into_response()
            }
            _ => StatusCode::BAD_REQUEST.into_response(),
        }
    }

    Router::new().route("/api/internal", post(rpc_handler))
}

#[tokio::test]
async fn balance_free_tier() {
    let (base_url, _handle) = spawn_mock(balance_router()).await;
    let client = mock_client(&base_url);

    let info = client.balance().await.unwrap();
    assert_eq!(info.plan, Plan::Free);
    assert_eq!(info.user.as_deref(), Some("test@example.com"));
    assert!((info.remaining_dollars.unwrap() - 6.5).abs() < f64::EPSILON);
    assert!((info.total_dollars.unwrap() - 10.0).abs() < f64::EPSILON);
    assert!((info.replenish_rate_dollars.unwrap() - 0.42).abs() < f64::EPSILON);
    assert!(info.credits_dollars.is_none());
}

#[tokio::test]
async fn balance_display_text_raw() {
    let (base_url, _handle) = spawn_mock(balance_router()).await;
    let client = mock_client(&base_url);

    let text = client.balance_display_text().await.unwrap();
    assert!(text.starts_with("Signed in as test@example.com"));
    assert!(text.contains("$6.50/$10.00"));
}

#[tokio::test]
async fn balance_individual_credits() {
    async fn handler(Json(_body): Json<Value>) -> Json<Value> {
        Json(rpc_ok(&json!({
            "displayText": "Signed in as buyer@corp.io Individual credits: $123.45 remaining"
        })))
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let info = client.balance().await.unwrap();
    assert_eq!(info.plan, Plan::IndividualCredits);
    assert_eq!(info.user.as_deref(), Some("buyer@corp.io"));
    assert!((info.credits_dollars.unwrap() - 123.45).abs() < f64::EPSILON);
}

#[tokio::test]
async fn balance_free_tier_with_bonus() {
    async fn handler(Json(_body): Json<Value>) -> Json<Value> {
        Json(rpc_ok(&json!({
            "displayText": "Signed in as promo@test.io Amp Free: $8.00/$10.00 remaining (replenishes +$0.50/hour +30% bonus for 5 more days)"
        })))
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let info = client.balance().await.unwrap();
    assert_eq!(info.plan, Plan::Free);
    assert_eq!(info.bonus_percent, Some(30));
    assert_eq!(info.bonus_days_remaining, Some(5));
    assert!((info.replenish_rate_dollars.unwrap() - 0.50).abs() < f64::EPSILON);
}

#[tokio::test]
async fn balance_bare_credits() {
    async fn handler(Json(_body): Json<Value>) -> Json<Value> {
        Json(rpc_ok(&json!({ "displayText": "$42.00 remaining" })))
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let info = client.balance().await.unwrap();
    assert_eq!(info.plan, Plan::IndividualCredits);
    assert!(info.user.is_none());
    assert!((info.credits_dollars.unwrap() - 42.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn balance_unknown_format_falls_back_to_display_text() {
    async fn handler(Json(_body): Json<Value>) -> Json<Value> {
        Json(rpc_ok(
            &json!({ "displayText": "Enterprise workspace: unlimited" }),
        ))
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    // balance() should fail with BalanceParse
    let err = client.balance().await.unwrap_err();
    assert!(matches!(err, AmpcodeError::BalanceParse(ref s) if s.contains("Enterprise")));

    // balance_display_text() should succeed with the raw text
    let text = client.balance_display_text().await.unwrap();
    assert_eq!(text, "Enterprise workspace: unlimited");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Error handling
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn api_error_401_unauthorized() {
    async fn handler() -> impl IntoResponse {
        (StatusCode::UNAUTHORIZED, "invalid token")
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let err = client
        .rpc::<Value>("userDisplayBalanceInfo", None)
        .await
        .unwrap_err();

    match err {
        AmpcodeError::Api { status, body } => {
            assert_eq!(status, 401);
            assert_eq!(body, "invalid token");
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

#[tokio::test]
async fn api_error_500_internal() {
    async fn handler() -> impl IntoResponse {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "something broke"})),
        )
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let err = client.balance().await.unwrap_err();
    match err {
        AmpcodeError::Api { status, .. } => assert_eq!(status, 500),
        other => panic!("expected Api error, got: {other}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Thread API
// ═══════════════════════════════════════════════════════════════════════════════

fn thread_fixture() -> Value {
    json!({
        "v": 3,
        "id": "T-abcdef01-2345-6789-abcd-ef0123456789",
        "created": 1_711_728_000_000_u64,
        "title": "Fix authentication bug",
        "agentMode": "smart",
        "messages": [
            {
                "role": "user",
                "messageId": 0,
                "content": [{"type": "text", "text": "Fix the login bug"}]
            },
            {
                "role": "assistant",
                "messageId": 1,
                "content": [
                    {"type": "thinking", "thinking": "Let me look at the auth module..."},
                    {"type": "text", "text": "I found the issue in auth.rs"},
                    {"type": "tool_use", "id": "toolu_01", "name": "Read", "input": {"path": "src/auth.rs"}}
                ],
                "usage": {
                    "model": "claude-sonnet-4-20250514",
                    "inputTokens": 2500,
                    "outputTokens": 800,
                    "cacheCreationInputTokens": 100,
                    "cacheReadInputTokens": 50,
                    "totalInputTokens": 2650
                },
                "state": {"type": "complete", "stopReason": "tool_use"}
            },
            {
                "role": "user",
                "messageId": 2,
                "content": [{
                    "type": "tool_result",
                    "toolUseID": "toolu_01",
                    "run": {"status": "done", "result": {"output": "pub fn login() { ... }"}}
                }]
            }
        ],
        "relationships": [
            {"threadID": "T-00000000-0000-0000-0000-000000000001", "type": "fork", "role": "child"}
        ],
        "nextMessageId": 3
    })
}

fn thread_router() -> Router {
    /// Handles both `/api/threads/{id}` and `/api/threads/{id}.md` via a
    /// wildcard, since axum doesn't support mixed param+literal segments.
    async fn get_thread_any(Path(raw): Path<String>) -> impl IntoResponse {
        if let Some(id) = raw.strip_suffix(".md") {
            if id == "T-abcdef01-2345-6789-abcd-ef0123456789" {
                return "# Fix authentication bug\n\n**User:** Fix the login bug\n\n**Assistant:** I found the issue in auth.rs".into_response();
            }
            return (StatusCode::NOT_FOUND, "thread not found").into_response();
        }
        if raw == "T-abcdef01-2345-6789-abcd-ef0123456789" {
            return Json(thread_fixture()).into_response();
        }
        (StatusCode::NOT_FOUND, "thread not found").into_response()
    }

    async fn find_threads(
        axum::extract::Query(params): axum::extract::Query<
            std::collections::HashMap<String, String>,
        >,
    ) -> Json<Vec<Value>> {
        let query = params.get("q").map_or("", String::as_str);
        if query == "auth" {
            Json(vec![thread_fixture()])
        } else {
            Json(vec![])
        }
    }

    async fn list_threads(
        axum::extract::Query(params): axum::extract::Query<
            std::collections::HashMap<String, String>,
        >,
    ) -> Json<Vec<Value>> {
        if params.contains_key("createdByUserID") {
            Json(vec![thread_fixture()])
        } else {
            Json(vec![])
        }
    }

    Router::new()
        .route("/api/threads/find", get(find_threads))
        .route("/api/threads", get(list_threads))
        .route("/api/threads/{*id}", get(get_thread_any))
}

#[tokio::test]
async fn get_thread_by_id() {
    let (base_url, _handle) = spawn_mock(thread_router()).await;
    let client = mock_client(&base_url);

    let thread = client
        .get_thread("T-abcdef01-2345-6789-abcd-ef0123456789")
        .await
        .unwrap();

    assert_eq!(thread.id, "T-abcdef01-2345-6789-abcd-ef0123456789");
    assert_eq!(thread.title.as_deref(), Some("Fix authentication bug"));
    assert_eq!(thread.agent_mode.as_deref(), Some("smart"));
    assert_eq!(thread.v, 3);
    assert_eq!(thread.messages.len(), 3);

    // Verify assistant message content blocks
    let assistant = &thread.messages[1];
    assert_eq!(assistant.role, "assistant");
    assert_eq!(assistant.content.len(), 3);
    assert!(matches!(
        &assistant.content[0],
        ampcode::ContentBlock::Thinking { thinking } if thinking.contains("auth module")
    ));
    assert!(matches!(
        &assistant.content[1],
        ampcode::ContentBlock::Text { text } if text.contains("auth.rs")
    ));
    assert!(matches!(
        &assistant.content[2],
        ampcode::ContentBlock::ToolUse { name, .. } if name == "Read"
    ));

    // Verify usage
    let usage = assistant.usage.as_ref().unwrap();
    assert_eq!(usage.model, "claude-sonnet-4-20250514");
    assert_eq!(usage.input_tokens, Some(2500));
    assert_eq!(usage.output_tokens, Some(800));
    assert_eq!(usage.cache_creation_input_tokens, Some(100));
    assert_eq!(usage.total_input_tokens, Some(2650));

    // Verify tool result
    let tool_result = &thread.messages[2];
    assert!(matches!(
        &tool_result.content[0],
        ampcode::ContentBlock::ToolResult { tool_use_id, run } if tool_use_id == "toolu_01" && run.status == "done"
    ));

    // Verify relationship
    assert_eq!(thread.relationships.len(), 1);
    assert_eq!(thread.relationships[0].rel_type, "fork");
    assert_eq!(thread.relationships[0].role.as_deref(), Some("child"));
}

#[tokio::test]
async fn get_thread_not_found() {
    let (base_url, _handle) = spawn_mock(thread_router()).await;
    let client = mock_client(&base_url);

    let err = client.get_thread("T-nonexistent").await.unwrap_err();
    match err {
        AmpcodeError::Api { status, .. } => assert_eq!(status, 404),
        other => panic!("expected Api 404 error, got: {other}"),
    }
}

#[tokio::test]
async fn get_thread_markdown() {
    let (base_url, _handle) = spawn_mock(thread_router()).await;
    let client = mock_client(&base_url);

    let md = client
        .get_thread_markdown("T-abcdef01-2345-6789-abcd-ef0123456789")
        .await
        .unwrap();

    assert!(md.contains("# Fix authentication bug"));
    assert!(md.contains("I found the issue in auth.rs"));
}

#[tokio::test]
async fn find_threads_with_results() {
    let (base_url, _handle) = spawn_mock(thread_router()).await;
    let client = mock_client(&base_url);

    let threads = client.find_threads("auth", None, None).await.unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].title.as_deref(), Some("Fix authentication bug"));
}

#[tokio::test]
async fn find_threads_empty() {
    let (base_url, _handle) = spawn_mock(thread_router()).await;
    let client = mock_client(&base_url);

    let threads = client
        .find_threads("nonexistent", None, None)
        .await
        .unwrap();
    assert!(threads.is_empty());
}

#[tokio::test]
async fn list_threads_by_user() {
    let (base_url, _handle) = spawn_mock(thread_router()).await;
    let client = mock_client(&base_url);

    let threads = client.list_threads_by_user("user-123").await.unwrap();
    assert_eq!(threads.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Generic RPC
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn generic_rpc_call() {
    async fn handler(Json(body): Json<Value>) -> impl IntoResponse {
        let method = body["method"].as_str().unwrap_or("");
        if method == "customMethod" {
            Json(json!({"result": "ok", "data": 42})).into_response()
        } else {
            StatusCode::BAD_REQUEST.into_response()
        }
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let resp: Value = client.rpc("customMethod", None).await.unwrap();
    assert_eq!(resp["result"], "ok");
    assert_eq!(resp["data"], 42);
}

#[tokio::test]
async fn generic_rpc_with_params() {
    async fn handler(Json(body): Json<Value>) -> Json<Value> {
        let echo = body["params"].clone();
        Json(json!({"echo": echo}))
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let params = json!({"key": "value", "num": 123});
    let resp: Value = client
        .rpc("echoMethod", Some(params.clone()))
        .await
        .unwrap();
    assert_eq!(resp["echo"], params);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  GitHub auth status
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn github_auth_status() {
    async fn handler() -> Json<Value> {
        Json(json!({"authenticated": true, "username": "octocat"}))
    }

    let app = Router::new().route("/api/internal/github-auth-status", get(handler));
    let (base_url, _handle) = spawn_mock(app).await;
    let client = mock_client(&base_url);

    let status = client.github_auth_status().await.unwrap();
    assert_eq!(status["authenticated"], true);
    assert_eq!(status["username"], "octocat");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Local thread file I/O
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn local_read_thread_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("T-test-0001.json");
    std::fs::write(&path, serde_json::to_string(&thread_fixture()).unwrap()).unwrap();

    let thread = ampcode::local::read_thread(&path).unwrap();
    assert_eq!(thread.id, "T-abcdef01-2345-6789-abcd-ef0123456789");
    assert_eq!(thread.messages.len(), 3);
    assert_eq!(thread.title.as_deref(), Some("Fix authentication bug"));
}

#[test]
fn local_read_thread_file_not_found() {
    let err = ampcode::local::read_thread(std::path::Path::new("/tmp/nonexistent_thread.json"))
        .unwrap_err();
    assert!(matches!(err, AmpcodeError::Io(_)));
}

#[test]
fn local_read_thread_invalid_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("T-bad.json");
    std::fs::write(&path, "not valid json {{{").unwrap();

    let err = ampcode::local::read_thread(&path).unwrap_err();
    assert!(matches!(err, AmpcodeError::Json(_)));
}

#[test]
fn local_scan_summaries() {
    let dir = tempfile::tempdir().unwrap();

    // Write 3 thread files with different timestamps.
    for (i, created) in [(1, 1_000_000_u64), (2, 3_000_000), (3, 2_000_000)] {
        let path = dir.path().join(format!("T-{i:08x}.json"));
        let thread = json!({
            "v": 1,
            "id": format!("T-{i:08x}"),
            "created": created,
            "messages": [{
                "role": "assistant",
                "messageId": 0,
                "usage": {"model": "claude-sonnet-4-20250514", "inputTokens": i * 100, "outputTokens": i * 50}
            }],
            "nextMessageId": 1
        });
        std::fs::write(path, serde_json::to_string(&thread).unwrap()).unwrap();
    }

    // Write a non-thread file that should be ignored.
    std::fs::write(dir.path().join("settings.json"), "{}").unwrap();

    let summaries = ampcode::local::scan_summaries_sync(dir.path());
    assert_eq!(summaries.len(), 3);

    // Should be sorted by created descending.
    assert_eq!(summaries[0].created, 3_000_000);
    assert_eq!(summaries[1].created, 2_000_000);
    assert_eq!(summaries[2].created, 1_000_000);

    // Verify usage stubs are populated.
    let usage = summaries[0].messages[0].usage.as_ref().unwrap();
    assert_eq!(usage.model.as_deref(), Some("claude-sonnet-4-20250514"));
}

#[test]
fn local_scan_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let summaries = ampcode::local::scan_summaries_sync(dir.path());
    assert!(summaries.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Secrets file
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn secrets_load_valid_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    std::fs::write(
        &path,
        r#"{"apiKey@https://ampcode.com/":"sgamp_user_abc123"}"#,
    )
    .unwrap();

    let token = ampcode::secrets::load_token_from(&path).await.unwrap();
    assert_eq!(token, "sgamp_user_abc123");
}

#[tokio::test]
async fn secrets_missing_provider() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    std::fs::write(&path, r#"{"apiKey@http://localhost:8317":"local_tok"}"#).unwrap();

    let err = ampcode::secrets::load_token_from(&path).await.unwrap_err();
    assert!(matches!(err, AmpcodeError::NoToken));
}

#[tokio::test]
async fn secrets_empty_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    std::fs::write(&path, r#"{"apiKey@https://ampcode.com/":""}"#).unwrap();

    let err = ampcode::secrets::load_token_from(&path).await.unwrap_err();
    assert!(matches!(err, AmpcodeError::NoToken));
}

#[tokio::test]
async fn secrets_file_not_found() {
    let err = ampcode::secrets::load_token_from(std::path::Path::new("/tmp/no_such_secrets.json"))
        .await
        .unwrap_err();
    assert!(matches!(err, AmpcodeError::Io(_)));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Token provider
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn token_provider_string() {
    use ampcode::TokenProvider;
    let s = "my-token".to_string();
    assert_eq!(s.token(), "my-token");
}

#[test]
fn token_provider_static_str() {
    use ampcode::TokenProvider;
    let s: &'static str = "static-token";
    assert_eq!(s.token(), "static-token");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  End-to-end: secrets → client → balance
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_secrets_to_balance() {
    // Step 1: Write a secrets file.
    let secrets_dir = tempfile::tempdir().unwrap();
    let secrets_path = secrets_dir.path().join("secrets.json");
    std::fs::write(
        &secrets_path,
        r#"{"apiKey@https://ampcode.com/":"sgamp_user_e2e"}"#,
    )
    .unwrap();

    // Step 2: Spin up a mock server that verifies the token.
    async fn handler(
        headers: axum::http::HeaderMap,
        Json(_body): Json<Value>,
    ) -> impl IntoResponse {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer sgamp_user_e2e" {
            return (StatusCode::UNAUTHORIZED, "bad token").into_response();
        }
        Json(rpc_ok(&json!({
            "displayText": "Signed in as e2e@test.com Amp Free: $9.00/$10.00 remaining (replenishes +$0.42/hour)"
        })))
        .into_response()
    }

    let app = Router::new().route("/api/internal", post(handler));
    let (base_url, _handle) = spawn_mock(app).await;

    // Step 3: Load token from secrets, create client, fetch balance.
    let token = ampcode::secrets::load_token_from(&secrets_path)
        .await
        .unwrap();
    let client = AmpcodeClient::new(token).with_base_url(&base_url);
    let info = client.balance().await.unwrap();

    assert_eq!(info.plan, Plan::Free);
    assert_eq!(info.user.as_deref(), Some("e2e@test.com"));
    assert!((info.remaining_dollars.unwrap() - 9.0).abs() < f64::EPSILON);
}
