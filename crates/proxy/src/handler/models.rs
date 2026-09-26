//! `GET /v1/models`: the models `/v1/messages` can route.
//!
//! Unprefixed ids go to Anthropic, or to `claude.backend` when it is set,
//! whose catalog then stands in for Anthropic's; `copilot/` and `cursor/`
//! ids always reach their provider. Copilot and Cursor are listed from
//! their accounts' live catalogs, Anthropic from the static registry, each
//! once signed in or given an API key.
//!
//! Anthropic clients (Claude Code, Claude Desktop) send `anthropic-version`
//! and get Anthropic's list shape; everyone else gets the `OpenAI` one.
//! Claude Desktop reads `supports_1m` and offers the `<id>[1m]` variant of
//! such models in its picker.
//!
//! Pickers keep list order, so each provider's models are listed in lineup
//! order (see [`lineup`]), dated with their release.

mod lineup;

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use byokey_config::Config;
use byokey_provider::{CopilotModel, CursorUpstream, all_models};
use byokey_types::ProviderId;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

use crate::AppState;

/// A listed model.
#[derive(Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub object: String,
    /// Release time, as Unix seconds (0 when unknown).
    pub created: i64,
    pub owned_by: String,
    /// Label for model pickers, when the upstream names the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The model takes a 1M-token context, selected as `<id>[1m]`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub supports_1m: bool,
    #[serde(skip)]
    released: Option<time::Date>,
}

impl ModelEntry {
    fn new(id: String, owned_by: &ProviderId, display_name: Option<String>) -> Self {
        let released = lineup::released_on(&id);
        Self {
            id,
            object: "model".into(),
            created: released.map_or(0, |d| d.midnight().assume_utc().unix_timestamp()),
            owned_by: owned_by.to_string(),
            display_name,
            supports_1m: false,
            released,
        }
    }

    /// Anthropic's RFC 3339 `created_at`: the release at midnight UTC, or
    /// the Unix epoch when unknown, as Anthropic's API does.
    fn created_at(&self) -> String {
        match self.released {
            Some(d) => format!("{d}T00:00:00Z"),
            None => "1970-01-01T00:00:00Z".to_owned(),
        }
    }
}

/// Tokens of context from which a model counts as long-context.
const LONG_CONTEXT_TOKENS: u64 = 1_000_000;

/// A model from a provider's live catalog.
struct LiveModel {
    id: String,
    name: String,
    supports_1m: bool,
}

impl From<&CopilotModel> for LiveModel {
    fn from(m: &CopilotModel) -> Self {
        Self {
            id: m.id.clone(),
            name: m.name.clone(),
            supports_1m: m.context_window >= Some(LONG_CONTEXT_TOKENS),
        }
    }
}

impl From<(String, String)> for LiveModel {
    /// Cursor's catalog names its models but not their context windows.
    fn from((id, name): (String, String)) -> Self {
        Self {
            id,
            name,
            supports_1m: false,
        }
    }
}

/// Handles `GET /v1/models`. See the module documentation.
pub async fn list_models(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let config = state.config.load();
    let live = Live::fetch(&state, &config).await;
    let data = messages_models(&config, &live);
    if headers.contains_key("anthropic-version") {
        let first = data.first().map(|m| m.id.clone());
        let last = data.last().map(|m| m.id.clone());
        let data: Vec<_> = data
            .into_iter()
            .map(|m| {
                json!({
                    "type": "model",
                    "display_name": m.display_name.as_ref().unwrap_or(&m.id),
                    "created_at": m.created_at(),
                    "id": m.id,
                    "supports_1m": m.supports_1m,
                })
            })
            .collect();
        return Json(json!({"data": data, "has_more": false, "first_id": first, "last_id": last}))
            .into_response();
    }
    Json(json!({"object": "list", "data": data})).into_response()
}

/// Whether `provider` may be listed: enabled, and signed in or keyed.
async fn usable(state: &AppState, config: &Config, provider: &ProviderId) -> bool {
    let pc = config.providers.get(provider);
    if pc.is_some_and(|c| !c.enabled) {
        return false;
    }
    pc.is_some_and(|c| c.api_key.is_some()) || state.auth.is_authenticated(provider).await
}

/// Live catalogs of the providers that publish one, plus which providers
/// may be listed at all.
struct Live {
    usable: Vec<ProviderId>,
    copilot: Vec<CopilotModel>,
    cursor: Vec<LiveModel>,
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
            super::messages::copilot_upstream(state)
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
            CursorUpstream::builder()
                .http(state.http.clone())
                .auth(state.auth.clone())
                .maybe_api_key(api_key)
                .build()
                .models()
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "Cursor model listing failed"))
                .unwrap_or_default()
                .into_iter()
                .map(LiveModel::from)
                .collect()
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

    /// Copilot models served on its Anthropic-format `/v1/messages`.
    fn copilot_messages(&self) -> Vec<LiveModel> {
        self.copilot
            .iter()
            .filter(|m| m.messages)
            .map(LiveModel::from)
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
    let copilot = live.copilot_messages();
    let mut out = Vec::new();
    match backend {
        Some(ProviderId::Copilot) => push_all(&mut out, &ProviderId::Copilot, &copilot, false),
        Some(ProviderId::Cursor) => push_all(&mut out, &ProviderId::Cursor, &live.cursor, false),
        _ if live.has(&ProviderId::Claude) => {
            for entry in all_models() {
                out.push(ModelEntry::new(
                    entry.id.to_owned(),
                    &ProviderId::Claude,
                    None,
                ));
            }
            lineup::sort(&mut out, |e| &e.id);
        }
        _ => {}
    }
    // The backend's models are already listed unprefixed.
    if backend != Some(ProviderId::Copilot) {
        push_all(&mut out, &ProviderId::Copilot, &copilot, true);
    }
    if backend != Some(ProviderId::Cursor) {
        push_all(&mut out, &ProviderId::Cursor, &live.cursor, true);
    }
    out
}

/// Append `models` of `provider` in lineup order, skipping ids already
/// listed. Qualified entries are listed as `provider/<id>` and named after
/// their provider too, since several providers serve the same models.
fn push_all(
    out: &mut Vec<ModelEntry>,
    provider: &ProviderId,
    models: &[LiveModel],
    qualified: bool,
) {
    let start = out.len();
    for m in models {
        let listed = if qualified {
            format!("{provider}/{}", m.id)
        } else {
            m.id.clone()
        };
        if !out.iter().any(|e| e.id == listed) {
            let name = if qualified {
                format!("{} ({})", m.name, provider.display_name())
            } else {
                m.name.clone()
            };
            out.push(ModelEntry {
                supports_1m: m.supports_1m,
                ..ModelEntry::new(listed, provider, Some(name))
            });
        }
    }
    lineup::sort(&mut out[start..], |e| &e.id);
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
            context_window: None,
        }
    }

    fn live(usable: &[ProviderId]) -> Live {
        Live {
            usable: usable.to_vec(),
            copilot: vec![
                CopilotModel {
                    context_window: Some(LONG_CONTEXT_TOKENS),
                    ..copilot("claude-opus-5.5", true, true)
                },
                CopilotModel {
                    context_window: Some(200_000),
                    ..copilot("claude-haiku-4.5", true, true)
                },
                copilot("gpt-5.6-sol", false, false),
                copilot("gpt-5.4", false, true),
            ],
            cursor: vec![LiveModel::from((
                "claude-opus-5-5".to_owned(),
                "Claude Opus 5.5".to_owned(),
            ))],
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
    fn the_list_follows_the_claude_backend() {
        let live = live(&[ProviderId::Claude, ProviderId::Copilot, ProviderId::Cursor]);
        let direct = messages_models(&Config::default(), &live);
        let direct = ids(&direct);
        assert!(
            direct.contains(&"claude-opus-5-5"),
            "Anthropic ids when not redirected"
        );
        assert!(direct.contains(&"copilot/claude-opus-5.5"));
        assert!(direct.contains(&"cursor/claude-opus-5-5"));
        assert!(
            !direct.contains(&"copilot/gpt-5.4"),
            "chat-only Copilot models are not on /v1/messages"
        );

        let redirected = messages_models(&backend(ProviderId::Copilot), &live);
        assert_eq!(
            redirected[2].display_name.as_deref(),
            Some("Claude Opus 5.5 (Cursor)"),
            "qualified entries name their provider"
        );
        assert!(
            redirected[0].supports_1m,
            "a 1M-token catalog window is announced"
        );
        assert!(!redirected[1].supports_1m, "a 200k window is not");
        assert!(!redirected[2].supports_1m, "Cursor's window is unknown");
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
    }

    #[test]
    fn nothing_is_listed_without_a_login() {
        // `Live::fetch` loads no catalog for a provider that is not usable.
        let live = Live {
            usable: Vec::new(),
            copilot: Vec::new(),
            cursor: Vec::new(),
        };
        assert!(messages_models(&Config::default(), &live).is_empty());
    }

    #[test]
    fn each_provider_is_listed_in_lineup_order_with_release_dates() {
        let cursor = |id: &str| LiveModel::from((id.to_owned(), id.to_owned()));
        let live = Live {
            usable: vec![ProviderId::Copilot, ProviderId::Cursor],
            // Copilot's own catalog order.
            copilot: ["claude-opus-4.7", "claude-haiku-4.5", "claude-opus-5.5"]
                .into_iter()
                .map(|id| copilot(id, true, true))
                .collect(),
            cursor: vec![cursor("claude-sonnet-4-6"), cursor("claude-fable-5-1")],
        };
        let listed = messages_models(&backend(ProviderId::Copilot), &live);
        assert_eq!(
            ids(&listed),
            [
                "claude-opus-5.5",
                "claude-haiku-4.5",
                "claude-opus-4.7",
                "cursor/claude-fable-5-1",
                "cursor/claude-sonnet-4-6",
            ],
            "sorted within each provider, providers kept apart"
        );
        assert_eq!(listed[0].created_at(), "2026-09-22T00:00:00Z");
        assert_eq!(listed[0].created, 1_790_035_200);
    }

    #[test]
    fn unknown_release_dates_fall_back_to_the_epoch() {
        let entry = ModelEntry::new("composer-2.5".into(), &ProviderId::Cursor, None);
        assert_eq!(entry.created, 0);
        assert_eq!(entry.created_at(), "1970-01-01T00:00:00Z");
    }
}
