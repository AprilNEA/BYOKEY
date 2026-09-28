//! BYOKEY management `ConnectRPC` services, read by `byokey tui` and
//! `byokey route`.
//!
//! - `StatusService` — server address, provider login state, usage
//! - `AccountsService` — the accounts stored per provider
//! - `RoutesService` — which provider serves each model

// The generated traits return `impl Future`, so a handler that has nothing
// to await still declares `async fn` to satisfy them.
#![allow(
    clippy::unused_async_trait_impl,
    reason = "handler signatures follow the generated trait"
)]
// The traits declare `ServiceResult<impl Encodable<M>>` so a handler may
// return either the owned message or a borrowed view. Returning the owned
// type refines that bound, which connectrpc's own guide calls the intended
// use and tells callers to allow.
#![allow(
    refining_impl_trait,
    reason = "handlers return the owned response type, as connectrpc's guide prescribes"
)]

mod routes;

use std::sync::Arc;

use connectrpc::{
    RequestContext, Response, Router as ConnectRouter, ServiceRequest, ServiceResult,
};

use byokey_proto::byokey::accounts as acct;
use byokey_proto::byokey::routes as rt;
use byokey_proto::byokey::status as stat;

use crate::AppState;
use routes::RoutesServiceImpl;

/// Build a [`ConnectRouter`] with all management services registered.
#[must_use]
pub fn build_router(state: Arc<AppState>) -> ConnectRouter {
    use acct::AccountsServiceExt as _;
    use rt::RoutesServiceExt as _;
    use stat::StatusServiceExt as _;

    let router = ConnectRouter::new();
    let router = Arc::new(StatusServiceImpl(state.clone())).register(router);
    let router = Arc::new(RoutesServiceImpl(state.clone())).register(router);
    Arc::new(AccountsServiceImpl(state)).register(router)
}

struct StatusServiceImpl(Arc<AppState>);

impl stat::StatusService for StatusServiceImpl {
    async fn get_status(
        &self,
        _ctx: RequestContext,
        _: ServiceRequest<'_, stat::GetStatusRequest>,
    ) -> ServiceResult<stat::GetStatusResponse> {
        let snapshot = self.0.config.load();
        let server = stat::ServerInfo {
            host: snapshot.host.clone(),
            port: u32::from(snapshot.port),
            ..Default::default()
        };

        let mut providers = Vec::new();
        for pid in byokey_types::ProviderId::all() {
            let cfg = snapshot.providers.get(&pid);
            let has_key = cfg.is_some_and(|c| c.api_key.is_some());
            let auth = if has_key || self.0.auth.is_authenticated(pid).await {
                stat::AuthStatus::AUTH_STATUS_VALID
            } else {
                let accts = self.0.auth.list_accounts(pid).await.unwrap_or_default();
                if accts.is_empty() {
                    stat::AuthStatus::AUTH_STATUS_NOT_CONFIGURED
                } else {
                    stat::AuthStatus::AUTH_STATUS_EXPIRED
                }
            };
            providers.push(stat::ProviderStatus {
                id: pid.to_string(),
                display_name: pid.display_name().to_string(),
                enabled: cfg.is_none_or(|c| c.enabled),
                auth_status: auth.into(),
                ..Default::default()
            });
        }
        Response::ok(stat::GetStatusResponse {
            server: server.into(),
            providers,
            ..Default::default()
        })
    }

    async fn get_usage(
        &self,
        _ctx: RequestContext,
        _: ServiceRequest<'_, stat::GetUsageRequest>,
    ) -> ServiceResult<stat::GetUsageResponse> {
        let s = self.0.usage.snapshot();
        let models = s
            .models
            .into_iter()
            .map(|(k, m)| {
                (
                    k,
                    stat::ModelStats {
                        requests: m.requests,
                        success: m.success,
                        failure: m.failure,
                        input_tokens: m.input_tokens,
                        output_tokens: m.output_tokens,
                        ..Default::default()
                    },
                )
            })
            .collect();
        Response::ok(stat::GetUsageResponse {
            total_requests: s.total_requests,
            success_requests: s.success_requests,
            failure_requests: s.failure_requests,
            input_tokens: s.input_tokens,
            output_tokens: s.output_tokens,
            models,
            ..Default::default()
        })
    }
}

/// A token's state as the accounts service reports it.
fn wire_token_state(state: byokey_types::TokenState) -> acct::TokenState {
    match state {
        byokey_types::TokenState::Valid => acct::TokenState::TOKEN_STATE_VALID,
        byokey_types::TokenState::Expired => acct::TokenState::TOKEN_STATE_EXPIRED,
        byokey_types::TokenState::Invalid => acct::TokenState::TOKEN_STATE_INVALID,
    }
}

struct AccountsServiceImpl(Arc<AppState>);

impl acct::AccountsService for AccountsServiceImpl {
    async fn list_accounts(
        &self,
        _ctx: RequestContext,
        _: ServiceRequest<'_, acct::ListAccountsRequest>,
    ) -> ServiceResult<acct::ListAccountsResponse> {
        let mut providers = Vec::new();
        for pid in byokey_types::ProviderId::all() {
            let infos = self.0.auth.list_accounts(pid).await.unwrap_or_default();
            let tokens = self.0.auth.get_all_tokens(pid).await.unwrap_or_default();
            let accounts = infos
                .iter()
                .map(|info| {
                    let (ts, exp) = match tokens.iter().find(|t| t.account_id == info.account_id) {
                        Some(t) => (wire_token_state(t.token.state()), t.token.expires_at),
                        None => (acct::TokenState::TOKEN_STATE_INVALID, None),
                    };
                    acct::AccountDetail {
                        account_id: info.account_id.clone(),
                        label: info.label.clone(),
                        is_active: info.is_active,
                        token_state: ts.into(),
                        expires_at: exp,
                        ..Default::default()
                    }
                })
                .collect();
            providers.push(acct::ProviderAccounts {
                id: pid.to_string(),
                display_name: pid.display_name().to_string(),
                accounts,
                ..Default::default()
            });
        }
        Response::ok(acct::ListAccountsResponse {
            providers,
            ..Default::default()
        })
    }
}
