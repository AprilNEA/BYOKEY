//! Small management API client wrapper around generated ConnectRPC clients.

use std::time::Duration;

use connectrpc::ConnectError;
use connectrpc::client::{ClientConfig, HttpClient};

use crate::byokey::{accounts as acct, amp, status as stat};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared client for BYOKEY's local ConnectRPC management API.
#[derive(Clone)]
pub struct ManagementClient {
    status: stat::StatusServiceClient<HttpClient>,
    accounts: acct::AccountsServiceClient<HttpClient>,
    amp: amp::AmpServiceClient<HttpClient>,
}

impl ManagementClient {
    /// Build a plaintext HTTP client for a local management API endpoint.
    #[must_use]
    pub fn local_http(base_uri: http::Uri) -> Self {
        Self::with_config(ClientConfig::new(base_uri).default_timeout(DEFAULT_TIMEOUT))
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
            accounts: acct::AccountsServiceClient::new(transport.clone(), config.clone()),
            amp: amp::AmpServiceClient::new(transport, config),
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

    #[must_use]
    pub fn amp(&self) -> &amp::AmpServiceClient<HttpClient> {
        &self.amp
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
            .map(connectrpc::client::UnaryResponse::into_owned)
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
            .map(connectrpc::client::UnaryResponse::into_owned)
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
            .map(connectrpc::client::UnaryResponse::into_owned)
    }
}
