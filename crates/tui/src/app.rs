//! TUI application state and snapshot refresh logic.

use byokey_auth::AuthManager;
use byokey_daemon::process::ServerStatus;
use byokey_store::SqliteTokenStore;
use byokey_types::{AccountInfo, ProviderId, TokenState, UsageBucket, UsageStore};
use std::{sync::Arc, time::SystemTime};

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

pub struct ProviderSnapshot {
    pub id: ProviderId,
    pub display_name: &'static str,
    pub accounts: Vec<AccountInfo>,
    /// State of the active account (or `Invalid` when no accounts exist).
    pub active_state: TokenState,
}

pub struct UsageSnapshot {
    pub buckets: Vec<UsageBucket>,
}

impl UsageSnapshot {
    pub fn total_requests(&self) -> u64 {
        self.buckets.iter().map(|b| b.request_count).sum()
    }

    pub fn total_input(&self) -> u64 {
        self.buckets.iter().map(|b| b.input_tokens).sum()
    }

    pub fn total_output(&self) -> u64 {
        self.buckets.iter().map(|b| b.output_tokens).sum()
    }
}

pub struct App {
    store: Arc<SqliteTokenStore>,
    auth: Arc<AuthManager>,
    pub tab: Tab,
    pub server: ServerStatus,
    pub providers: Vec<ProviderSnapshot>,
    pub usage: UsageSnapshot,
    pub selected: usize,
    pub last_refresh: Option<SystemTime>,
    pub last_error: Option<String>,
}

impl App {
    pub fn new(store: Arc<SqliteTokenStore>, auth: Arc<AuthManager>) -> Self {
        Self {
            store,
            auth,
            tab: Tab::Status,
            server: ServerStatus::Stopped,
            providers: Vec::new(),
            usage: UsageSnapshot {
                buckets: Vec::new(),
            },
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
            Tab::Usage => self.usage.buckets.len(),
        }
    }

    pub async fn refresh(&mut self) {
        self.server = byokey_daemon::process::status().unwrap_or(ServerStatus::Stopped);
        self.last_error = None;

        let mut providers = Vec::with_capacity(ProviderId::all().len());
        for id in ProviderId::all() {
            let accounts = self.auth.list_accounts(id).await.unwrap_or_default();
            let active_state = if accounts.is_empty() {
                TokenState::Invalid
            } else {
                self.auth.token_state(id).await
            };
            providers.push(ProviderSnapshot {
                id: id.clone(),
                display_name: id.display_name(),
                accounts,
                active_state,
            });
        }
        self.providers = providers;

        match self.store.totals(None, None).await {
            Ok(buckets) => self.usage = UsageSnapshot { buckets },
            Err(e) => {
                self.last_error = Some(format!("usage query failed: {e}"));
                self.usage = UsageSnapshot {
                    buckets: Vec::new(),
                };
            }
        }

        let max = self.list_len_for_current_tab().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
        self.last_refresh = Some(SystemTime::now());
    }
}
