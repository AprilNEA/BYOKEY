//! Codex-native model metadata. Aliases inherit metadata, never fabricated instructions.

use axum::{
    Json,
    extract::{OriginalUri, State},
    http::HeaderMap,
};
use byokey_config::Config;
use byokey_provider::CopilotModel;
use byokey_types::{ByokError, ProviderId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

use super::{chatgpt_headers, copilot_upstream, custom_headers, endpoint, require_chatgpt_auth};
use crate::{ApiError, AppState};

pub(crate) async fn models(
    State(state): State<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let config = state.config.load_full();
    let settings = &config.responses;
    let routes = &settings.routes;
    let copilot_config = config.providers.get("copilot");
    let requires_copilot =
        routes.default == "copilot" || routes.models.values().any(|m| m.provider == "copilot");
    let copilot_enabled = copilot_config.is_none_or(|c| c.enabled);
    if requires_copilot && !copilot_enabled {
        return Err(ByokError::UnsupportedProvider("copilot is disabled".into()).into());
    }
    let uses_copilot = requires_copilot
        || (copilot_enabled
            && (copilot_config.is_some_and(|c| c.api_key.is_some())
                || !state
                    .auth
                    .list_accounts(ProviderId::Copilot)
                    .await?
                    .is_empty()));
    let needs_catalog = routes.default == "chatgpt"
        || uses_copilot
        || routes.models.values().any(|route| {
            config
                .model_override(&route.provider, &route.model)
                .is_none_or(|m| m.catalog.is_none())
        })
        || config
            .providers
            .values()
            .any(|p| p.enabled && p.models_url.is_some());
    let originals = if needs_catalog {
        if config.providers.get("chatgpt").is_some_and(|p| !p.enabled) {
            return Err(ByokError::UnsupportedProvider(
                "chatgpt is disabled; Codex metadata requires the ChatGPT catalog".into(),
            )
            .into());
        }
        fetch_catalog(&state.http, config.chatgpt_base_url(), uri.query(), headers).await?
    } else {
        BTreeMap::new()
    };

    let copilot = if uses_copilot {
        copilot_upstream(&state)?
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
    if routes.default == "chatgpt" {
        for slug in originals.keys() {
            if let Some(metadata) = model_metadata(&config, &originals, "chatgpt", slug) {
                let mut metadata = metadata.clone();
                metadata["slug"] = json!(slug);
                output.insert(slug.clone(), metadata);
            }
        }
    }
    for (slug, model) in &copilot {
        if let Some(metadata) = model_metadata(&config, &originals, "copilot", slug) {
            let mut metadata = metadata.clone();
            metadata["upgrade"] = Value::Null;
            cap_context(&mut metadata, model.context_window);
            if routes.default == "copilot" {
                metadata["slug"] = json!(slug);
                output.insert(slug.clone(), metadata.clone());
            }
            let alias = format!("copilot/{slug}");
            metadata["slug"] = json!(alias);
            output.insert(alias, metadata);
        }
    }
    add_custom_models(&state.http, &config, &originals, &mut output).await?;
    add_aliases(&config, &originals, &copilot, &mut output)?;
    if settings.auto_review_follow_provider {
        for (slug, metadata) in &mut output {
            if config.response_route(slug)?.0 != "chatgpt" {
                metadata["auto_review_model_override"] = json!(slug);
            }
        }
    }
    present_models(&config, &mut output)?;
    Ok(Json(
        json!({"models": output.into_values().collect::<Vec<_>>()}),
    ))
}

fn add_aliases(
    config: &Config,
    originals: &BTreeMap<String, Value>,
    copilot: &BTreeMap<String, CopilotModel>,
    output: &mut BTreeMap<String, Value>,
) -> Result<(), ByokError> {
    for (alias, route) in &config.responses.routes.models {
        config.response_route(alias)?;
        let mut metadata = model_metadata(config, originals, &route.provider, &route.model)
            .cloned()
            .ok_or_else(|| {
                ByokError::Config(format!(
                    "providers.{}.model_overrides.{} needs catalog metadata or a valid catalog_model for alias {alias}", route.provider, route.model
                ))
            })?;
        if route.provider == "copilot" {
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
        output.insert(alias.clone(), metadata);
    }
    Ok(())
}

fn present_models(config: &Config, models: &mut BTreeMap<String, Value>) -> Result<(), ByokError> {
    let settings = &config.responses;
    let format_name = settings.catalog.name_formatter()?;
    let mut preferred = BTreeMap::new();
    for (slug, metadata) in models.iter_mut() {
        if settings.catalog.hidden_aliases.contains(slug) {
            metadata["visibility"] = json!("hide");
        }
        let (upstream, model) = config.response_route(slug)?;
        let rank = if slug == model {
            0
        } else if *slug == format!("{upstream}/{model}") {
            1
        } else {
            2
        };
        let hidden = matches!(metadata["visibility"].as_str(), Some("hide" | "none"));
        let candidate = (hidden, rank, slug.clone());
        preferred
            .entry((upstream.to_owned(), model.to_owned()))
            .and_modify(|current| {
                if candidate < *current {
                    current.clone_from(&candidate);
                }
            })
            .or_insert(candidate);
    }
    for (slug, metadata) in models.iter_mut() {
        let (upstream, model) = config.response_route(slug)?;
        let label = config.provider_name(upstream);
        let name = config
            .model_override(upstream, model)
            .and_then(|patch| patch.name.as_deref())
            .or_else(|| metadata["display_name"].as_str())
            .unwrap_or(model);
        metadata["display_name"] = json!(format_name(name, label)?);
        if preferred[&(upstream.to_owned(), model.to_owned())].2 != *slug {
            // Hidden aliases retain metadata for existing sessions and explicit selection.
            metadata["visibility"] = json!("hide");
        }
    }
    // Codex ignores legacy instructions when a template exists; omit that duplicate to limit catalog size.
    for metadata in models.values_mut().filter_map(Value::as_object_mut) {
        if metadata
            .get("model_messages")
            .and_then(|messages| messages.get("instructions_template"))
            .is_some_and(Value::is_string)
        {
            metadata.remove("base_instructions");
        }
    }
    Ok(())
}

async fn add_custom_models(
    http: &reqwest::Client,
    config: &Config,
    originals: &BTreeMap<String, Value>,
    output: &mut BTreeMap<String, Value>,
) -> Result<(), ApiError> {
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    #[derive(Deserialize)]
    struct ModelList {
        data: Vec<Model>,
    }

    for (name, upstream) in config.providers.iter().filter(|(_, p)| p.enabled) {
        let Some(url) = &upstream.models_url else {
            continue;
        };
        let response = http
            .get(url)
            .headers(custom_headers(upstream, &HeaderMap::new())?)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| ByokError::Http(e.without_url().to_string()))?;
        if !response.status().is_success() {
            return Err(ApiError::from_response(response).await);
        }
        let models: ModelList = response.json().await.map_err(ByokError::from)?;
        for model in models.data {
            let Some(metadata) = model_metadata(config, originals, name, &model.id) else {
                continue;
            };
            let mut metadata = metadata.clone();
            metadata["upgrade"] = Value::Null;
            if config.responses.routes.default == *name {
                metadata["slug"] = json!(model.id);
                output.insert(model.id.clone(), metadata.clone());
            }
            let alias = format!("{name}/{}", model.id);
            metadata["slug"] = json!(alias);
            output.insert(alias, metadata);
        }
    }
    Ok(())
}

fn model_metadata<'a>(
    config: &'a Config,
    originals: &'a BTreeMap<String, Value>,
    provider: &str,
    model: &str,
) -> Option<&'a Value> {
    let patch = config.model_override(provider, model);
    patch.and_then(|p| p.catalog.as_ref()).or_else(|| {
        let source = patch
            .and_then(|p| p.catalog_model.as_deref())
            .unwrap_or(model);
        originals.get(source)
    })
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
