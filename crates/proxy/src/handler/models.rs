//! `GET /v1/models`: the models a client can actually use.
//!
//! One gateway serves two request formats, and a model id routes differently
//! on each: `/v1/chat/completions` resolves providers through the model
//! registry, while `/v1/messages` sends unprefixed ids to Anthropic (or the
//! `claude.backend` override) and only `copilot/` or `cursor/` ids elsewhere.
//! Anthropic clients (Claude Code, Claude Desktop) send `anthropic-version`;
//! they get the ids `/v1/messages` routes, in Anthropic's list shape. Every
//! other client gets the ids `/v1/chat/completions` routes.
//!
//! Copilot and Cursor are listed from their accounts' live catalogs; other
//! providers from the static registry, once signed in or given an API key.

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use byokey_config::Config;
use byokey_provider::{CopilotModel, CursorExecutor, all_models};
use byokey_types::ProviderId;
use serde::Serialize;
use serde_json::json;
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
    /// Label for model pickers, when the upstream names the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl ModelEntry {
    fn new(id: String, owned_by: &ProviderId, display_name: Option<String>) -> Self {
        Self {
            id,
            object: "model".into(),
            created: 0,
            owned_by: owned_by.to_string(),
            display_name,
        }
    }
}

/// Handles `GET /v1/models`.
///
/// Anthropic-format clients (with an `anthropic-version` header) get the
/// models `/v1/messages` can route; others get what `/v1/chat/completions`
/// can route. See the module documentation.
#[utoipa::path(
    get,
    path = "/v1/models",
    responses((status = 200, body = ModelsResponse)),
    tag = "management"
)]
pub async fn list_models(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let config = state.config.load();
    let live = Live::fetch(&state, &config).await;
    if headers.contains_key("anthropic-version") {
        let data = messages_models(&config, &live);
        let first = data.first().map(|m| m.id.clone());
        let last = data.last().map(|m| m.id.clone());
        let data: Vec<_> = data
            .into_iter()
            .map(|m| {
                json!({
                    "type": "model",
                    "display_name": m.display_name.as_ref().unwrap_or(&m.id),
                    "id": m.id,
                    "created_at": "1970-01-01T00:00:00Z",
                })
            })
            .collect();
        return Json(json!({"data": data, "has_more": false, "first_id": first, "last_id": last}))
            .into_response();
    }
    Json(ModelsResponse {
        object: "list".into(),
        data: chat_models(&config, &live),
    })
    .into_response()
}

/// Whether `provider` may be listed: enabled, and signed in or keyed.
async fn usable(state: &AppState, config: &Config, provider: &ProviderId) -> bool {
    let pc = config.providers.get(provider);
    if pc.is_some_and(|c| !c.enabled) {
        return false;
    }
    pc.is_some_and(|c| c.api_key.is_some() || !c.api_keys.is_empty())
        || state.auth.is_authenticated(provider).await
}

/// Live catalogs of the providers that publish one, plus which providers
/// may be listed at all.
struct Live {
    usable: Vec<ProviderId>,
    copilot: Vec<CopilotModel>,
    cursor: Vec<(String, String)>,
}

impl Live {
    async fn fetch(state: &AppState, config: &Config) -> Self {
        let mut usable = Vec::new();
        for provider in ProviderId::all() {
            if self::usable(state, config, provider).await {
                usable.push(provider.clone());
            }
        }
        let copilot = if usable.contains(&ProviderId::Copilot) {
            super::messages::copilot_executor(state)
                .0
                .models()
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "Copilot model listing failed"))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let cursor = if usable.contains(&ProviderId::Cursor) {
            let api_key = config
                .providers
                .get(&ProviderId::Cursor)
                .and_then(|c| c.api_key.clone());
            CursorExecutor::builder()
                .http(state.http.clone())
                .auth(state.auth.clone())
                .maybe_api_key(api_key)
                .build()
                .models()
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "Cursor model listing failed"))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            usable,
            copilot,
            cursor,
        }
    }

    fn has(&self, provider: &ProviderId) -> bool {
        self.usable.contains(provider)
    }

    /// Copilot models served on the given endpoint.
    fn copilot_on(&self, endpoint: fn(&CopilotModel) -> bool) -> Vec<(String, String)> {
        self.copilot
            .iter()
            .filter(|m| endpoint(m))
            .map(|m| (m.id.clone(), m.name.clone()))
            .collect()
    }
}

/// What `/v1/messages` routes. Unprefixed ids go to `claude.backend` when set
/// (Copilot or Cursor, whose catalog then stands in for Anthropic's), else to
/// Anthropic; `copilot/` and `cursor/` ids always reach their provider.
fn messages_models(config: &Config, live: &Live) -> Vec<ModelEntry> {
    let backend = config
        .providers
        .get(&ProviderId::Claude)
        .and_then(|c| c.backend.clone());
    let copilot = live.copilot_on(|m| m.messages);
    let mut out = Vec::new();
    match backend {
        Some(ProviderId::Copilot) => {
            push_all(&mut out, config, &ProviderId::Copilot, &copilot, false);
        }
        Some(ProviderId::Cursor) => {
            push_all(&mut out, config, &ProviderId::Cursor, &live.cursor, false);
        }
        _ if live.has(&ProviderId::Claude) => {
            for entry in all_models() {
                if entry.providers.first() == Some(&ProviderId::Claude)
                    && !config.is_model_excluded(&ProviderId::Claude, entry.id)
                {
                    out.push(ModelEntry::new(
                        entry.id.to_owned(),
                        &ProviderId::Claude,
                        None,
                    ));
                }
            }
        }
        _ => {}
    }
    // The backend's models are already listed unprefixed.
    if backend != Some(ProviderId::Copilot) {
        push_all(&mut out, config, &ProviderId::Copilot, &copilot, true);
    }
    if backend != Some(ProviderId::Cursor) {
        push_all(&mut out, config, &ProviderId::Cursor, &live.cursor, true);
    }
    out
}

/// What `/v1/chat/completions` routes: registry models of listed providers
/// (Copilot and Cursor from their live catalogs instead), unprefixed under
/// the provider the registry resolves them to, and live models qualified.
fn chat_models(config: &Config, live: &Live) -> Vec<ModelEntry> {
    let mut out = Vec::new();
    for entry in all_models() {
        let Some(primary) = entry.providers.first() else {
            continue;
        };
        if matches!(primary, ProviderId::Copilot | ProviderId::Cursor)
            || !live.has(primary)
            || config.is_model_excluded(primary, entry.id)
        {
            continue;
        }
        let alias = config
            .model_alias
            .get(primary)
            .and_then(|a| a.iter().find(|ae| ae.name == entry.id));
        match alias {
            Some(ae) => {
                out.push(ModelEntry::new(ae.alias.clone(), primary, None));
                if ae.fork {
                    out.push(ModelEntry::new(entry.id.to_owned(), primary, None));
                }
            }
            None => out.push(ModelEntry::new(entry.id.to_owned(), primary, None)),
        }
    }
    push_all(
        &mut out,
        config,
        &ProviderId::Copilot,
        &live.copilot_on(|m| m.chat),
        true,
    );
    push_all(&mut out, config, &ProviderId::Cursor, &live.cursor, true);
    out
}

/// Append `models` of `provider`, skipping excluded ids and ids already
/// listed. Qualified entries are listed as `provider/<id>` and named after
/// their provider too, since several providers serve the same models.
fn push_all(
    out: &mut Vec<ModelEntry>,
    config: &Config,
    provider: &ProviderId,
    models: &[(String, String)],
    qualified: bool,
) {
    for (id, name) in models {
        if config.is_model_excluded(provider, id) {
            continue;
        }
        let listed = if qualified {
            format!("{provider}/{id}")
        } else {
            id.clone()
        };
        if !out.iter().any(|e| e.id == listed) {
            let name = if qualified {
                format!("{name} ({})", provider.display_name())
            } else {
                name.clone()
            };
            out.push(ModelEntry::new(listed, provider, Some(name)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copilot(id: &str, messages: bool, chat: bool) -> CopilotModel {
        CopilotModel {
            id: id.into(),
            name: id.into(),
            messages,
            chat,
        }
    }

    fn live(usable: &[ProviderId]) -> Live {
        Live {
            usable: usable.to_vec(),
            copilot: vec![
                copilot("claude-opus-5.5", true, true),
                copilot("gpt-5.6-sol", false, false),
                copilot("gpt-5.4", false, true),
            ],
            cursor: vec![("claude-opus-5-5".into(), "Claude Opus 5.5".into())],
        }
    }

    fn ids(models: &[ModelEntry]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    fn backend(provider: ProviderId) -> Config {
        let mut config = Config::default();
        config.providers.insert(
            ProviderId::Claude,
            byokey_config::ProviderConfig {
                backend: Some(provider),
                ..Default::default()
            },
        );
        config
    }

    #[test]
    fn messages_list_follows_the_claude_backend() {
        let live = live(&[ProviderId::Claude, ProviderId::Copilot, ProviderId::Cursor]);
        let direct = messages_models(&Config::default(), &live);
        let direct = ids(&direct);
        assert!(
            direct.contains(&"claude-opus-5-5"),
            "Anthropic ids when not redirected"
        );
        assert!(direct.contains(&"copilot/claude-opus-5.5"));
        assert!(direct.contains(&"cursor/claude-opus-5-5"));

        let redirected = messages_models(&backend(ProviderId::Copilot), &live);
        assert_eq!(
            redirected[1].display_name.as_deref(),
            Some("Claude Opus 5.5 (Cursor)"),
            "qualified entries name their provider"
        );
        let redirected = ids(&redirected);
        assert_eq!(
            redirected[0], "claude-opus-5.5",
            "the backend's ids, unprefixed"
        );
        assert!(
            !redirected.contains(&"copilot/claude-opus-5.5"),
            "the backend's ids are listed once"
        );
        assert!(
            !redirected.contains(&"claude-fable-5-1"),
            "would reach Copilot"
        );
        assert!(
            !redirected.contains(&"copilot/gpt-5.6-sol"),
            "not on /v1/messages"
        );
    }

    #[test]
    fn chat_list_keeps_only_what_chat_completions_reaches() {
        let live = live(&[ProviderId::Copilot, ProviderId::Codex]);
        let chat = chat_models(&Config::default(), &live);
        let chat = ids(&chat);
        assert!(chat.contains(&"copilot/gpt-5.4"));
        assert!(
            !chat.contains(&"copilot/gpt-5.6-sol"),
            "Responses-only on Copilot"
        );
        assert!(
            !chat.contains(&"claude-fable-5-1"),
            "Claude is not signed in"
        );
        assert!(
            chat.iter().any(|id| id.starts_with("gpt-")),
            "Codex registry models"
        );
    }
}
