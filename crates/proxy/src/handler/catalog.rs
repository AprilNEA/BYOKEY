//! Which Anthropic models each provider serves, and which one serves each
//! model once [`Routes`] have picked.
//!
//! Copilot and Cursor publish live catalogs under their own spellings
//! (`claude-opus-5.5`); Anthropic's models come from the static registry.
//! Built-in offers use [`ClaudeModel`]. Custom Messages catalogs retain raw
//! Claude IDs, including variants that have no standard spelling.
//!
//! The live catalogs are fetched side by side. Claude Desktop gives up on
//! `/v1/models` after 10 s. Built-in catalogs that fail or miss
//! [`CATALOG_DEADLINE`] use the last fetch; late fetches refill that cache.
//! Custom discovery failures propagate rather than hide a provider.

use byokey_config::{Config, Routes};
use byokey_provider::{CopilotModel, CursorModel, CursorUpstream, all_models};
use byokey_types::{ClaudeFamily as Family, ClaudeModel, ProviderId};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::Instrument;

use super::custom_messages;
use crate::{ApiError, AppState};

/// Tokens of context from which a model counts as long-context.
const LONG_CONTEXT_TOKENS: u64 = 1_000_000;

/// How long a listing waits for a live catalog before it uses the last one.
const CATALOG_DEADLINE: Duration = Duration::from_secs(5);

/// The live catalogs last fetched. Process-wide because upstreams are built
/// per request.
static COPILOT_CATALOG: Mutex<Vec<CopilotModel>> = Mutex::new(Vec::new());
static CURSOR_CATALOG: Mutex<Vec<CursorModel>> = Mutex::new(Vec::new());

/// A model as one provider offers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Offer {
    /// The provider's name for the model, when it gives one.
    pub name: Option<String>,
    /// Clients should offer an additional `<id>[1m]` context mode.
    pub supports_1m: bool,
}

/// Enabled providers' Claude models. Built-ins also require a login or key;
/// custom providers use their configured Messages connection.
#[derive(Debug, Default)]
pub(crate) struct Catalog {
    builtin: BTreeMap<ProviderId, BTreeMap<ClaudeModel, Offer>>,
    pub(super) custom: BTreeMap<String, Vec<custom_messages::Model>>,
}

impl Catalog {
    pub(crate) async fn fetch(state: &Arc<AppState>, config: &Config) -> Result<Self, ApiError> {
        let mut usable = Vec::new();
        for provider in ProviderId::all() {
            if self::usable(state, config, provider).await {
                usable.push(provider);
            }
        }
        let copilot = async {
            if !usable.contains(&ProviderId::Copilot) {
                return None;
            }
            let upstream = super::copilot::copilot_upstream(state);
            Some(
                last_or(ProviderId::Copilot, &COPILOT_CATALOG, async move {
                    upstream?.models().await
                })
                .await,
            )
        };
        let cursor = async {
            if !usable.contains(&ProviderId::Cursor) {
                return None;
            }
            let api_key = config
                .providers
                .get("cursor")
                .and_then(|c| c.api_key.as_ref())
                .map(byokey_config::ConfigValue::resolve)
                .transpose();
            let http = state.http.clone();
            let auth = state.auth.clone();
            Some(
                last_or(ProviderId::Cursor, &CURSOR_CATALOG, async move {
                    let upstream = CursorUpstream::builder()
                        .http(http)
                        .auth(auth)
                        .maybe_api_key(api_key?)
                        .build();
                    upstream.models().await
                })
                .await,
            )
        };
        let custom = futures_util::future::try_join_all(config.providers.iter().filter_map(
            |(name, provider)| {
                let upstream = provider.anthropic.as_ref().filter(|_| provider.enabled)?;
                Some(async move {
                    custom_messages::models(&state.http, upstream)
                        .await
                        .map(|models| (name.clone(), models))
                })
            },
        ));
        let (copilot, cursor, custom) = tokio::join!(copilot, cursor, custom);
        let mut catalog = Self::new(
            usable.contains(&ProviderId::Claude),
            copilot,
            cursor,
            config.anthropic.catalog.merge_native_1m,
        );
        catalog.custom = custom?.into_iter().collect();
        Ok(catalog)
    }

    fn new(
        anthropic: bool,
        copilot: Option<Vec<CopilotModel>>,
        cursor: Option<Vec<CursorModel>>,
        merge_native_1m: bool,
    ) -> Self {
        let mut providers = BTreeMap::new();
        if anthropic {
            let offers = all_models()
                .iter()
                .filter_map(|m| ClaudeModel::from_id(m.id))
                .map(|m| {
                    let offer = Offer {
                        name: None,
                        supports_1m: false,
                    };
                    (m, offer)
                })
                .collect();
            providers.insert(ProviderId::Claude, offers);
        }
        if let Some(models) = copilot {
            let offers = models.into_iter().filter(|m| m.messages).filter_map(|m| {
                let model = ClaudeModel::from_id(&m.id)?;
                // These models default to 1M; an opt-in picker entry would duplicate them.
                // https://platform.claude.com/docs/en/build-with-claude/context-windows
                let native_1m = matches!(
                    (model.family, model.version),
                    (Family::Fable, (5, 0 | 1))
                        | (Family::Opus, (4, 6..=8) | (5, 0 | 5))
                        | (Family::Sonnet, (4, 6) | (5, 0 | 5))
                );
                let offer = Offer {
                    name: Some(m.name),
                    supports_1m: !(merge_native_1m && native_1m)
                        && m.context_window >= Some(LONG_CONTEXT_TOKENS),
                };
                Some((model, offer))
            });
            providers.insert(ProviderId::Copilot, first_spelling(offers));
        }
        if let Some(models) = cursor {
            // Cursor's catalog names its models but not their context windows.
            let offers = models.into_iter().filter_map(|m| {
                let offer = Offer {
                    name: Some(m.name),
                    supports_1m: false,
                };
                Some((ClaudeModel::from_id(&m.id)?, offer))
            });
            providers.insert(ProviderId::Cursor, first_spelling(offers));
        }
        Self {
            builtin: providers,
            custom: BTreeMap::new(),
        }
    }

    /// The models `provider` offers, or `None` when it may not be used.
    pub(crate) fn offers(&self, provider: ProviderId) -> Option<&BTreeMap<ClaudeModel, Offer>> {
        self.builtin.get(&provider)
    }

    /// Every model some usable provider offers, with who offers it.
    pub(crate) fn models(&self) -> BTreeMap<String, Vec<String>> {
        let mut models: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (&provider, offers) in &self.builtin {
            for &model in offers.keys() {
                models
                    .entry(model.to_string())
                    .or_default()
                    .push(provider.to_string());
            }
        }
        for (provider, offers) in &self.custom {
            for model in offers {
                models
                    .entry(model.id.clone())
                    .or_default()
                    .push(provider.clone());
            }
        }
        models
    }

    /// The models `/v1/messages` serves under `routes`: each model its
    /// provider offers, as that provider offers it.
    pub(crate) fn routed(&self, routes: &Routes) -> Vec<(ClaudeModel, ProviderId, &Offer)> {
        self.builtin
            .values()
            .flat_map(|models| models.keys().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|model| {
                let provider = routes.provider(model).parse().ok()?;
                Some((model, provider, self.offers(provider)?.get(&model)?))
            })
            .collect()
    }
}

/// Keep the first offer for each model, as the catalog lists it.
fn first_spelling(
    offers: impl Iterator<Item = (ClaudeModel, Offer)>,
) -> BTreeMap<ClaudeModel, Offer> {
    let mut map = BTreeMap::new();
    for (model, offer) in offers {
        map.entry(model).or_insert(offer);
    }
    map
}

/// Whether `provider` may be used: enabled, and signed in or keyed.
async fn usable(state: &AppState, config: &Config, provider: ProviderId) -> bool {
    let pc = config.providers.get(&provider.to_string());
    if pc.is_some_and(|c| !c.enabled) {
        return false;
    }
    pc.is_some_and(|c| c.api_key.is_some()) || state.auth.is_authenticated(provider).await
}

/// The catalog `fetch` returns, or the one `last` holds when `fetch` fails or
/// misses [`CATALOG_DEADLINE`]. A late `fetch` keeps running and refills
/// `last` when it lands.
async fn last_or<T: Clone + Send + 'static>(
    provider: ProviderId,
    last: &'static Mutex<Vec<T>>,
    fetch: impl Future<Output = byokey_types::Result<Vec<T>>> + Send + 'static,
) -> Vec<T> {
    let fetch = tokio::spawn(
        async move {
            let models = fetch
                .await
                .inspect_err(|e| tracing::warn!(%provider, error = %e, "model listing failed"))?;
            last.lock().expect("catalog lock").clone_from(&models);
            byokey_types::Result::Ok(models)
        }
        .in_current_span(),
    );
    let fetched = tokio::time::timeout(CATALOG_DEADLINE, fetch).await;
    match fetched.map(|joined| joined.expect("catalog fetch panicked")) {
        Ok(Ok(models)) => return models,
        // The fetch logged its failure, also when it lands late.
        Ok(Err(_)) => {}
        Err(_) => tracing::warn!(
            %provider,
            deadline_secs = CATALOG_DEADLINE.as_secs(),
            "model listing is late"
        ),
    }
    last.lock().expect("catalog lock").clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copilot(id: &str, messages: bool, context_window: Option<u64>) -> CopilotModel {
        CopilotModel {
            id: id.into(),
            name: id.into(),
            messages,
            responses: false,
            context_window,
        }
    }

    fn cursor(id: &str) -> CursorModel {
        CursorModel {
            id: id.into(),
            name: id.into(),
        }
    }

    fn model(id: &str) -> ClaudeModel {
        id.parse().unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::new(
            true,
            Some(vec![
                copilot("claude-opus-5.5", true, Some(LONG_CONTEXT_TOKENS)),
                copilot("claude-opus-4.8-fast", true, Some(LONG_CONTEXT_TOKENS)),
                copilot("claude-haiku-4.5", true, Some(200_000)),
                copilot("claude-sonnet-4.6", false, None),
                copilot("gpt-5.4", true, None),
            ]),
            Some(vec![
                cursor("claude-opus-5-5"),
                cursor("claude-sonnet-4-6"),
                cursor("composer-2.5"),
            ]),
            true,
        )
    }

    #[test]
    fn providers_offer_their_claude_models_by_model() {
        let catalog = catalog();
        let copilot = catalog.offers(ProviderId::Copilot).unwrap();
        assert_eq!(
            copilot.keys().map(ToString::to_string).collect::<Vec<_>>(),
            ["claude-opus-5-5", "claude-haiku-4-5"],
            "not variants, models off /v1/messages, or other vendors'"
        );
        assert!(!copilot[&model("claude-opus-5-5")].supports_1m);
        assert!(!copilot[&model("claude-haiku-4-5")].supports_1m);
        assert_eq!(
            catalog.models()["claude-opus-5-5"],
            ["claude", "copilot", "cursor"]
        );
        assert_eq!(
            Catalog::new(false, None, None, true).offers(ProviderId::Claude),
            None,
            "a provider that may not be used offers nothing"
        );
    }

    #[test]
    fn default_1m_models_do_not_offer_extra_context_variants() {
        let ids = [
            "claude-fable-5.1",
            "claude-fable-5",
            "claude-opus-5.5",
            "claude-opus-5",
            "claude-opus-4.8",
            "claude-opus-4.7",
            "claude-opus-4.6",
            "claude-sonnet-5.5",
            "claude-sonnet-5",
            "claude-sonnet-4.6",
        ];
        let models = ids.map(|id| copilot(id, true, Some(1_000_000))).into();

        let catalog = Catalog::new(false, Some(models), None, true);
        let offers = catalog.offers(ProviderId::Copilot).unwrap();

        assert_eq!(offers.len(), ids.len());
        assert!(offers.values().all(|offer| !offer.supports_1m));
    }

    #[test]
    fn other_models_offer_context_variants_only_when_the_upstream_supports_them() {
        let catalog = Catalog::new(
            false,
            Some(vec![
                copilot("claude-sonnet-4.5", true, Some(1_000_000)),
                copilot("claude-opus-4.5", true, Some(999_999)),
                copilot("claude-opus-6", true, Some(1_000_000)),
                copilot("claude-haiku-4.5", true, None),
            ]),
            None,
            true,
        );
        let offers = catalog.offers(ProviderId::Copilot).unwrap();

        assert!(offers[&model("claude-sonnet-4-5")].supports_1m);
        assert!(!offers[&model("claude-opus-4-5")].supports_1m);
        assert!(offers[&model("claude-opus-6")].supports_1m);
        assert!(!offers[&model("claude-haiku-4-5")].supports_1m);
    }

    #[test]
    fn disabling_native_1m_merging_still_respects_upstream_limits() {
        let catalog = Catalog::new(
            false,
            Some(vec![
                copilot("claude-opus-5.5", true, Some(1_000_000)),
                copilot("claude-fable-5.1", true, Some(200_000)),
                copilot("claude-sonnet-4.5", true, Some(1_000_000)),
                copilot("claude-haiku-4.5", true, None),
            ]),
            None,
            false,
        );
        let offers = catalog.offers(ProviderId::Copilot).unwrap();

        assert!(offers[&model("claude-opus-5-5")].supports_1m);
        assert!(!offers[&model("claude-fable-5-1")].supports_1m);
        assert!(offers[&model("claude-sonnet-4-5")].supports_1m);
        assert!(!offers[&model("claude-haiku-4-5")].supports_1m);
    }

    #[test]
    fn a_model_is_served_as_its_route_offers_it() {
        let catalog = catalog();
        let routes = Routes {
            default: Some("copilot".into()),
            models: [(model("claude-sonnet-4-6"), "cursor".into())].into(),
            ..Routes::default()
        };
        let served: Vec<(String, ProviderId)> = catalog
            .routed(&routes)
            .into_iter()
            .map(|(m, p, _)| (m.to_string(), p))
            .collect();
        assert_eq!(
            served,
            [
                ("claude-opus-5-5".into(), ProviderId::Copilot),
                ("claude-sonnet-4-6".into(), ProviderId::Cursor),
                ("claude-haiku-4-5".into(), ProviderId::Copilot),
            ],
            "Copilot does not serve Fable 5.1 or Sonnet 5, so they are not listed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_or_late_catalog_is_used_as_last_fetched() {
        static LAST: Mutex<Vec<&str>> = Mutex::new(Vec::new());
        let provider = ProviderId::Copilot;

        let listed = last_or(provider, &LAST, async { Ok(vec!["a"]) }).await;
        assert_eq!(listed, ["a"]);

        let failed = last_or(provider, &LAST, async {
            Err(byokey_types::ByokError::Http("unreachable".into()))
        })
        .await;
        assert_eq!(failed, ["a"], "a failed fetch uses the last catalog");

        let late = last_or(provider, &LAST, async {
            tokio::time::sleep(CATALOG_DEADLINE * 2).await;
            Ok(vec!["b"])
        })
        .await;
        assert_eq!(late, ["a"], "a late fetch uses the last catalog");
        tokio::time::sleep(CATALOG_DEADLINE * 2).await;
        assert_eq!(
            *LAST.lock().unwrap(),
            ["b"],
            "a late fetch refills the catalog when it lands"
        );
    }
}
