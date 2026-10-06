//! `GET /v1/models`: the Anthropic models `/v1/messages` serves.
//!
//! Each model is listed once, under Anthropic's id, when the provider its
//! route names offers it (see [`super::catalog`]). Standard ids preserve
//! Claude Desktop's effort recognition; display names identify the routed
//! provider. `byokey claude desktop` uses these names as `labelOverride`.
//! Which provider serves a model is set with `byokey route`; a
//! `copilot/` or `cursor/` prefix still picks one per request, for models
//! this list leaves out.
//!
//! Anthropic clients (Claude Code, Claude Desktop) send `anthropic-version`
//! and get Anthropic's list shape; everyone else gets the `OpenAI` one.
//! Claude Desktop reads `supports_1m` and offers the `<id>[1m]` variant of
//! such models in its picker. By default, native 1M models keep only their
//! standard entry; `anthropic.catalog.merge_native_1m` controls this behavior.
//!
//! Pickers keep list order, so models are listed in lineup order (see
//! [`lineup`]), dated with their release.

pub(crate) mod lineup;

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use byokey_config::Config;
use byokey_types::{ByokError, ProviderId};
use serde::{Serialize, Serializer};
use std::sync::Arc;
use time::{Date, OffsetDateTime};

use super::catalog::Catalog;
use crate::{AppState, error::ApiError};

/// A listed model.
struct ModelEntry {
    id: String,
    provider: ProviderId,
    display_name: String,
    released: Released,
    /// Clients should offer an additional `<id>[1m]` context mode.
    supports_1m: bool,
}

/// When a model was released: midnight UTC of its release date, or the Unix
/// epoch when unknown, as Anthropic's API reports unknown dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Released(OffsetDateTime);

impl Released {
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
            display_name: m.display_name,
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
    display_name: String,
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
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let config = state.config.load();
    let catalog = Catalog::fetch(&state, &config).await;
    let anthropic = headers.contains_key("anthropic-version");
    let models = listed(&catalog, &config).map_err(|error| {
        let error = ApiError::new(error);
        if anthropic { error.anthropic() } else { error }
    })?;
    Ok(if anthropic {
        Json(AnthropicList::from(models)).into_response()
    } else {
        Json(OpenAiList::from(models)).into_response()
    })
}

/// Routed models in lineup order, with provider-scoped display names.
fn listed(catalog: &Catalog, config: &Config) -> Result<Vec<ModelEntry>, ByokError> {
    let format_name = config.anthropic.catalog.name_formatter()?;
    let mut served = catalog.routed(&config.anthropic.routes);
    lineup::sort(&mut served, |(m, _, _)| *m);
    served
        .into_iter()
        .map(|(model, provider, offer)| {
            let id = model.to_string();
            let provider_id = provider.to_string();
            let name = config
                .model_override(&provider_id, &id)
                .and_then(|patch| patch.name.clone())
                .unwrap_or_else(|| model.display_name());
            let label = config.provider_name(&provider_id);
            Ok(ModelEntry {
                id,
                provider,
                display_name: format_name(&name, label)?,
                released: Released::from(lineup::released(model)),
                supports_1m: offer.supports_1m,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::to_bytes, http::StatusCode, routing::get};
    use byokey_auth::AuthManager;
    use byokey_config::Config;
    use byokey_store::InMemoryTokenStore;
    use serde_json::{Value, json};

    fn entries() -> Vec<ModelEntry> {
        let model = |id: &str| -> byokey_types::ClaudeModel { id.parse().unwrap() };
        vec![
            ModelEntry {
                id: "claude-opus-5-5".into(),
                provider: ProviderId::Copilot,
                display_name: "Claude Opus 5.5 · GitHub Copilot".into(),
                released: Released::from(lineup::released(model("claude-opus-5-5"))),
                supports_1m: false,
            },
            ModelEntry {
                id: "claude-opus-4-1".into(),
                provider: ProviderId::Claude,
                display_name: "Claude Opus 4.1 · Claude (Anthropic)".into(),
                released: Released::from(lineup::released(model("claude-opus-4-1"))),
                supports_1m: false,
            },
        ]
    }

    #[test]
    fn both_list_shapes_carry_the_release_date() {
        let anthropic = serde_json::to_value(AnthropicList::from(entries())).unwrap();
        assert_eq!(
            anthropic,
            json!({
                "first_id": "claude-opus-5-5",
                "last_id": "claude-opus-4-1",
                "has_more": false,
                "data": [
                    {
                        "type": "model",
                        "id": "claude-opus-5-5",
                        "display_name": "Claude Opus 5.5 · GitHub Copilot",
                        "created_at": "2026-09-22T00:00:00Z",
                        "supports_1m": false,
                    },
                    {
                        "type": "model",
                        "id": "claude-opus-4-1",
                        "display_name": "Claude Opus 4.1 · Claude (Anthropic)",
                        "created_at": "1970-01-01T00:00:00Z",
                        "supports_1m": false,
                    },
                ],
            })
        );
        let openai = serde_json::to_value(OpenAiList::from(entries())).unwrap();
        let created: Vec<&Value> = openai["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| &m["created"])
            .collect();
        assert_eq!(created, [&json!(1_790_035_200), &json!(0)]);
        assert_eq!(openai["data"][0]["owned_by"], "copilot");
        assert_eq!(openai["data"][0]["id"], "claude-opus-5-5");
        assert_eq!(
            openai["data"][0]["display_name"],
            "Claude Opus 5.5 · GitHub Copilot"
        );
    }

    async fn state() -> (Config, Arc<AppState>) {
        let upstream = Router::new().route(
            "/models",
            get(|| async {
                Json(json!({"data": [{
                    "id": "claude-opus-5.5", "name": "Upstream spelling",
                    "model_picker_enabled": true, "supported_endpoints": ["/v1/messages"],
                    "capabilities": {"limits": {"max_context_window_tokens": 1_000_000}}
                }]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let config: Config = serde_json::from_value(json!({
            "anthropic": {"routes": {"default": "copilot"}},
            "providers": {
                "claude": {"api_key": "test-anthropic-key"},
                "copilot": {"api_key": uuid::Uuid::new_v4().to_string(), "base_url": url}
            }
        }))
        .unwrap();
        let http = reqwest::Client::new();
        let state = AppState::new(
            Arc::new(arc_swap::ArcSwap::from_pointee(config.clone())),
            Arc::new(AuthManager::new(
                Arc::new(InMemoryTokenStore::new()),
                http.clone(),
            )),
            http,
            None,
        )
        .unwrap();
        (config, state)
    }

    #[tokio::test]
    async fn route_changes_update_the_provider_label_without_changing_the_model_id() {
        let (mut config, state) = state().await;
        let list = || async {
            let response = list_models(State(state.clone()), HeaderMap::new())
                .await
                .unwrap();
            serde_json::from_slice::<Value>(
                &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            )
            .unwrap()
        };
        let before = list().await;

        config
            .anthropic
            .routes
            .models
            .insert("claude-opus-5-5".parse().unwrap(), ProviderId::Claude);
        state.config.store(Arc::new(config));
        let after = list().await;

        assert_eq!(
            before["data"],
            json!([{
                "id": "claude-opus-5-5", "object": "model", "created": 1_790_035_200,
                "owned_by": "copilot", "display_name": "Claude Opus 5.5 · Copilot"
            }])
        );
        assert_eq!(
            after["data"],
            json!([{
                "id": "claude-opus-5-5", "object": "model", "created": 1_790_035_200,
                "owned_by": "claude", "display_name": "Claude Opus 5.5 · Claude (Anthropic)"
            }])
        );
    }

    #[tokio::test]
    async fn native_context_merging_can_be_disabled_on_reload() {
        let (mut config, state) = state().await;
        let list = || async {
            let response = list_models(State(state.clone()), HeaderMap::new())
                .await
                .unwrap();
            serde_json::from_slice::<Value>(
                &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            )
            .unwrap()
        };
        let before = list().await;

        config.anthropic.catalog =
            Config::from_yaml("anthropic: { catalog: { merge_native_1m: false } }")
                .unwrap()
                .anthropic
                .catalog;
        state.config.store(Arc::new(config));
        let after = list().await;

        assert_eq!(before["data"][0]["id"], "claude-opus-5-5");
        assert!(before["data"][0].get("supports_1m").is_none());
        assert_eq!(
            after["data"],
            json!([{
                "id": "claude-opus-5-5", "object": "model", "created": 1_790_035_200,
                "owned_by": "copilot", "display_name": "Claude Opus 5.5 · Copilot",
                "supports_1m": true
            }])
        );
    }

    #[tokio::test]
    async fn templates_customize_names_without_changing_ids_or_context() {
        let (mut config, state) = state().await;
        let presentation = Config::from_yaml(
            r"
anthropic:
  catalog:
    name_format: '{{ provider | upper }} / {{ model }}'
providers:
  copilot:
    display_name: GitHub
    model_overrides:
      claude-opus-5-5:
        name: 'Opus <{{ provider }}>'
",
        )
        .unwrap();
        config.anthropic.catalog = presentation.anthropic.catalog;
        let copilot = config.providers.get_mut("copilot").unwrap();
        copilot
            .display_name
            .clone_from(&presentation.providers["copilot"].display_name);
        copilot
            .model_overrides
            .clone_from(&presentation.providers["copilot"].model_overrides);
        config.responses.catalog.name_format = "Responses only".into();
        config
            .anthropic
            .routes
            .models
            .insert("claude-haiku-4-5".parse().unwrap(), ProviderId::Claude);
        state.config.store(Arc::new(config));

        let headers = HeaderMap::from_iter([(
            "anthropic-version".parse().unwrap(),
            "2023-06-01".parse().unwrap(),
        )]);
        let response = list_models(State(state), headers).await.unwrap();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();

        assert_eq!(body["data"].as_array().unwrap().len(), 2);
        assert_eq!(body["data"][0]["id"], "claude-opus-5-5");
        assert_eq!(
            body["data"][0]["display_name"],
            "GITHUB / Opus <{{ provider }}>"
        );
        assert_eq!(body["data"][0]["supports_1m"], false);
        assert_eq!(body["data"][1]["id"], "claude-haiku-4-5");
        assert_eq!(
            body["data"][1]["display_name"],
            "CLAUDE (ANTHROPIC) / Claude Haiku 4.5"
        );
        assert_eq!(body["data"][1]["supports_1m"], false);
    }

    #[tokio::test]
    async fn a_template_that_renders_empty_for_a_real_model_fails_the_catalog() {
        let (mut config, state) = state().await;
        config.anthropic.catalog = Config::from_yaml(
            r#"
anthropic:
  catalog:
    name_format: '{% if model == "model" %}{{ model }}{% endif %}'
"#,
        )
        .unwrap()
        .anthropic
        .catalog;
        state.config.store(Arc::new(config));
        let headers = HeaderMap::from_iter([(
            "anthropic-version".parse().unwrap(),
            "2023-06-01".parse().unwrap(),
        )]);

        let anthropic = list_models(State(state.clone()), headers)
            .await
            .into_response();
        let openai = list_models(State(state), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(anthropic.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(openai.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let anthropic: Value =
            serde_json::from_slice(&to_bytes(anthropic.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let openai: Value =
            serde_json::from_slice(&to_bytes(openai.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(anthropic["type"], "error");
        assert_eq!(openai["error"]["code"], "internal_error");
        assert!(
            anthropic["error"]["message"]
                .as_str()
                .unwrap()
                .contains("anthropic.catalog.name_format must render a nonempty name")
        );
        assert_eq!(anthropic["error"]["message"], openai["error"]["message"]);
    }
}
