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
use byokey_provider::{CopilotModel, CursorModel, CursorUpstream, all_models};
use byokey_types::ProviderId;
use serde::{Serialize, Serializer};
use std::sync::Arc;
use time::{Date, OffsetDateTime};

use crate::AppState;

/// Tokens of context from which a model counts as long-context.
const LONG_CONTEXT_TOKENS: u64 = 1_000_000;

/// A listed model.
struct ModelEntry {
    id: String,
    provider: ProviderId,
    /// Label for model pickers, when the upstream names the model.
    display_name: Option<String>,
    released: Released,
    /// The model takes a 1M-token context, selected as `<id>[1m]`.
    supports_1m: bool,
}

impl ModelEntry {
    fn new(id: String, provider: ProviderId) -> Self {
        Self {
            released: Released::of(&id),
            id,
            provider,
            display_name: None,
            supports_1m: false,
        }
    }
}

/// When a model was released: midnight UTC of its release date, or the Unix
/// epoch when unknown, as Anthropic's API reports unknown dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Released(OffsetDateTime);

impl Released {
    fn of(id: &str) -> Self {
        Self::from(lineup::released_on(id))
    }

    /// As Unix seconds, `OpenAI`'s `created`.
    fn serialize_unix<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        time::serde::timestamp::serialize(&self.0, s)
    }

    /// As RFC 3339, Anthropic's `created_at`.
    fn serialize_rfc3339<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        time::serde::rfc3339::serialize(&self.0, s)
    }
}

impl From<Option<Date>> for Released {
    fn from(date: Option<Date>) -> Self {
        Self(date.map_or(OffsetDateTime::UNIX_EPOCH, |d| d.midnight().assume_utc()))
    }
}

/// An entry of Anthropic's model list.
#[derive(Serialize)]
struct AnthropicModel {
    #[serde(rename = "type")]
    kind: &'static str,
    id: String,
    display_name: String,
    #[serde(serialize_with = "Released::serialize_rfc3339")]
    created_at: Released,
    supports_1m: bool,
}

impl From<ModelEntry> for AnthropicModel {
    fn from(m: ModelEntry) -> Self {
        Self {
            kind: "model",
            display_name: m.display_name.unwrap_or_else(|| m.id.clone()),
            id: m.id,
            created_at: m.released,
            supports_1m: m.supports_1m,
        }
    }
}

/// Anthropic's model list: one page holding every model.
#[derive(Serialize)]
struct AnthropicList {
    first_id: Option<String>,
    last_id: Option<String>,
    has_more: bool,
    data: Vec<AnthropicModel>,
}

impl From<Vec<ModelEntry>> for AnthropicList {
    fn from(models: Vec<ModelEntry>) -> Self {
        let data: Vec<AnthropicModel> = models.into_iter().map(AnthropicModel::from).collect();
        Self {
            first_id: data.first().map(|m| m.id.clone()),
            last_id: data.last().map(|m| m.id.clone()),
            has_more: false,
            data,
        }
    }
}

/// An entry of the `OpenAI` model list.
#[derive(Serialize)]
struct OpenAiModel {
    id: String,
    object: &'static str,
    #[serde(serialize_with = "Released::serialize_unix")]
    created: Released,
    owned_by: ProviderId,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    supports_1m: bool,
}

impl From<ModelEntry> for OpenAiModel {
    fn from(m: ModelEntry) -> Self {
        Self {
            id: m.id,
            object: "model",
            created: m.released,
            owned_by: m.provider,
            display_name: m.display_name,
            supports_1m: m.supports_1m,
        }
    }
}

/// The `OpenAI` model list.
#[derive(Serialize)]
struct OpenAiList {
    object: &'static str,
    data: Vec<OpenAiModel>,
}

impl From<Vec<ModelEntry>> for OpenAiList {
    fn from(models: Vec<ModelEntry>) -> Self {
        Self {
            object: "list",
            data: models.into_iter().map(OpenAiModel::from).collect(),
        }
    }
}

/// Handles `GET /v1/models`. See the module documentation.
pub async fn list_models(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let config = state.config.load();
    let live = Live::fetch(&state, &config).await;
    let models = messages_models(&config, &live);
    if headers.contains_key("anthropic-version") {
        Json(AnthropicList::from(models)).into_response()
    } else {
        Json(OpenAiList::from(models)).into_response()
    }
}

/// Whether `provider` may be listed: enabled, and signed in or keyed.
async fn usable(state: &AppState, config: &Config, provider: ProviderId) -> bool {
    let pc = config.providers.get(&provider);
    if pc.is_some_and(|c| !c.enabled) {
        return false;
    }
    pc.is_some_and(|c| c.api_key.is_some()) || state.auth.is_authenticated(provider).await
}

/// A model from a provider's live catalog.
struct LiveModel {
    id: String,
    name: String,
    supports_1m: bool,
}

impl From<CopilotModel> for LiveModel {
    fn from(m: CopilotModel) -> Self {
        Self {
            supports_1m: m.context_window >= Some(LONG_CONTEXT_TOKENS),
            id: m.id,
            name: m.name,
        }
    }
}

impl From<CursorModel> for LiveModel {
    /// Cursor's catalog names its models but not their context windows.
    fn from(m: CursorModel) -> Self {
        Self {
            id: m.id,
            name: m.name,
            supports_1m: false,
        }
    }
}

/// The live catalogs of the providers that publish one, as far as
/// `/v1/messages` reaches them, plus which providers may be listed at all.
struct Live {
    usable: Vec<ProviderId>,
    /// Copilot models served on its Anthropic-format `/v1/messages`.
    copilot: Vec<LiveModel>,
    cursor: Vec<LiveModel>,
}

impl Live {
    async fn fetch(state: &AppState, config: &Config) -> Self {
        let mut usable = Vec::new();
        for provider in ProviderId::all() {
            if self::usable(state, config, provider).await {
                usable.push(provider);
            }
        }
        let copilot = if usable.contains(&ProviderId::Copilot) {
            super::copilot_messages::copilot_upstream(state)
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
        } else {
            Vec::new()
        };
        Self::new(usable, copilot, cursor)
    }

    fn new(usable: Vec<ProviderId>, copilot: Vec<CopilotModel>, cursor: Vec<CursorModel>) -> Self {
        Self {
            usable,
            copilot: copilot
                .into_iter()
                .filter(|m| m.messages)
                .map(LiveModel::from)
                .collect(),
            cursor: cursor.into_iter().map(LiveModel::from).collect(),
        }
    }

    fn has(&self, provider: ProviderId) -> bool {
        self.usable.contains(&provider)
    }
}

/// What `/v1/messages` routes. Unprefixed ids go to `claude.backend` when set
/// (Copilot or Cursor, whose catalog then stands in for Anthropic's), else to
/// Anthropic; `copilot/` and `cursor/` ids always reach their provider.
fn messages_models(config: &Config, live: &Live) -> Vec<ModelEntry> {
    let backend = config
        .providers
        .get(&ProviderId::Claude)
        .and_then(|c| c.backend);
    let mut out = Vec::new();
    match backend {
        Some(ProviderId::Copilot) => push_all(&mut out, ProviderId::Copilot, &live.copilot, false),
        Some(ProviderId::Cursor) => push_all(&mut out, ProviderId::Cursor, &live.cursor, false),
        _ if live.has(ProviderId::Claude) => {
            out.extend(
                all_models()
                    .iter()
                    .map(|entry| ModelEntry::new(entry.id.to_owned(), ProviderId::Claude)),
            );
            lineup::sort(&mut out, |e| &e.id);
        }
        _ => {}
    }
    // The backend's models are already listed unprefixed.
    if backend != Some(ProviderId::Copilot) {
        push_all(&mut out, ProviderId::Copilot, &live.copilot, true);
    }
    if backend != Some(ProviderId::Cursor) {
        push_all(&mut out, ProviderId::Cursor, &live.cursor, true);
    }
    out
}

/// Append `models` of `provider` in lineup order, skipping ids already
/// listed. Qualified entries are listed as `provider/<id>` and named after
/// their provider too, since several providers serve the same models.
fn push_all(
    out: &mut Vec<ModelEntry>,
    provider: ProviderId,
    models: &[LiveModel],
    qualified: bool,
) {
    let start = out.len();
    for m in models {
        let (id, name) = if qualified {
            (
                format!("{provider}/{}", m.id),
                format!("{} ({})", m.name, provider.display_name()),
            )
        } else {
            (m.id.clone(), m.name.clone())
        };
        if !out.iter().any(|e| e.id == id) {
            out.push(ModelEntry {
                display_name: Some(name),
                supports_1m: m.supports_1m,
                ..ModelEntry::new(id, provider)
            });
        }
    }
    lineup::sort(&mut out[start..], |e| &e.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn copilot(id: &str, messages: bool, context_window: Option<u64>) -> CopilotModel {
        CopilotModel {
            id: id.into(),
            name: id.into(),
            messages,
            context_window,
        }
    }

    fn cursor(id: &str, name: &str) -> CursorModel {
        CursorModel {
            id: id.into(),
            name: name.into(),
        }
    }

    fn live(usable: &[ProviderId]) -> Live {
        Live::new(
            usable.to_vec(),
            vec![
                copilot("claude-opus-5.5", true, Some(LONG_CONTEXT_TOKENS)),
                copilot("claude-haiku-4.5", true, Some(200_000)),
                copilot("gpt-5.4", false, None),
            ],
            vec![cursor("claude-opus-5-5", "Claude Opus 5.5")],
        )
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
            "Copilot models off /v1/messages are not listed"
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
        let live = Live::new(Vec::new(), Vec::new(), Vec::new());
        assert!(messages_models(&Config::default(), &live).is_empty());
    }

    #[test]
    fn each_provider_is_listed_in_lineup_order() {
        let live = Live::new(
            vec![ProviderId::Copilot, ProviderId::Cursor],
            // Copilot's own catalog order.
            ["claude-opus-4.7", "claude-haiku-4.5", "claude-opus-5.5"]
                .into_iter()
                .map(|id| copilot(id, true, None))
                .collect(),
            vec![
                cursor("claude-sonnet-4-6", "Claude Sonnet 4.6"),
                cursor("claude-fable-5-1", "Claude Fable 5.1"),
            ],
        );
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
    }

    #[test]
    fn both_list_shapes_carry_the_release_date() {
        let models = || {
            vec![
                ModelEntry::new("claude-opus-5-5".into(), ProviderId::Claude),
                ModelEntry {
                    display_name: Some("Composer 2.5 (Cursor)".into()),
                    ..ModelEntry::new("cursor/composer-2.5".into(), ProviderId::Cursor)
                },
            ]
        };
        let anthropic = serde_json::to_value(AnthropicList::from(models())).unwrap();
        assert_eq!(
            anthropic,
            json!({
                "first_id": "claude-opus-5-5",
                "last_id": "cursor/composer-2.5",
                "has_more": false,
                "data": [
                    {
                        "type": "model",
                        "id": "claude-opus-5-5",
                        "display_name": "claude-opus-5-5",
                        "created_at": "2026-09-22T00:00:00Z",
                        "supports_1m": false,
                    },
                    {
                        "type": "model",
                        "id": "cursor/composer-2.5",
                        "display_name": "Composer 2.5 (Cursor)",
                        "created_at": "1970-01-01T00:00:00Z",
                        "supports_1m": false,
                    },
                ],
            })
        );
        let openai = serde_json::to_value(OpenAiList::from(models())).unwrap();
        let created: Vec<&Value> = openai["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| &m["created"])
            .collect();
        assert_eq!(created, [&json!(1_790_035_200), &json!(0)]);
        assert_eq!(openai["data"][0]["owned_by"], "claude");
    }
}
