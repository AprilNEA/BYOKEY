//! Models listing handler — returns available models in `OpenAI` format.

use axum::{Json, extract::State};
use byokey_provider::all_models;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;

/// Handles `GET /v1/models` requests.
///
/// Returns an OpenAI-compatible model list from the unified registry.
/// For models available on multiple providers, both unqualified (primary)
/// and qualified (`provider/model`) forms are listed.
pub async fn list_models(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut data = Vec::new();
    let config = state.config.load();

    for entry in all_models() {
        let Some(primary_provider) = entry.providers.first() else {
            continue;
        };

        let primary_pc = config
            .providers
            .get(primary_provider)
            .cloned()
            .unwrap_or_default();
        let primary_enabled =
            primary_pc.enabled && !config.is_model_excluded(primary_provider, entry.id);

        // List the unqualified model under its primary provider if enabled.
        if primary_enabled {
            let aliases = config.model_alias.get(primary_provider);
            let alias_entry = aliases.and_then(|a| a.iter().find(|ae| ae.name == entry.id));

            if let Some(ae) = alias_entry {
                data.push(json!({
                    "id": ae.alias,
                    "object": "model",
                    "created": 0,
                    "owned_by": primary_provider.to_string(),
                }));
                if ae.fork {
                    data.push(json!({
                        "id": entry.id,
                        "object": "model",
                        "created": 0,
                        "owned_by": primary_provider.to_string(),
                    }));
                }
            } else {
                data.push(json!({
                    "id": entry.id,
                    "object": "model",
                    "created": 0,
                    "owned_by": primary_provider.to_string(),
                }));
            }
        }

        // Emit qualified alternatives for other providers, regardless of
        // whether the primary is enabled.
        for alt_provider in &entry.providers[1..] {
            let alt_pc = config
                .providers
                .get(alt_provider)
                .cloned()
                .unwrap_or_default();
            if !alt_pc.enabled {
                continue;
            }
            if config.is_model_excluded(alt_provider, entry.id) {
                continue;
            }
            data.push(json!({
                "id": format!("{}/{}", alt_provider, entry.id),
                "object": "model",
                "created": 0,
                "owned_by": alt_provider.to_string(),
            }));
        }
    }

    Json(json!({
        "object": "list",
        "data": data,
    }))
}
