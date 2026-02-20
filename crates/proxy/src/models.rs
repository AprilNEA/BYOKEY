//! Models listing handler — returns available models in `OpenAI` format.

use axum::{Json, extract::State};
use byok_provider::make_executor;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;

/// Handles `GET /v1/models` requests.
///
/// Returns an OpenAI-compatible model list containing all models from
/// enabled providers in the configuration.
pub async fn list_models(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut data = Vec::new();

    for (provider_id, config) in &state.config.providers {
        if !config.enabled {
            continue;
        }
        let api_key = config.api_key.clone();
        if let Some(executor) = make_executor(provider_id, api_key, state.auth.clone()) {
            for model_id in executor.supported_models() {
                data.push(json!({
                    "id": model_id,
                    "object": "model",
                    "created": 0,
                    "owned_by": provider_id.to_string(),
                }));
            }
        }
    }

    Json(json!({
        "object": "list",
        "data": data,
    }))
}
