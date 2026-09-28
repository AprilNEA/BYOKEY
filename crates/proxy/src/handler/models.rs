//! `GET /v1/models`: the Anthropic models `/v1/messages` serves.
//!
//! Each model is listed once, under Anthropic's id, when the provider its
//! route names offers it (see [`super::catalog`]): Claude Desktop recognises
//! only those ids, and reads a model's effort levels and description from
//! them. Which provider serves a model is set with `byokey route`; a
//! `copilot/` or `cursor/` prefix still picks one per request, for models
//! this list leaves out.
//!
//! Anthropic clients (Claude Code, Claude Desktop) send `anthropic-version`
//! and get Anthropic's list shape; everyone else gets the `OpenAI` one.
//! Claude Desktop reads `supports_1m` and offers the `<id>[1m]` variant of
//! such models in its picker.
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
use byokey_config::Routes;
use byokey_types::ProviderId;
use serde::{Serialize, Serializer};
use std::sync::Arc;
use time::{Date, OffsetDateTime};

use super::catalog::Catalog;
use crate::AppState;

/// A listed model.
struct ModelEntry {
    id: String,
    provider: ProviderId,
    display_name: String,
    released: Released,
    /// The model takes a 1M-token context, selected as `<id>[1m]`.
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
pub async fn list_models(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let config = state.config.load();
    let catalog = Catalog::fetch(&state, &config).await;
    let models = listed(&catalog, &config.routes);
    if headers.contains_key("anthropic-version") {
        Json(AnthropicList::from(models)).into_response()
    } else {
        Json(OpenAiList::from(models)).into_response()
    }
}

/// The models `routes` serve, in lineup order, named as Anthropic names them.
fn listed(catalog: &Catalog, routes: &Routes) -> Vec<ModelEntry> {
    let mut served = catalog.routed(routes);
    lineup::sort(&mut served, |(m, _, _)| *m);
    served
        .into_iter()
        .map(|(model, provider, offer)| ModelEntry {
            id: model.to_string(),
            provider,
            display_name: model.display_name(),
            released: Released::from(lineup::released(model)),
            supports_1m: offer.supports_1m,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn entries() -> Vec<ModelEntry> {
        let model = |id: &str| -> byokey_types::ClaudeModel { id.parse().unwrap() };
        vec![
            ModelEntry {
                id: "claude-opus-5-5".into(),
                provider: ProviderId::Copilot,
                display_name: model("claude-opus-5-5").display_name(),
                released: Released::from(lineup::released(model("claude-opus-5-5"))),
                supports_1m: true,
            },
            ModelEntry {
                id: "claude-opus-4-1".into(),
                provider: ProviderId::Claude,
                display_name: model("claude-opus-4-1").display_name(),
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
                        "display_name": "Claude Opus 5.5",
                        "created_at": "2026-09-22T00:00:00Z",
                        "supports_1m": true,
                    },
                    {
                        "type": "model",
                        "id": "claude-opus-4-1",
                        "display_name": "Claude Opus 4.1",
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
    }
}
