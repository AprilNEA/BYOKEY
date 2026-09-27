use std::sync::Arc;

use byokey_auth::flow::{LoginOptions, LoginStep};
use byokey_types::ProviderId;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::StreamExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use byokey_proto::byokey::accounts as acct;

use super::{parse_provider, to_connect_error};
use crate::AppState;

/// A token's state as the accounts service reports it.
fn wire_token_state(state: byokey_types::TokenState) -> acct::TokenState {
    match state {
        byokey_types::TokenState::Valid => acct::TokenState::TOKEN_STATE_VALID,
        byokey_types::TokenState::Expired => acct::TokenState::TOKEN_STATE_EXPIRED,
        byokey_types::TokenState::Invalid => acct::TokenState::TOKEN_STATE_INVALID,
    }
}

pub(super) struct AccountsServiceImpl(pub(super) Arc<AppState>);

impl acct::AccountsService for AccountsServiceImpl {
    async fn list_accounts(
        &self,
        _ctx: RequestContext,
        _: ServiceRequest<'_, acct::ListAccountsRequest>,
    ) -> ServiceResult<acct::ListAccountsResponse> {
        let mut providers = Vec::new();
        for pid in ProviderId::all() {
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

    async fn remove_account(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, acct::RemoveAccountRequest>,
    ) -> ServiceResult<acct::RemoveAccountResponse> {
        let provider = parse_provider(request.provider)?;
        self.0
            .auth
            .remove_token_for(provider, request.account_id)
            .await
            .map_err(|e| to_connect_error(&e))?;
        Response::ok(acct::RemoveAccountResponse::default())
    }

    async fn activate_account(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, acct::ActivateAccountRequest>,
    ) -> ServiceResult<acct::ActivateAccountResponse> {
        let provider = parse_provider(request.provider)?;
        self.0
            .auth
            .set_active_account(provider, request.account_id)
            .await
            .map_err(|e| to_connect_error(&e))?;
        Response::ok(acct::ActivateAccountResponse::default())
    }

    async fn add_api_key(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, acct::AddApiKeyRequest>,
    ) -> ServiceResult<acct::AddApiKeyResponse> {
        let provider = parse_provider(request.provider)?;
        let account_id = self
            .0
            .auth
            .add_api_key(provider, request.api_key, request.account_id, request.label)
            .await
            .map_err(|e| to_connect_error(&e))?;
        Response::ok(acct::AddApiKeyResponse {
            account_id,
            ..Default::default()
        })
    }

    async fn import_claude_code(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, acct::ImportClaudeCodeRequest>,
    ) -> ServiceResult<acct::ImportClaudeCodeResponse> {
        let account_id = self
            .0
            .auth
            .import_claude_code(request.account_id, request.label)
            .await
            .map_err(|e| to_connect_error(&e))?;
        Response::ok(acct::ImportClaudeCodeResponse {
            account_id,
            ..Default::default()
        })
    }

    async fn login(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, acct::LoginRequest>,
    ) -> ServiceResult<ServiceStream<acct::LoginEvent>> {
        let provider = parse_provider(request.provider)?;
        let req = request.to_owned_message();
        let auth = self.0.auth.clone();
        let (tx, rx) = mpsc::unbounded_channel();

        // The flow outlives this call: it waits on the user's browser. It
        // stops early once the client hangs up, so an abandoned login doesn't
        // hold its callback port.
        tokio::spawn(async move {
            let on_step = |step| {
                let _ = tx.send(step_event(step));
            };
            let options = LoginOptions {
                account: req.account_id.as_deref(),
                client: req.client.as_deref(),
            };
            let result = tokio::select! {
                result = byokey_auth::flow::login(provider, &auth, options, &on_step) => result,
                () = tx.closed() => return,
            };
            let last = match result {
                Ok(()) => acct::LoginDone::default().into(),
                Err(e) => acct::LoginFailed {
                    error: e.to_string(),
                    ..Default::default()
                }
                .into(),
            };
            let _ = tx.send(acct::LoginEvent {
                event: last,
                ..Default::default()
            });
        });

        Response::stream_ok(UnboundedReceiverStream::new(rx).map(Ok).boxed())
    }
}

fn step_event(step: LoginStep) -> acct::LoginEvent {
    let event = match step {
        LoginStep::Visit { url, user_code } => acct::LoginVisit {
            url,
            user_code,
            ..Default::default()
        }
        .into(),
        LoginStep::Exchanging => acct::LoginExchanging::default().into(),
    };
    acct::LoginEvent {
        event,
        ..Default::default()
    }
}
