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

async fn copilot_catalog_server() -> Server {
    serve(Router::new()
        .route("/chatgpt/models", get(|| async { Json(json!({"models": [
            {"slug": "gpt-6-astra", "display_name": "GPT-6-Astra", "visibility": "list", "context_window": 400_000},
            {"slug": "gpt-6-sol", "display_name": "GPT-6-Sol", "visibility": "list", "context_window": 64_000}
        ]})) }))
        .route("/models", get(|| async { Json(json!({"data": [
            {"id": "gpt-6-astra", "name": "Different provider spelling", "model_picker_enabled": true,
             "supported_endpoints": ["/responses"], "capabilities": {"limits": {"max_context_window_tokens": 128_000}}},
            {"id": "gpt-6-sol", "model_picker_enabled": true, "supported_endpoints": ["/messages"]}
        ]})) })))
        .await
}

#[tokio::test]
async fn provider_names_are_consistent_and_legacy_aliases_remain_routable_but_hidden() {
    let catalog = copilot_catalog_server().await;
    let (router, mut received) = capture(StatusCode::OK, COMPLETED);
    let upstream = serve(router).await;
    let config: Config = serde_json::from_value(json!({
        "providers": {"copilot": {"api_key": uuid::Uuid::new_v4().to_string(), "base_url": catalog.url}},
        "responses": {
            "chatgpt_base_url": format!("{}/chatgpt", catalog.url),
            "catalog": {"model_names": {"gpt-6-astra": "GPT-6 Astra", "gpt-6-sol": "GPT-6 Sol"}},
            "upstreams": {"llm-router": {
                "base_url": upstream.url, "models_url": format!("{}/models", catalog.url), "display_name": "LLM Router",
            }},
            "models": {
                "copilot/gpt-6-astra": {"upstream": "copilot", "model": "gpt-6-astra"},
                "LLM Router": {"upstream": "llm-router", "model": "gpt-6-astra"},
                "deployment": {"upstream": "llm-router", "model": "private-deployment", "catalog": {
                    "display_name": "Private deployment", "visibility": "list", "base_instructions": "deployment instructions",
                }},
            },
        },
    })).unwrap();
    let app = crate::make_router(state(config));

    let response = app.clone().oneshot(catalog_request()).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let models = body["models"].as_array().unwrap();
    let visible: Vec<_> = models
        .iter()
        .filter(|m| m["visibility"] == "list")
        .map(|m| {
            (
                m["slug"].as_str().unwrap(),
                m["display_name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        visible,
        [
            ("copilot/gpt-6-astra", "GPT-6 Astra (Copilot)"),
            ("deployment", "Private deployment (LLM Router)"),
            ("gpt-6-astra", "GPT-6 Astra (ChatGPT)"),
            ("gpt-6-sol", "GPT-6 Sol (ChatGPT)"),
            ("llm-router/gpt-6-astra", "GPT-6 Astra (LLM Router)"),
            ("llm-router/gpt-6-sol", "GPT-6 Sol (LLM Router)"),
        ]
    );
    assert_eq!(models[0]["slug"], "LLM Router");
    assert_eq!(models[0]["visibility"], "hide");
    assert_eq!(models[0]["display_name"], "GPT-6 Astra (LLM Router)");
    assert_eq!(models[1]["context_window"], 128_000);
    assert_eq!(models[3]["context_window"], 400_000);

    let response = app
        .oneshot(request(
            &json!({"model": "LLM Router", "input": "legacy session", "stream": true}),
        ))
        .await
        .unwrap();
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        COMPLETED
    );
    let (_, headers, payload) = received.recv().await.unwrap();
    assert_eq!(payload["model"], "gpt-6-astra");
    assert!(!headers.contains_key("authorization"));
}

#[tokio::test]
async fn copilot_api_keys_enable_discovery_without_an_alias() {
    let upstream = copilot_catalog_server().await;
    let config: Config = serde_json::from_value(json!({
        "providers": {"copilot": {"api_key": uuid::Uuid::new_v4().to_string(), "base_url": upstream.url}},
        "responses": {"chatgpt_base_url": format!("{}/chatgpt", upstream.url)},
    })).unwrap();

    let response = crate::make_router(state(config))
        .oneshot(catalog_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let slugs: Vec<_> = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["copilot/gpt-6-astra", "gpt-6-astra", "gpt-6-sol"]);
    assert_eq!(body["models"][0]["display_name"], "GPT-6-Astra (Copilot)");
    assert_eq!(body["models"][1]["display_name"], "GPT-6-Astra (ChatGPT)");
    assert_eq!(body["models"][2]["display_name"], "GPT-6-Sol (ChatGPT)");
}

#[tokio::test]
async fn stored_copilot_accounts_enable_discovery_without_an_alias() {
    let upstream = copilot_catalog_server().await;
    let config: Config = serde_json::from_value(json!({
        "providers": {"copilot": {"base_url": upstream.url}},
        "responses": {"chatgpt_base_url": format!("{}/chatgpt", upstream.url)},
    }))
    .unwrap();
    let state = state(config);
    state
        .auth
        .save_token(
            ProviderId::Copilot,
            byokey_types::OAuthToken::new(uuid::Uuid::new_v4().to_string()).with_client("opencode"),
        )
        .await
        .unwrap();

    let response = crate::make_router(state)
        .oneshot(catalog_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let slugs: Vec<_> = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["copilot/gpt-6-astra", "gpt-6-astra", "gpt-6-sol"]);
}

#[tokio::test]
async fn disabled_copilot_credentials_do_not_enable_discovery() {
    let upstream = copilot_catalog_server().await;
    let config: Config = serde_json::from_value(json!({
        "providers": {"copilot": {"api_key": uuid::Uuid::new_v4().to_string(), "enabled": false, "base_url": upstream.url}},
        "responses": {"chatgpt_base_url": format!("{}/chatgpt", upstream.url)},
    })).unwrap();

    let response = crate::make_router(state(config))
        .oneshot(catalog_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let slugs: Vec<_> = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["gpt-6-astra", "gpt-6-sol"]);
}

#[tokio::test]
async fn manual_aliases_prefer_visible_entries_without_exposing_hidden_models() {
    let config: Config = serde_json::from_value(json!({"responses": {
        "default": "company",
        "catalog": {"model_names": {"deployment": "GPT-5.5"}},
        "upstreams": {"company": {"base_url": "http://127.0.0.1:1", "display_name": "Company"}},
        "models": {
            "alpha": {"upstream": "company", "model": "deployment", "catalog": {
                "display_name": "GPT-5.5", "visibility": "hide", "base_instructions": "retained instructions",
            }},
            "beta": {"upstream": "company", "model": "deployment", "catalog": {
                "display_name": "GPT-5.5 (Company)", "visibility": "list",
            }},
            "gamma": {"upstream": "company", "model": "deployment", "catalog": {
                "display_name": "GPT-5.5", "visibility": "list",
            }},
            "unique": {"upstream": "company", "model": "hidden-deployment", "catalog": {
                "display_name": "Hidden", "visibility": "hide",
            }},
        },
    }})).unwrap();

    let response = crate::make_router(state(config))
        .oneshot(catalog_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let models = body["models"].as_array().unwrap();
    let visibility: Vec<_> = models
        .iter()
        .map(|m| {
            (
                m["slug"].as_str().unwrap(),
                m["visibility"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        visibility,
        [
            ("alpha", "hide"),
            ("beta", "list"),
            ("gamma", "hide"),
            ("unique", "hide")
        ]
    );
    assert_eq!(models[1]["display_name"], "GPT-5.5 (Company)");
    assert_eq!(models[0]["base_instructions"], "retained instructions");
}

#[tokio::test]
async fn catalog_settings_format_names_and_hide_ids_without_changing_routes() {
    let catalog = copilot_catalog_server().await;
    let config: Config = serde_json::from_value(json!({
        "providers": {"copilot": {"api_key": uuid::Uuid::new_v4().to_string(), "base_url": catalog.url}},
        "responses": {
            "chatgpt_base_url": format!("{}/chatgpt", catalog.url),
            "catalog": {
                "name_format": "{{ provider }} / {{ model }}",
                "provider_names": {"chatgpt": "Native & Direct"},
                "model_names": {"gpt-6-astra": "Astra <Fast>"},
                "hidden_aliases": ["gpt-6-astra", "llm-router/gpt-6-sol", "LLM Router"],
            },
            "upstreams": {"llm-router": {
                "base_url": catalog.url, "models_url": format!("{}/models", catalog.url), "display_name": "LLM Router",
            }},
            "models": {
                "native-alias": {"upstream": "chatgpt", "model": "gpt-6-astra"},
                "LLM Router": {"upstream": "llm-router", "model": "gpt-6-astra"},
            },
        },
    })).unwrap();
    let original_route = config.responses.route("LLM Router").unwrap();
    assert_eq!(original_route, ("llm-router", "gpt-6-astra"));

    let response = crate::make_router(state(config))
        .oneshot(catalog_request())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let models = body["models"].as_array().unwrap();
    let visible: Vec<_> = models
        .iter()
        .filter(|m| m["visibility"] == "list")
        .map(|m| {
            (
                m["slug"].as_str().unwrap(),
                m["display_name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        visible,
        [
            ("copilot/gpt-6-astra", "Copilot / Astra <Fast>"),
            ("gpt-6-sol", "Native & Direct / GPT-6-Sol"),
            ("llm-router/gpt-6-astra", "LLM Router / Astra <Fast>"),
            ("native-alias", "Native & Direct / Astra <Fast>"),
        ]
    );
    assert_eq!(models[0]["slug"], "LLM Router");
    assert_eq!(models[0]["visibility"], "hide");
    assert_eq!(models[2]["slug"], "gpt-6-astra");
    assert_eq!(models[2]["visibility"], "hide");
    assert_eq!(models[2]["context_window"], 400_000);
    assert_eq!(models[5]["slug"], "llm-router/gpt-6-sol");
    assert_eq!(models[5]["visibility"], "hide");
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
            "display_name": "Company Gateway",
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
    assert_eq!(models[0]["display_name"], "Large (Company Gateway)");
    assert_eq!(models[0]["context_window"], 400_000);
    assert_eq!(
        models[0]["model_messages"]["instructions_template"],
        "large instructions"
    );
    assert_eq!(models[0]["future_capability"], json!({"enabled": true}));
    assert!(models[0].get("base_instructions").is_none());
    assert_eq!(models[1]["display_name"], "Small (Company Gateway)");
    assert_eq!(models[1]["context_window"], 64_000);
    assert_eq!(models[1]["base_instructions"], "small instructions");
    assert_eq!(models[1]["upgrade"], Value::Null);
    assert_eq!(models[2]["display_name"], "Large (ChatGPT)");
    assert_eq!(models[3]["display_name"], "Small (ChatGPT)");
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
    assert_eq!(models[0]["visibility"], "hide");
    assert_eq!(models[1]["display_name"], "Pinned (company)");
    assert_ne!(models[1]["visibility"], "hide");
    assert_eq!(models[1]["base_instructions"], "deployment instructions");
    assert_eq!(models[2]["display_name"], "Large (company)");
    assert_ne!(models[3]["visibility"], "hide");
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
