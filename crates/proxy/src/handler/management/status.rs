//! Management status endpoint — reports provider health and server info.

use crate::AppState;
use axum::{Json, extract::State};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

/// Top-level status response.
#[derive(Serialize, ToSchema)]
pub struct StatusResponse {
    pub server: ServerInfo,
    pub providers: Vec<ProviderStatus>,
}

/// Server listen address.
#[derive(Serialize, ToSchema)]
pub struct ServerInfo {
    pub host: String,
    pub port: u16,
}

/// Per-provider status summary.
#[derive(Serialize, ToSchema)]
pub struct ProviderStatus {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub auth_status: AuthStatus,
    pub models_count: usize,
}

/// Authentication state for a provider.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    Valid,
    Expired,
    NotConfigured,
}

/// Returns the current server and provider status.
#[utoipa::path(
    get,
    path = "/v0/management/status",
    responses((status = 200, body = StatusResponse)),
    tag = "management"
)]
pub async fn status_handler(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let snapshot = state.config.load();

    let server = ServerInfo {
        host: snapshot.host.clone(),
        port: snapshot.port,
    };

    let mut providers = Vec::new();

    for provider_id in byokey_types::ProviderId::all() {
        let config = snapshot.providers.get(provider_id);
        let enabled = config.is_none_or(|c| c.enabled);
        let has_api_key = config.is_some_and(|c| c.api_key.is_some() || !c.api_keys.is_empty());

        let auth_status = if has_api_key {
            // API key configured — always valid (no OAuth needed).
            AuthStatus::Valid
        } else if state.auth.is_authenticated(provider_id).await {
            AuthStatus::Valid
        } else {
            let accounts = state
                .auth
                .list_accounts(provider_id)
                .await
                .unwrap_or_default();
            if accounts.is_empty() {
                AuthStatus::NotConfigured
            } else {
                AuthStatus::Expired
            }
        };

        let models_count = byokey_provider::models_for_provider(provider_id).len();

        providers.push(ProviderStatus {
            id: provider_id.to_string(),
            display_name: provider_id.display_name().to_string(),
            enabled,
            auth_status,
            models_count,
        });
    }

    Json(StatusResponse { server, providers })
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
    use http_body_util::BodyExt as _;
    use serde_json::Value;
    use tower::ServiceExt as _;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store, rquest::Client::new()));
        let config = Arc::new(ArcSwap::from_pointee(Config::default()));
        AppState::with_thread_index(
            config,
            auth,
            None,
            byokey_provider::VersionStore::empty(),
            Arc::new(crate::AmpThreadIndex::empty()),
        )
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let app = make_router(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v0/management/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&bytes).unwrap();

        assert!(json["server"]["host"].is_string());
        assert!(json["server"]["port"].is_number());
        assert!(json["providers"].is_array());

        let providers = json["providers"].as_array().unwrap();
        assert!(!providers.is_empty());

        // Each provider has required fields.
        for p in providers {
            assert!(p["id"].is_string());
            assert!(p["display_name"].is_string());
            assert!(p["models_count"].is_number());
        }
    }
}
