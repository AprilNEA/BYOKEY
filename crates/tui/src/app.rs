//! TUI application state and management API snapshot refresh logic.

use anyhow::{Context as _, Result};
use byokey_proto::byokey::{accounts as acct, status as stat};
use byokey_proto::client::ManagementClient;
use std::{collections::HashMap, time::SystemTime};

struct ManagementSnapshot {
    server: ConnectionStatus,
    providers: Vec<ProviderSnapshot>,
    usage: UsageSnapshot,
}

impl ManagementSnapshot {
    async fn fetch(client: &ManagementClient) -> Result<Self> {
        let status = client
            .get_status()
            .await
            .map_err(|e| anyhow::anyhow!("GetStatus failed: {e}"))?;
        let accounts = client
            .list_accounts()
            .await
            .map_err(|e| anyhow::anyhow!("ListAccounts failed: {e}"))?;
        let usage = client
            .get_usage()
            .await
            .map_err(|e| anyhow::anyhow!("GetUsage failed: {e}"))?;

        Self::from_proto(status, accounts, usage)
    }

    fn from_proto(
        status: stat::GetStatusResponse,
        accounts: acct::ListAccountsResponse,
        usage: stat::GetUsageResponse,
    ) -> Result<Self> {
        let server_info = status.server.into_option().context("missing server info")?;
        let port = u16::try_from(server_info.port).unwrap_or(u16::MAX);
        let server = ConnectionStatus::Connected {
            host: server_info.host,
            port,
        };

        let accounts_by_provider: HashMap<String, Vec<AccountSnapshot>> = accounts
            .providers
            .into_iter()
            .map(|provider| {
                let accounts = provider
                    .accounts
                    .into_iter()
                    .map(AccountSnapshot::from_proto)
                    .collect();
                (provider.id, accounts)
            })
            .collect();

        let providers = status
            .providers
            .into_iter()
            .map(|provider| {
                let accounts = accounts_by_provider
                    .get(&provider.id)
                    .cloned()
                    .unwrap_or_default();
                ProviderSnapshot {
                    id: provider.id,
                    display_name: provider.display_name,
                    enabled: provider.enabled,
                    auth_state: AuthState::from_proto(provider.auth_status.as_known()),
                    accounts,
                    models_count: provider.models_count,
                }
            })
            .collect();

        Ok(Self {
            server,
            providers,
            usage: UsageSnapshot::from_proto(usage),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Status,
    Accounts,
    Usage,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Status, Tab::Accounts, Tab::Usage];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Status => "Status",
            Tab::Accounts => "Accounts",
            Tab::Usage => "Usage",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Tab::Status => 0,
            Tab::Accounts => 1,
            Tab::Usage => 2,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected { host: String, port: u16 },
    Disconnected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthState {
    Authenticated,
    Expired,
    NotConfigured,
    Unknown,
}

impl AuthState {
    fn from_proto(status: Option<stat::AuthStatus>) -> Self {
        match status {
            Some(stat::AuthStatus::AUTH_STATUS_VALID) => Self::Authenticated,
            Some(stat::AuthStatus::AUTH_STATUS_EXPIRED) => Self::Expired,
            Some(stat::AuthStatus::AUTH_STATUS_NOT_CONFIGURED) => Self::NotConfigured,
            Some(stat::AuthStatus::AUTH_STATUS_UNSPECIFIED) | None => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    Valid,
    Expired,
    Invalid,
    Unknown,
}

impl TokenState {
    fn from_proto(state: Option<acct::TokenState>) -> Self {
        match state {
            Some(acct::TokenState::TOKEN_STATE_VALID) => Self::Valid,
            Some(acct::TokenState::TOKEN_STATE_EXPIRED) => Self::Expired,
            Some(acct::TokenState::TOKEN_STATE_INVALID) => Self::Invalid,
            Some(acct::TokenState::TOKEN_STATE_UNSPECIFIED) | None => Self::Unknown,
        }
    }
}

#[derive(Clone)]
pub struct AccountSnapshot {
    pub account_id: String,
    pub label: Option<String>,
    pub is_active: bool,
    pub token_state: TokenState,
}

impl AccountSnapshot {
    fn from_proto(account: acct::AccountDetail) -> Self {
        Self {
            account_id: account.account_id,
            label: account.label,
            is_active: account.is_active,
            token_state: TokenState::from_proto(account.token_state.as_known()),
        }
    }
}

pub struct ProviderSnapshot {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub auth_state: AuthState,
    pub accounts: Vec<AccountSnapshot>,
    pub models_count: u32,
}

pub struct UsageRow {
    pub model: String,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Default)]
pub struct UsageSnapshot {
    pub total_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub rows: Vec<UsageRow>,
}

impl UsageSnapshot {
    fn from_proto(usage: stat::GetUsageResponse) -> Self {
        let mut rows: Vec<_> = usage
            .models
            .into_iter()
            .map(|(model, stats)| UsageRow {
                model,
                request_count: stats.requests,
                input_tokens: stats.input_tokens,
                output_tokens: stats.output_tokens,
            })
            .collect();
        rows.sort_unstable_by(|a, b| a.model.cmp(&b.model));

        Self {
            total_requests: usage.total_requests,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            rows,
        }
    }

    pub fn total_requests(&self) -> u64 {
        self.total_requests
    }

    pub fn total_input(&self) -> u64 {
        self.input_tokens
    }

    pub fn total_output(&self) -> u64 {
        self.output_tokens
    }
}

pub struct App {
    client: ManagementClient,
    pub tab: Tab,
    pub server: ConnectionStatus,
    pub providers: Vec<ProviderSnapshot>,
    pub usage: UsageSnapshot,
    pub selected: usize,
    pub last_refresh: Option<SystemTime>,
    pub last_error: Option<String>,
}

impl App {
    pub fn new(client: ManagementClient) -> Self {
        Self {
            client,
            tab: Tab::Status,
            server: ConnectionStatus::Disconnected,
            providers: Vec::new(),
            usage: UsageSnapshot::default(),
            selected: 0,
            last_refresh: None,
            last_error: None,
        }
    }

    pub fn next_tab(&mut self) {
        let idx = (self.tab.index() + 1) % Tab::ALL.len();
        self.tab = Tab::ALL[idx];
        self.selected = 0;
    }

    pub fn prev_tab(&mut self) {
        let idx = (self.tab.index() + Tab::ALL.len() - 1) % Tab::ALL.len();
        self.tab = Tab::ALL[idx];
        self.selected = 0;
    }

    pub fn scroll_down(&mut self) {
        let max = self.list_len_for_current_tab().saturating_sub(1);
        self.selected = (self.selected + 1).min(max);
    }

    pub fn scroll_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn list_len_for_current_tab(&self) -> usize {
        match self.tab {
            Tab::Status => self.providers.len(),
            Tab::Accounts => self.providers.iter().map(|p| p.accounts.len().max(1)).sum(),
            Tab::Usage => self.usage.rows.len(),
        }
    }

    pub async fn refresh(&mut self) {
        match ManagementSnapshot::fetch(&self.client).await {
            Ok(snapshot) => {
                self.server = snapshot.server;
                self.providers = snapshot.providers;
                self.usage = snapshot.usage;
                self.last_error = None;
            }
            Err(e) => {
                self.server = ConnectionStatus::Disconnected;
                self.providers.clear();
                self.usage = UsageSnapshot::default();
                self.last_error = Some(format!("management API unavailable: {e}"));
            }
        }

        let max = self.list_len_for_current_tab().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
        self.last_refresh = Some(SystemTime::now());
    }
}
