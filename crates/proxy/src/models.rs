//! Models listing handler — returns available models in `OpenAI` format.

use axum::{Json, extract::State};
use byokey_provider::make_executor;
use byokey_types::ProviderId;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;

/// Handles `GET /v1/models` requests.
///
/// Returns an OpenAI-compatible model list containing all models from
/// enabled providers. Providers absent from the config are enabled by default.
pub async fn list_models(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut data = Vec::new();
    let config = state.config.load();

    for provider_id in ProviderId::all() {
        let provider_config = config
            .providers
            .get(provider_id)
            .cloned()
            .unwrap_or_default();
        if !provider_config.enabled {
            continue;
        }
        let api_key = provider_config.api_key.clone();
        if let Some(executor) =
            make_executor(provider_id, api_key, state.auth.clone(), state.http.clone())
        {
            let aliases = config.model_alias.get(provider_id);

            for model_id in executor.supported_models() {
                if config.is_model_excluded(provider_id, &model_id) {
                    continue;
                }

                // Check if this model has an alias configured.
                let alias_entry =
                    aliases.and_then(|a| a.iter().find(|entry| entry.name == model_id));

                if let Some(entry) = alias_entry {
                    // Always expose the alias name.
                    data.push(json!({
                        "id": entry.alias,
                        "object": "model",
                        "created": 0,
                        "owned_by": provider_id.to_string(),
                    }));
                    // If fork mode, also keep the original.
                    if entry.fork {
                        data.push(json!({
                            "id": model_id,
                            "object": "model",
                            "created": 0,
                            "owned_by": provider_id.to_string(),
                        }));
                    }
                } else {
                    data.push(json!({
                        "id": model_id,
                        "object": "model",
                        "created": 0,
                        "owned_by": provider_id.to_string(),
                    }));
                }
            }
        }
    }

    Json(json!({
        "object": "list",
        "data": data,
    }))
}
