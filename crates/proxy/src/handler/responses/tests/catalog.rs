use super::*;

fn catalog_request() -> Request<Body> {
    Request::builder()
        .uri("/codex/models?client_version=1.2.3")
        .header("authorization", "Bearer client-chatgpt-token")
        .header("chatgpt-account-id", "client-account")
        .header("cookie", "private-cookie")
        .header("x-oai-attestation", "private-attestation")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn custom_catalogs_list_and_route_each_matching_model() {
    let native = serve(Router::new().route(
        "/models",
        get(|| async {
            Json(json!({"models": [
                {"slug": "gpt-small", "display_name": "Small", "context_window": 64_000,
                 "base_instructions": "small instructions", "upgrade": {"model": "gpt-large"}},
                {"slug": "gpt-large", "display_name": "Large", "context_window": 400_000,
                 "base_instructions": "legacy", "model_messages": {"instructions_template": "large instructions"},
                 "future_capability": {"enabled": true}},
                {"slug": "native-only", "display_name": "Native only", "base_instructions": "native instructions"}
            ]}))
        }),
    ))
    .await;
    let (router, mut catalog_requests) = capture(
        StatusCode::OK,
        r#"{"data":[{"id":"gpt-small"},{"id":"gpt-large"},{"id":"image-only"},{"id":"gpt-small"}]}"#,
    );
    let catalog = serve(router).await;
    let (router, mut response_requests) = capture(StatusCode::OK, COMPLETED);
    let upstream = serve(router).await;
    let config: Config = serde_json::from_value(json!({"responses": {
        "chatgpt_base_url": native.url,
        "upstreams": {"company": {
            "base_url": format!("{}/deployment", upstream.url),
            "models_url": format!("{}/directory?tenant=5", catalog.url),
            "display_name": "LLM Router",
            "api_key": "company-key",
            "headers": {"x-tenant": "engineering"},
        }},
    }}))
    .unwrap();
    let app = crate::make_router(state(config));

    let response = app.clone().oneshot(catalog_request()).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let models = body["models"].as_array().unwrap();
    let slugs: Vec<_> = models.iter().map(|m| m["slug"].as_str().unwrap()).collect();
    assert_eq!(
        slugs,
        [
            "company/gpt-large",
            "company/gpt-small",
            "gpt-large",
            "gpt-small",
            "native-only"
        ]
    );
    assert_eq!(models[0]["display_name"], "Large (LLM Router)");
    assert_eq!(models[0]["context_window"], 400_000);
    assert_eq!(
        models[0]["model_messages"]["instructions_template"],
        "large instructions"
    );
    assert_eq!(models[0]["future_capability"], json!({"enabled": true}));
    assert!(models[0].get("base_instructions").is_none());
    assert_eq!(models[1]["display_name"], "Small (LLM Router)");
    assert_eq!(models[1]["context_window"], 64_000);
    assert_eq!(models[1]["base_instructions"], "small instructions");
    assert_eq!(models[1]["upgrade"], Value::Null);
    assert_eq!(models[3]["upgrade"]["model"], "gpt-large");
    let (path, headers, _) = catalog_requests.recv().await.unwrap();
    assert_eq!(path, "/directory?tenant=5");
    assert_eq!(headers["authorization"], "Bearer company-key");
    assert_eq!(headers["x-tenant"], "engineering");
    assert!(!headers.contains_key("chatgpt-account-id"));
    assert!(!headers.contains_key("cookie"));
    assert!(!headers.contains_key("x-oai-attestation"));

    let small = app
        .clone()
        .oneshot(request(&json!({
            "model": "company/gpt-small", "input": "small request", "stream": true,
        })))
        .await
        .unwrap();
    let large = app
        .oneshot(request(&json!({
            "model": "company/gpt-large", "input": "large request", "stream": true,
        })))
        .await
        .unwrap();

    assert_eq!(small.status(), StatusCode::OK);
    assert_eq!(large.status(), StatusCode::OK);
    let (small_path, _, small_body) = response_requests.recv().await.unwrap();
    let (large_path, _, large_body) = response_requests.recv().await.unwrap();
    assert_eq!(small_path, "/deployment/responses");
    assert_eq!(large_path, "/deployment/responses");
    assert_eq!(small_body["model"], "gpt-small");
    assert_eq!(large_body["model"], "gpt-large");
}

#[tokio::test]
async fn custom_default_discovery_keeps_explicit_aliases_and_uses_the_upstream_name() {
    let native = serve(Router::new().route(
        "/models",
        get(|| async {
            Json(json!({"models": [
                {"slug": "gpt-small", "display_name": "Small", "base_instructions": "small instructions"},
                {"slug": "gpt-large", "display_name": "Large", "base_instructions": "large instructions"},
                {"slug": "native-only", "base_instructions": "native instructions"}
            ]}))
        }),
    )).await;
    let upstream = serve(Router::new().route(
        "/models",
        get(|| async { Json(json!({"data": [{"id": "gpt-small"}, {"id": "gpt-large"}]})) }),
    ))
    .await;
    let config: Config = serde_json::from_value(json!({"responses": {
        "default": "company",
        "chatgpt_base_url": native.url,
        "upstreams": {"company": {"base_url": upstream.url, "models_url": format!("{}/models", upstream.url)}},
        "models": {"company/gpt-small": {
            "upstream": "company", "model": "deployment",
            "catalog": {"display_name": "Pinned", "base_instructions": "deployment instructions"},
        }},
    }})).unwrap();

    let response = crate::make_router(state(config))
        .oneshot(catalog_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let models = body["models"].as_array().unwrap();
    let slugs: Vec<_> = models.iter().map(|m| m["slug"].as_str().unwrap()).collect();
    assert_eq!(
        slugs,
        [
            "company/gpt-large",
            "company/gpt-small",
            "gpt-large",
            "gpt-small"
        ]
    );
    assert_eq!(models[0]["display_name"], "Large (company)");
    assert_eq!(models[1]["display_name"], "Pinned");
    assert_eq!(models[1]["base_instructions"], "deployment instructions");
    assert_eq!(models[2]["display_name"], "Large");
    assert_eq!(models[3]["base_instructions"], "small instructions");
}

#[tokio::test]
async fn custom_catalog_failures_are_not_hidden_as_an_empty_model_list() {
    const ERROR: &str = r#"{"error":{"message":"catalog quota exhausted"}}"#;
    let native =
        serve(Router::new().route("/models", get(|| async { Json(json!({"models": []})) }))).await;
    let (router, _requests) = capture(StatusCode::TOO_MANY_REQUESTS, ERROR);
    let upstream = serve(router).await;
    let config: Config = serde_json::from_value(json!({"responses": {
        "chatgpt_base_url": native.url,
        "upstreams": {"company": {"base_url": upstream.url, "models_url": format!("{}/models", upstream.url)}},
    }})).unwrap();

    let response = crate::make_router(state(config))
        .oneshot(catalog_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers()["retry-after"],
        "Fri, 02 Oct 2026 12:00:00 GMT"
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        ERROR
    );
}
