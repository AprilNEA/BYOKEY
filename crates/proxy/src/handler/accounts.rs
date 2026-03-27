//! Account management endpoints — list, remove, and activate per-provider accounts.

use crate::{ApiError, AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

/// All accounts grouped by provider.
#[derive(Serialize, ToSchema)]
pub struct AccountsResponse {
    pub providers: Vec<ProviderAccounts>,
}

/// Accounts for a single provider.
#[derive(Serialize, ToSchema)]
pub struct ProviderAccounts {
    pub id: String,
    pub display_name: String,
    pub accounts: Vec<AccountDetail>,
}

/// Details for a single stored account.
#[derive(Serialize, ToSchema)]
pub struct AccountDetail {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub is_active: bool,
    pub token_state: TokenStateDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

/// Token validity state.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenStateDto {
    Valid,
    Expired,
    Invalid,
}

/// Lists all accounts for every provider.
///
/// # Errors
///
/// Returns `ApiError` if the underlying store query fails.
#[utoipa::path(
    get,
    path = "/v0/management/accounts",
    responses((status = 200, body = AccountsResponse)),
    tag = "management"
)]
pub async fn accounts_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AccountsResponse>, ApiError> {
    let mut providers = Vec::new();

    for provider_id in byokey_types::ProviderId::all() {
        let accounts_info = state
            .auth
            .list_accounts(provider_id)
            .await
            .unwrap_or_default();
        let all_tokens = state
            .auth
            .get_all_tokens(provider_id)
            .await
            .unwrap_or_default();

        let mut accounts = Vec::new();
        for info in &accounts_info {
            let (token_state, expires_at) =
                match all_tokens.iter().find(|(id, _)| id == &info.account_id) {
                    Some((_, token)) => {
                        let ts = match token.state() {
                            byokey_types::TokenState::Valid => TokenStateDto::Valid,
                            byokey_types::TokenState::Expired => TokenStateDto::Expired,
                            byokey_types::TokenState::Invalid => TokenStateDto::Invalid,
                        };
                        (ts, token.expires_at)
                    }
                    None => (TokenStateDto::Invalid, None),
                };

            accounts.push(AccountDetail {
                account_id: info.account_id.clone(),
                label: info.label.clone(),
                is_active: info.is_active,
                token_state,
                expires_at,
            });
        }

        providers.push(ProviderAccounts {
            id: provider_id.to_string(),
            display_name: provider_id.display_name().to_string(),
            accounts,
        });
    }

    Ok(Json(AccountsResponse { providers }))
}

/// Removes a stored account (and its token) for a provider.
///
/// # Errors
///
/// Returns 400 if the provider is unknown, or `ApiError` on store failure.
#[utoipa::path(
    delete,
    path = "/v0/management/accounts/{provider}/{account_id}",
    params(
        ("provider" = String, Path, description = "Provider identifier"),
        ("account_id" = String, Path, description = "Account identifier"),
    ),
    responses((status = 204)),
    tag = "management"
)]
pub async fn remove_account_handler(
    State(state): State<Arc<AppState>>,
    Path((provider, account_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let provider_id: byokey_types::ProviderId = provider.parse()?;
    state
        .auth
        .remove_token_for(&provider_id, &account_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Switches the active account for a provider.
///
/// # Errors
///
/// Returns 400 if the provider is unknown, or `ApiError` on store failure.
#[utoipa::path(
    post,
    path = "/v0/management/accounts/{provider}/{account_id}/activate",
    params(
        ("provider" = String, Path, description = "Provider identifier"),
        ("account_id" = String, Path, description = "Account identifier"),
    ),
    responses((status = 204)),
    tag = "management"
)]
pub async fn activate_account_handler(
    State(state): State<Arc<AppState>>,
    Path((provider, account_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let provider_id: byokey_types::ProviderId = provider.parse()?;
    state
        .auth
        .set_active_account(&provider_id, &account_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
    use byokey_types::{OAuthToken, TokenStore as _};
    use http_body_util::BodyExt as _;
    use serde_json::Value;
    use tower::ServiceExt as _;

    fn make_state() -> (Arc<AppState>, Arc<InMemoryTokenStore>) {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store.clone(), rquest::Client::new()));
        let config = Arc::new(ArcSwap::from_pointee(Config::default()));
        (AppState::new(config, auth), store)
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn test_list_accounts_empty() {
        let (state, _store) = make_state();
        let app = make_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v0/management/accounts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let providers = json["providers"].as_array().unwrap();
        assert!(!providers.is_empty());
        for p in providers {
            assert!(p["accounts"].as_array().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn test_list_accounts_with_token() {
        let (state, store) = make_state();
        let token = OAuthToken::new("test-access").with_expiry(3600);
        store
            .save_account(
                &byokey_types::ProviderId::Claude,
                "default",
                Some("my-account"),
                &token,
            )
            .await
            .unwrap();

        let app = make_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v0/management/accounts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let claude = json["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "claude")
            .unwrap();
        let accounts = claude["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0]["account_id"], "default");
        assert_eq!(accounts[0]["label"], "my-account");
        assert_eq!(accounts[0]["token_state"], "valid");
        assert!(accounts[0]["is_active"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_remove_account() {
        let (state, store) = make_state();
        let token = OAuthToken::new("test-access");
        store
            .save_account(&byokey_types::ProviderId::Claude, "default", None, &token)
            .await
            .unwrap();

        let app = make_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v0/management/accounts/claude/default")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_remove_account_invalid_provider() {
        let (state, _store) = make_state();
        let app = make_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v0/management/accounts/nonexistent/default")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_activate_account() {
        let (state, store) = make_state();
        let provider = &byokey_types::ProviderId::Claude;
        let token1 = OAuthToken::new("tok1");
        let token2 = OAuthToken::new("tok2");
        store
            .save_account(provider, "acct1", None, &token1)
            .await
            .unwrap();
        store
            .save_account(provider, "acct2", None, &token2)
            .await
            .unwrap();

        let app = make_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v0/management/accounts/claude/acct2/activate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Verify acct2 is now active
        let accounts = store.list_accounts(provider).await.unwrap();
        let active = accounts.iter().find(|a| a.is_active).unwrap();
        assert_eq!(active.account_id, "acct2");
    }
}
