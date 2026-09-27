//! Small management API client wrapper around generated ConnectRPC clients.

use std::time::Duration;

use connectrpc::ConnectError;
use connectrpc::client::{
    CallOptions, ClientConfig, ClientTransport, HttpClient, ServerStream, UnaryResponse,
};

use crate::byokey::{accounts as acct, status as stat};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// A login waits on the user in a browser. Each server-side flow gives up on
/// its own well before this, so it only guards against a hung server.
const LOGIN_TIMEOUT: Duration = Duration::from_mins(15);

/// The events of one [`ManagementClient::login`], ending with `LoginDone` or
/// `LoginFailed`.
pub type LoginStream =
    ServerStream<<HttpClient as ClientTransport>::ResponseBody, acct::LoginEventView<'static>>;

/// Shared client for BYOKEY's local ConnectRPC management API.
#[derive(Clone)]
pub struct ManagementClient {
    status: stat::StatusServiceClient<HttpClient>,
    accounts: acct::AccountsServiceClient<HttpClient>,
}

impl ManagementClient {
    /// Build a plaintext HTTP client for a local management API endpoint.
    #[must_use]
    pub fn local_http(base_uri: http::Uri) -> Self {
        Self::with_config(ClientConfig::new(base_uri).with_default_timeout(DEFAULT_TIMEOUT))
    }

    /// Build a plaintext HTTP client with explicit ConnectRPC config.
    #[must_use]
    pub fn with_config(config: ClientConfig) -> Self {
        Self::with_transport(HttpClient::plaintext(), config)
    }

    /// Build a client from explicit transport and config.
    #[must_use]
    pub fn with_transport(transport: HttpClient, config: ClientConfig) -> Self {
        Self {
            status: stat::StatusServiceClient::new(transport.clone(), config.clone()),
            accounts: acct::AccountsServiceClient::new(transport, config),
        }
    }

    #[must_use]
    pub fn status(&self) -> &stat::StatusServiceClient<HttpClient> {
        &self.status
    }

    #[must_use]
    pub fn accounts(&self) -> &acct::AccountsServiceClient<HttpClient> {
        &self.accounts
    }

    /// Fetch server and provider status.
    ///
    /// # Errors
    ///
    /// Returns a ConnectRPC transport or application error from the server.
    pub async fn get_status(&self) -> Result<stat::GetStatusResponse, ConnectError> {
        self.status
            .get_status(stat::GetStatusRequest::default())
            .await
            .map(UnaryResponse::into_owned)
    }

    /// Fetch cumulative usage counters.
    ///
    /// # Errors
    ///
    /// Returns a ConnectRPC transport or application error from the server.
    pub async fn get_usage(&self) -> Result<stat::GetUsageResponse, ConnectError> {
        self.status
            .get_usage(stat::GetUsageRequest::default())
            .await
            .map(UnaryResponse::into_owned)
    }

    /// Fetch configured provider accounts.
    ///
    /// # Errors
    ///
    /// Returns a ConnectRPC transport or application error from the server.
    pub async fn list_accounts(&self) -> Result<acct::ListAccountsResponse, ConnectError> {
        self.accounts
            .list_accounts(acct::ListAccountsRequest::default())
            .await
            .map(UnaryResponse::into_owned)
    }

    /// Delete a stored account.
    ///
    /// # Errors
    ///
    /// `invalid_argument` for an unknown provider, or a transport error.
    pub async fn remove_account(
        &self,
        provider: &str,
        account_id: &str,
    ) -> Result<(), ConnectError> {
        self.accounts
            .remove_account(acct::RemoveAccountRequest {
                provider: provider.to_owned(),
                account_id: account_id.to_owned(),
                ..Default::default()
            })
            .await
            .map(drop)
    }

    /// Make an account the one a provider's requests use.
    ///
    /// # Errors
    ///
    /// `not_found` when the account doesn't exist, `invalid_argument` for an
    /// unknown provider, or a transport error.
    pub async fn activate_account(
        &self,
        provider: &str,
        account_id: &str,
    ) -> Result<(), ConnectError> {
        self.accounts
            .activate_account(acct::ActivateAccountRequest {
                provider: provider.to_owned(),
                account_id: account_id.to_owned(),
                ..Default::default()
            })
            .await
            .map(drop)
    }

    /// Store an API key as an account and return the account id it landed in.
    ///
    /// # Errors
    ///
    /// `invalid_argument` for an unknown provider or an empty or oversized
    /// key, or a transport error.
    pub async fn add_api_key(
        &self,
        request: acct::AddApiKeyRequest,
    ) -> Result<String, ConnectError> {
        self.accounts
            .add_api_key(request)
            .await
            .map(|r| r.into_owned().account_id)
    }

    /// Import the Claude Code login on the server's machine and return the
    /// account id it landed in.
    ///
    /// # Errors
    ///
    /// `invalid_argument` when Claude Code isn't logged in there, or a
    /// transport error.
    pub async fn import_claude_code(
        &self,
        request: acct::ImportClaudeCodeRequest,
    ) -> Result<String, ConnectError> {
        self.accounts
            .import_claude_code(request)
            .await
            .map(|r| r.into_owned().account_id)
    }

    /// Start an interactive login. The caller opens each `LoginVisit` URL;
    /// dropping the stream abandons the login on the server.
    ///
    /// # Errors
    ///
    /// `invalid_argument` for an unknown provider, or a transport error.
    pub async fn login(&self, request: acct::LoginRequest) -> Result<LoginStream, ConnectError> {
        self.accounts
            .login_with_options(request, CallOptions::default().with_timeout(LOGIN_TIMEOUT))
            .await
    }
}
