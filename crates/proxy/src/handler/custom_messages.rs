//! Native Messages for configured upstreams, without stored or client credentials.

use axum::http::{HeaderMap, HeaderValue};
use byokey_config::AnthropicProviderConfig;
use byokey_provider::claude::ANTHROPIC_VERSION;
use byokey_types::ByokError;
use serde::Deserialize;

use crate::ApiError;

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
pub(super) struct Model {
    pub id: String,
    pub display_name: Option<String>,
}

fn headers(upstream: &AnthropicProviderConfig) -> Result<HeaderMap, ByokError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "anthropic-version",
        HeaderValue::from_static(ANTHROPIC_VERSION),
    );
    if let Some(key) = &upstream.api_key {
        let mut value = HeaderValue::from_str(&key.resolve()?)
            .map_err(|_| ByokError::Config("invalid upstream API key header".into()))?;
        value.set_sensitive(true);
        headers.insert("x-api-key", value);
    }
    super::headers::apply(&mut headers, &upstream.headers)?;
    Ok(headers)
}

pub(super) async fn models(
    http: &reqwest::Client,
    upstream: &AnthropicProviderConfig,
) -> Result<Vec<Model>, ApiError> {
    #[derive(Deserialize)]
    struct List {
        data: Vec<Model>,
    }
    let Some(url) = &upstream.models_url else {
        return Ok(upstream
            .enabled_models
            .iter()
            .flatten()
            .filter(|id| id.starts_with("claude-"))
            .map(|id| Model {
                id: id.clone(),
                display_name: None,
            })
            .collect());
    };
    let response = http
        .get(url)
        .headers(headers(upstream)?)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(ByokError::from)?;
    if !response.status().is_success() {
        return Err(ByokError::from_response(response).await.into());
    }
    let list: List = response.json().await.map_err(ByokError::from)?;
    // Mixed-protocol catalogs also contain GPT models that Claude clients cannot configure.
    Ok(list
        .data
        .into_iter()
        .filter(|model| model.id.starts_with("claude-"))
        .filter(|model| {
            upstream
                .enabled_models
                .as_ref()
                .is_none_or(|ids| ids.contains(&model.id))
        })
        .collect())
}

pub(super) fn request(
    http: &reqwest::Client,
    upstream: &AnthropicProviderConfig,
    path: &str,
    beta: &str,
) -> Result<reqwest::RequestBuilder, ByokError> {
    let mut headers = headers(upstream)?;
    if !upstream
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("anthropic-beta"))
    {
        headers.insert(
            "anthropic-beta",
            HeaderValue::from_str(beta)
                .map_err(|_| ByokError::InvalidRequest("invalid anthropic-beta header".into()))?,
        );
    }
    Ok(http
        .post(format!("{}{path}", upstream.base_url.trim_end_matches('/')))
        .headers(headers))
}
