//! Models listing handler — returns available models in `OpenAI` format.

use axum::{Json, extract::State};
use byokey_provider::{CursorExecutor, all_models};
use byokey_types::ProviderId;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::AppState;

/// OpenAI-compatible model list response.
#[derive(Serialize, ToSchema)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<ModelEntry>,
}

/// A single model entry.
#[derive(Serialize, ToSchema)]
pub struct ModelEntry {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
    /// Label for pickers such as Claude Code's `/model`, when the upstream
    /// names the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl ModelEntry {
    fn new(id: String, owned_by: &ProviderId) -> Self {
        Self {
            id,
            object: "model".into(),
            created: 0,
            owned_by: owned_by.to_string(),
            display_name: None,
        }
    }
}

/// Handles `GET /v1/models` requests.
///
/// Returns an OpenAI-compatible model list from the unified registry.
/// For models available on multiple providers, both unqualified (primary)
/// and qualified (`provider/model`) forms are listed. The signed-in Copilot
/// and Cursor accounts' Claude models are added from their live catalogs,
/// with display names, for Claude Code's gateway model discovery.
#[utoipa::path(
    get,
    path = "/v1/models",
    responses((status = 200, body = ModelsResponse)),
    tag = "management"
)]
pub async fn list_models(State(state): State<Arc<AppState>>) -> Json<ModelsResponse> {
    let mut data: Vec<ModelEntry> = Vec::new();
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
                data.push(ModelEntry::new(ae.alias.clone(), primary_provider));
                if ae.fork {
                    data.push(ModelEntry::new(entry.id.to_string(), primary_provider));
                }
            } else {
                data.push(ModelEntry::new(entry.id.to_string(), primary_provider));
            }
        }

        // Emit qualified alternatives for all providers on multi-provider
        // models (including the primary, for explicit discoverability).
        if entry.providers.len() > 1 {
            for alt_provider in entry.providers {
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
                data.push(ModelEntry::new(
                    format!("{}/{}", alt_provider, entry.id),
                    alt_provider,
                ));
            }
        }
    }

    for entry in live_models(&state, &config).await {
        if !data.iter().any(|e| e.id == entry.id) {
            data.push(entry);
        }
    }

    Json(ModelsResponse {
        object: "list".into(),
        data,
    })
}

/// Claude models the signed-in Copilot and Cursor accounts serve on
/// `/v1/messages`, qualified by provider so `/v1/messages` routes them back.
/// A provider whose listing fails is left out; the static registry above
/// still lists its known models.
async fn live_models(state: &AppState, config: &byokey_config::Config) -> Vec<ModelEntry> {
    let enabled = |p: &ProviderId| config.providers.get(p).is_none_or(|c| c.enabled);
    let mut out = Vec::new();
    for provider in [ProviderId::Copilot, ProviderId::Cursor] {
        let configured = config
            .providers
            .get(&provider)
            .and_then(|c| c.api_key.clone());
        if !enabled(&provider)
            || (configured.is_none() && !state.auth.is_authenticated(&provider).await)
        {
            continue;
        }
        let listing = match provider {
            ProviderId::Copilot => {
                super::messages::copilot_executor(state)
                    .0
                    .messages_models()
                    .await
            }
            _ => {
                CursorExecutor::builder()
                    .http(state.http.clone())
                    .auth(state.auth.clone())
                    .maybe_api_key(configured)
                    .build()
                    .models()
                    .await
            }
        };
        match listing {
            Ok(models) => out.extend(
                models
                    .into_iter()
                    .filter(|(id, _)| is_claude(id))
                    .filter(|(id, _)| !config.is_model_excluded(&provider, id))
                    .map(|(id, name)| ModelEntry {
                        display_name: Some(name),
                        ..ModelEntry::new(format!("{provider}/{id}"), &provider)
                    }),
            ),
            Err(e) => tracing::warn!(%provider, error = %e, "live model listing failed"),
        }
    }
    out
}

/// Claude Code keeps only models whose id names Claude.
fn is_claude(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.contains("claude") || id.contains("anthropic")
}
