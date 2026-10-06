use super::*;
use axum::http::Method;
use bytes::Bytes;
use futures_util::StreamExt as _;
use std::{io, time::Duration};
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;

fn capture_raw(
    status: StatusCode,
    headers: &[(&str, &str)],
    body: &'static [u8],
) -> (Router, mpsc::UnboundedReceiver<Request<Bytes>>) {
    let headers: HeaderMap = headers
        .iter()
        .map(|(name, value)| (name.parse::<HeaderName>().unwrap(), value.parse().unwrap()))
        .collect();
    let (tx, rx) = mpsc::unbounded_channel();
    let router = Router::new().fallback(move |request: Request<Body>| {
        let tx = tx.clone();
        let headers = headers.clone();
        async move {
            let (parts, body_in) = request.into_parts();
            let bytes = to_bytes(body_in, usize::MAX).await.unwrap();
            tx.send(Request::from_parts(parts, bytes)).unwrap();
            let mut response = Response::new(Body::from(body));
            *response.status_mut() = status;
            *response.headers_mut() = headers;
            response
        }
    });
    (router, rx)
}

fn native_request(method: Method, path: &str, body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer client-chatgpt-token")
        .header("chatgpt-account-id", "client-account")
        .body(body.into())
        .unwrap()
}

#[tokio::test]
async fn image_generation_uses_chatgpt_even_with_a_custom_default() {
    const REQUEST: &str = "{ \"model\":\"gpt-image-2\", \"prompt\":\"a fox\", \"future\":[3,1] }";
    const RESPONSE: &[u8] = br#"{"data":[{"b64_json":"image"}]}"#;
    let (router, mut received) = capture_raw(
        StatusCode::CREATED,
        &[
            ("content-type", "application/json"),
            ("x-codex-imagegen-request-id", "image-request"),
            ("connection", "x-backend-hop"),
            ("x-backend-hop", "private"),
        ],
        RESPONSE,
    );
    let native = serve(router).await;
    let (router, mut other_requests) = capture_raw(StatusCode::OK, &[], b"wrong upstream");
    let other = serve(router).await;
    let config = serde_json::from_value(json!({
        "providers": {
            "chatgpt": {"base_url": format!("{}/backend-api/codex/", native.url)},
            "company": {"base_url": other.url, "api_key": "company-key"},
        },
        "responses": {"routes": {"default": "company"}},
    }))
    .unwrap();
    let mut request = native_request(
        Method::POST,
        "/codex/images/generations?tag=a%2Fb&tag=c+z",
        REQUEST,
    );
    request.headers_mut().extend(HeaderMap::from_iter([
        (
            HeaderName::from_static("content-type"),
            "application/json".parse().unwrap(),
        ),
        (
            HeaderName::from_static("cookie"),
            "private-cookie".parse().unwrap(),
        ),
        (
            HeaderName::from_static("connection"),
            "x-client-hop".parse().unwrap(),
        ),
        (
            HeaderName::from_static("x-client-hop"),
            "private".parse().unwrap(),
        ),
        (
            HeaderName::from_static("proxy-authorization"),
            "private-proxy".parse().unwrap(),
        ),
        (
            HeaderName::from_static("x-oai-attestation"),
            "client-attestation".parse().unwrap(),
        ),
    ]));

    let response = crate::make_router(state(config))
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers()["x-codex-imagegen-request-id"],
        "image-request"
    );
    assert!(!response.headers().contains_key("x-backend-hop"));
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        RESPONSE
    );
    let request = received.recv().await.unwrap();
    assert_eq!(request.method(), Method::POST);
    assert_eq!(
        request.uri(),
        "/backend-api/codex/images/generations?tag=a%2Fb&tag=c+z"
    );
    assert_eq!(request.body(), REQUEST);
    assert_eq!(request.headers()["content-type"], "application/json");
    assert_eq!(
        request.headers()["authorization"],
        "Bearer client-chatgpt-token"
    );
    assert_eq!(request.headers()["chatgpt-account-id"], "client-account");
    assert_eq!(request.headers()["x-oai-attestation"], "client-attestation");
    assert!(!request.headers().contains_key("cookie"));
    assert!(!request.headers().contains_key("x-client-hop"));
    assert!(!request.headers().contains_key("proxy-authorization"));
    assert!(other_requests.try_recv().is_err());
}

#[tokio::test]
async fn unknown_endpoints_preserve_methods_encoded_paths_and_compressed_bytes() {
    const GZIP: &[u8] = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff\xcb/H,,MU(J-.\xc8\xcf+Ne\xf8\x0f\x00\x9e\xfb6Q\x11\x00\x00\x00";
    let (router, mut received) = capture_raw(
        StatusCode::OK,
        &[
            ("content-type", "application/octet-stream"),
            ("content-encoding", "gzip"),
        ],
        GZIP,
    );
    let native = serve(router).await;
    let config = chatgpt_config(format!("{}/backend-api/codex", native.url));
    let mut request = native_request(Method::PATCH, "/codex/future/item%2Fone?x=%26", GZIP);
    request
        .headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    request
        .headers_mut()
        .insert("content-type", "application/octet-stream".parse().unwrap());

    let response = crate::make_router(state(config))
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        GZIP
    );
    let request = received.recv().await.unwrap();
    assert_eq!(request.method(), Method::PATCH);
    assert_eq!(request.uri(), "/backend-api/codex/future/item%2Fone?x=%26");
    assert_eq!(request.headers()["content-encoding"], "gzip");
    assert_eq!(request.body(), GZIP);
}

#[tokio::test]
async fn upstream_auth_failures_return_unchanged_without_retry() {
    let (router, mut received) = capture_raw(
        StatusCode::UNAUTHORIZED,
        &[
            ("retry-after", "17"),
            ("www-authenticate", "Bearer"),
            ("content-type", "text/plain"),
        ],
        b"refresh the client login",
    );
    let native = serve(router).await;
    let config = chatgpt_config(native.url.clone());

    let response = crate::make_router(state(config))
        .oneshot(native_request(
            Method::POST,
            "/codex/images/edits",
            "opaque edit request",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["retry-after"], "17");
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "refresh the client login"
    );
    assert_eq!(received.recv().await.unwrap().uri(), "/images/edits");
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn native_redirects_do_not_receive_a_second_request() {
    let (router, mut received) = capture_raw(StatusCode::OK, &[], b"credential leak");
    let destination = serve(router).await;
    let (router, mut redirects) = capture_raw(
        StatusCode::TEMPORARY_REDIRECT,
        &[("location", &destination.url)],
        b"redirect body",
    );
    let native = serve(router).await;
    let config = chatgpt_config(native.url.clone());

    let response = crate::make_router(state(config))
        .oneshot(native_request(Method::GET, "/codex/future", Body::empty()))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.headers()["location"], destination.url);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "redirect body"
    );
    assert_eq!(redirects.recv().await.unwrap().uri(), "/future");
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn native_requests_require_the_clients_login() {
    let (router, mut received) = capture_raw(StatusCode::OK, &[], b"unexpected request");
    let native = serve(router).await;
    let config = chatgpt_config(native.url.clone());

    let response = crate::make_router(state(config))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/images/generations")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn paths_outside_codex_do_not_reach_chatgpt() {
    let (router, mut received) = capture_raw(StatusCode::OK, &[], b"unexpected request");
    let native = serve(router).await;
    let config = chatgpt_config(native.url.clone());

    let response = crate::make_router(state(config))
        .oneshot(native_request(
            Method::POST,
            "/codex-other/images/generations",
            "{}",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn encoded_parent_paths_cannot_escape_the_configured_backend() {
    let (router, mut received) = capture_raw(StatusCode::OK, &[], b"unexpected request");
    let native = serve(router).await;
    let config = chatgpt_config(format!("{}/backend-api/codex", native.url));

    let response = crate::make_router(state(config))
        .oneshot(native_request(
            Method::GET,
            "/codex/%2e%2e/files",
            Body::empty(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn codex_responses_keeps_model_routing_instead_of_using_the_fallback() {
    let (router, mut native_requests) = capture_raw(StatusCode::OK, &[], b"wrong upstream");
    let native = serve(router).await;
    let (router, mut received) = capture(StatusCode::OK, COMPLETED);
    let company = serve(router).await;
    let config = serde_json::from_value(json!({"providers": {
        "chatgpt": {"base_url": native.url},
        "company": {"base_url": company.url, "api_key": "company-key"},
    }}))
    .unwrap();
    let mut request = request(&json!({"model":"company/deployment", "input":"hi", "stream":true}));
    *request.uri_mut() = "/codex/responses".parse().unwrap();

    let response = crate::make_router(state(config))
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        COMPLETED
    );
    let (_, headers, body) = received.recv().await.unwrap();
    assert_eq!(headers["authorization"], "Bearer company-key");
    assert!(!headers.contains_key("chatgpt-account-id"));
    assert_eq!(body["model"], "deployment");
    assert!(native_requests.try_recv().is_err());
}

#[tokio::test]
async fn native_responses_stream_unchanged_and_close_when_the_client_leaves() {
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
    let rx = Arc::new(tokio::sync::Mutex::new(Some(rx)));
    let native = serve(Router::new().fallback(move || {
        let rx = rx.clone();
        async move {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(ReceiverStream::new(
                    rx.lock().await.take().unwrap(),
                )))
                .unwrap()
        }
    }))
    .await;
    let config = chatgpt_config(native.url.clone());
    let response = timeout(
        Duration::from_secs(2),
        crate::make_router(state(config)).oneshot(native_request(
            Method::GET,
            "/codex/future/events",
            Body::empty(),
        )),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();

    tx.send(Ok(Bytes::from_static(b"event: future\ndata: opaque\n\n")))
        .await
        .unwrap();

    let first = timeout(Duration::from_secs(2), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first, "event: future\ndata: opaque\n\n");
    drop(body);
    timeout(Duration::from_secs(2), tx.closed()).await.unwrap();
}

#[tokio::test]
async fn native_uploads_reach_the_backend_before_the_body_finishes() {
    let (observed, mut received) = mpsc::unbounded_channel();
    let native = serve(Router::new().fallback(move |request: Request<Body>| {
        let observed = observed.clone();
        async move {
            let mut body = request.into_body().into_data_stream();
            observed.send(body.next().await.unwrap().unwrap()).unwrap();
            "upload started"
        }
    }))
    .await;
    let config = chatgpt_config(native.url.clone());
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
    tx.send(Ok(Bytes::from_static(b"first upload chunk")))
        .await
        .unwrap();

    let response = timeout(
        Duration::from_secs(2),
        crate::make_router(state(config)).oneshot(native_request(
            Method::POST,
            "/codex/future/upload",
            Body::from_stream(ReceiverStream::new(rx)),
        )),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(received.recv().await.unwrap(), "first upload chunk");
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "upload started"
    );
}
