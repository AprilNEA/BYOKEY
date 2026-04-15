use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_daemon::process::ServerStatus;
use byokey_types::ProviderId;
use std::{path::PathBuf, sync::Arc};

pub struct AuthCmd {
    auth: AuthManager,
}

impl AuthCmd {
    pub async fn new(db: Option<PathBuf>) -> Result<Self> {
        eprintln!("[auth] opening store...");
        let store = Arc::new(crate::open_store(db).await?);
        eprintln!("[auth] creating http client...");
        let auth = AuthManager::new(store, rquest::Client::new());
        eprintln!("[auth] ready");
        Ok(Self { auth })
    }

    pub async fn login(&self, provider: ProviderId, account: Option<String>) -> Result<()> {
        byokey_auth::flow::login(&provider, &self.auth, account.as_deref())
            .await
            .map_err(|e| anyhow::anyhow!("login failed: {e}"))?;
        Ok(())
    }

    pub async fn logout(&self, provider: ProviderId, account: Option<String>) -> Result<()> {
        if let Some(account_id) = &account {
            self.auth
                .remove_token_for(&provider, account_id)
                .await
                .map_err(|e| anyhow::anyhow!("logout failed: {e}"))?;
            println!("{provider} account '{account_id}' logged out");
        } else {
            self.auth
                .remove_token(&provider)
                .await
                .map_err(|e| anyhow::anyhow!("logout failed: {e}"))?;
            println!("{provider} logged out");
        }
        Ok(())
    }

    pub async fn status(&self) -> Result<()> {
        match byokey_daemon::process::status() {
            Ok(ServerStatus::Running { pid }) => println!("server: running (pid {pid})"),
            Ok(ServerStatus::Stale { .. }) => println!("server: not running (stale pid file)"),
            Ok(ServerStatus::Stopped) | Err(_) => println!("server: not running"),
        }
        println!();

        for provider in ProviderId::all() {
            let accounts = self.auth.list_accounts(provider).await.unwrap_or_default();
            if accounts.is_empty() {
                println!("{provider}: not authenticated");
            } else if accounts.len() == 1 {
                let status = if self.auth.is_authenticated(provider).await {
                    "authenticated"
                } else {
                    "expired"
                };
                println!("{provider}: {status}");
            } else {
                let active = accounts.iter().find(|a| a.is_active);
                let label = active
                    .and_then(|a| a.label.as_deref())
                    .unwrap_or_else(|| active.map_or("?", |a| a.account_id.as_str()));
                println!("{provider}: {} account(s), active: {label}", accounts.len());
            }
        }
        Ok(())
    }

    pub async fn accounts(&self, provider: ProviderId) -> Result<()> {
        let accounts = self
            .auth
            .list_accounts(&provider)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        if accounts.is_empty() {
            println!("{provider}: no accounts");
        } else {
            for a in &accounts {
                let marker = if a.is_active { " (active)" } else { "" };
                let label = a
                    .label
                    .as_deref()
                    .map_or(String::new(), |l| format!(" [{l}]"));
                println!("  {}{label}{marker}", a.account_id);
            }
        }
        Ok(())
    }

    pub async fn switch(&self, provider: ProviderId, account: String) -> Result<()> {
        self.auth
            .set_active_account(&provider, &account)
            .await
            .map_err(|e| anyhow::anyhow!("switch failed: {e}"))?;
        println!("{provider}: switched to account '{account}'");
        Ok(())
    }
}
