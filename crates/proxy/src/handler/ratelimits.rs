//! Rate limit management endpoint — exposes captured upstream rate limit headers.

use crate::AppState;
use axum::{Json, extract::State};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

/// Top-level rate limits response.
#[derive(Serialize, ToSchema)]
pub struct RateLimitsResponse {
    pub providers: Vec<ProviderRateLimits>,
}

/// Rate limit data for a single provider.
#[derive(Serialize, ToSchema)]
pub struct ProviderRateLimits {
    pub id: String,
    pub display_name: String,
    pub accounts: Vec<AccountRateLimit>,
}

/// Rate limit snapshot for a single provider account.
#[derive(Serialize, ToSchema)]
pub struct AccountRateLimit {
    pub account_id: String,
    pub snapshot: byokey_types::RateLimitSnapshot,
}

/// Returns the latest captured rate limit headers for all providers.
#[utoipa::path(
    get,
    path = "/v0/management/ratelimits",
    responses((status = 200, body = RateLimitsResponse)),
    tag = "management"
)]
pub async fn ratelimits_handler(State(state): State<Arc<AppState>>) -> Json<RateLimitsResponse> {
    let all = state.ratelimits.all();

    let mut by_provider: std::collections::HashMap<
        byokey_types::ProviderId,
        Vec<AccountRateLimit>,
    > = std::collections::HashMap::new();

    for ((provider, account_id), snapshot) in all {
        by_provider
            .entry(provider)
            .or_default()
            .push(AccountRateLimit {
                account_id,
                snapshot,
            });
    }

    let mut providers = Vec::new();
    for provider_id in byokey_types::ProviderId::all() {
        let accounts = by_provider.remove(provider_id).unwrap_or_default();
        if accounts.is_empty() {
            continue;
        }
        providers.push(ProviderRateLimits {
            id: provider_id.to_string(),
            display_name: provider_id.display_name().to_string(),
            accounts,
        });
    }

    Json(RateLimitsResponse { providers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::make_router;
    use arc_swap::ArcSwap;
    use axum::{body::Body, http::Request};
    use byokey_auth::AuthManager;
    use byokey_config::Config;
    use byokey_store::InMemoryTokenStore;
    use byokey_types::{ProviderId, RateLimitSnapshot};
    use http_body_util::BodyExt as _;
    use serde_json::Value;
    use tower::ServiceExt as _;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store, rquest::Client::new()));
        let config = Arc::new(ArcSwap::from_pointee(Config::default()));
        AppState::new(config, auth)
    }

    #[tokio::test]
    async fn test_ratelimits_empty() {
        let app = make_router(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v0/management/ratelimits")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json["providers"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_ratelimits_with_data() {
        let state = make_state();
        // Insert a snapshot directly
        state.ratelimits.update(
            ProviderId::Claude,
            "active".into(),
            RateLimitSnapshot {
                headers: std::collections::HashMap::from([(
                    "anthropic-ratelimit-requests-remaining".into(),
                    "950".into(),
                )]),
                captured_at: 1_700_000_000,
            },
        );

        let app = make_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v0/management/ratelimits")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&bytes).unwrap();

        let providers = json["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["id"], "claude");
        let accounts = providers[0]["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0]["account_id"], "active");
        assert_eq!(
            accounts[0]["snapshot"]["headers"]["anthropic-ratelimit-requests-remaining"],
            "950"
        );
    }
}
