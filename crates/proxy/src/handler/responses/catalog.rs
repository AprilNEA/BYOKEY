//! Codex-native model metadata. Aliases inherit metadata, never fabricated instructions.

use axum::{
    Json,
    extract::{OriginalUri, State},
    http::HeaderMap,
};
use byokey_types::{ByokError, ProviderId};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

use super::{chatgpt_headers, copilot_upstream, endpoint, require_chatgpt_auth};
use crate::{ApiError, AppState};

pub(crate) async fn models(
    State(state): State<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let config = state.config.load_full();
    let settings = &config.responses;
    let needs_catalog = matches!(settings.default.as_str(), "chatgpt" | "copilot")
        || settings
            .models
            .values()
            .any(|model| model.catalog.is_none());
    let originals = if needs_catalog {
        fetch_catalog(
            &state.http,
            &settings.chatgpt_base_url,
            uri.query(),
            headers,
        )
        .await?
    } else {
        BTreeMap::new()
    };

    let uses_copilot =
        settings.default == "copilot" || settings.models.values().any(|m| m.upstream == "copilot");
    let copilot = if uses_copilot {
        if config
            .providers
            .get(&ProviderId::Copilot)
            .is_some_and(|c| !c.enabled)
        {
            return Err(ByokError::UnsupportedProvider("copilot is disabled".into()).into());
        }
        copilot_upstream(&state)
            .models()
            .await?
            .into_iter()
            .filter(|m| m.responses)
            .map(|m| (m.id.clone(), m))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let mut output = BTreeMap::new();
    for (slug, metadata) in &originals {
        if settings.default == "chatgpt" {
            output.insert(slug.clone(), metadata.clone());
        }
        if let Some(model) = copilot.get(slug) {
            let mut metadata = metadata.clone();
            metadata["upgrade"] = Value::Null;
            cap_context(&mut metadata, model.context_window);
            if settings.default == "copilot" {
                output.insert(slug.clone(), metadata.clone());
            }
            let alias = format!("copilot/{slug}");
            metadata["slug"] = json!(alias);
            metadata["display_name"] = json!(format!("{} (Copilot)", model.name));
            output.insert(alias, metadata);
        }
    }
    for (alias, route) in &settings.models {
        let source = route.catalog_model.as_deref().unwrap_or(&route.model);
        let mut metadata = route
            .catalog
            .as_ref()
            .or_else(|| originals.get(source))
            .cloned()
            .ok_or_else(|| {
                ByokError::Config(format!(
                    "alias {alias} needs catalog metadata or a valid catalog_model"
                ))
            })?;
        if route.upstream == "copilot" {
            let model = copilot.get(&route.model).ok_or_else(|| {
                ByokError::UnsupportedModel(format!(
                    "{} does not support Copilot /responses",
                    route.model
                ))
            })?;
            cap_context(&mut metadata, model.context_window);
        }
        metadata["slug"] = json!(alias);
        metadata["upgrade"] = Value::Null;
        if route.catalog.is_none() {
            metadata["display_name"] = json!(alias);
        }
        output.insert(alias.clone(), metadata);
    }
    // Codex limits custom catalogs to 1 MiB and ignores legacy instructions when a template exists.
    for metadata in output.values_mut().filter_map(Value::as_object_mut) {
        if metadata
            .get("model_messages")
            .and_then(|messages| messages.get("instructions_template"))
            .is_some_and(Value::is_string)
        {
            metadata.remove("base_instructions");
        }
    }
    Ok(Json(
        json!({"models": output.into_values().collect::<Vec<_>>()}),
    ))
}

fn cap_context(metadata: &mut Value, limit: Option<u64>) {
    if let Some(limit) = limit {
        let limit = metadata
            .get("context_window")
            .and_then(Value::as_u64)
            .map_or(limit, |n| n.min(limit));
        metadata["context_window"] = json!(limit);
    }
}

async fn fetch_catalog(
    http: &reqwest::Client,
    base: &str,
    query: Option<&str>,
    headers: HeaderMap,
) -> Result<BTreeMap<String, Value>, ByokError> {
    require_chatgpt_auth(&headers)?;
    let mut headers = chatgpt_headers(headers);
    // This endpoint changes the catalog, so upstream cache validators do not apply.
    headers.remove("if-none-match");
    headers.remove("if-modified-since");
    let mut url = reqwest::Url::parse(&endpoint(base, "models"))
        .map_err(|_| ByokError::Config("invalid ChatGPT catalog URL".into()))?;
    url.set_query(query);
    let response = http
        .get(url)
        .headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| ByokError::Http(e.without_url().to_string()))?;
    if !response.status().is_success() {
        return Err(ByokError::from_response(response).await);
    }
    let body: Value = response.json().await.map_err(ByokError::from)?;
    let models = body
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| ByokError::Http("ChatGPT returned no model catalog".into()))?;
    let mut originals = BTreeMap::new();
    for model in models {
        if let Some(slug) = model.get("slug").and_then(Value::as_str) {
            originals.insert(slug.to_owned(), model.clone());
        }
    }
    Ok(originals)
}
