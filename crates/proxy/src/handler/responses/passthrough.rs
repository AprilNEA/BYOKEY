//! Native Codex HTTP forwarding without model routing or body interpretation.

use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
};
use byokey_types::ByokError;
use futures_util::TryStreamExt as _;
use std::sync::Arc;

use super::{chatgpt_headers, require_chatgpt_auth, strip_hop_headers};
use crate::{ApiError, AppState};

pub(crate) async fn passthrough(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Result<Response, ApiError> {
    let (parts, body) = request.into_parts();
    require_chatgpt_auth(&parts.headers)?;
    let config = state.config.load_full();
    if config.providers.get("chatgpt").is_some_and(|p| !p.enabled) {
        return Err(ByokError::UnsupportedProvider("chatgpt is disabled".into()).into());
    }
    let mut url = reqwest::Url::parse(&format!(
        "{}/",
        config.chatgpt_base_url().trim_end_matches('/')
    ))
    .map_err(|e| ByokError::Config(format!("invalid ChatGPT backend URL: {e}")))?;
    let root = url.path().to_owned();
    let path = parts
        .uri
        .path()
        .strip_prefix("/codex/")
        .expect("passthrough is only registered for /codex/*");
    url.set_path(&format!("{root}{path}"));
    url.set_query(parts.uri.query());
    // URL parsing normalizes encoded parent segments. Keep requests inside the backend root.
    if !url.path().starts_with(&root) {
        return Err(ByokError::InvalidRequest("path escapes the ChatGPT backend".into()).into());
    }

    let upstream = state
        .passthrough_http
        .request(parts.method, url)
        .headers(chatgpt_headers(parts.headers))
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(|e| ByokError::from(e.without_url()))?;
    let status = upstream.status();
    let mut headers = upstream.headers().clone();
    strip_hop_headers(&mut headers);
    let mut response = Response::new(Body::from_stream(
        upstream.bytes_stream().map_err(reqwest::Error::without_url),
    ));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}
