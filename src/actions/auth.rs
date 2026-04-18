use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_daemon::process::ServerStatus;
use byokey_types::{OAuthToken, ProviderId};
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

    /// Store an API key as a non-expiring token for the given provider.
    pub async fn add_api_key(
        &self,
        provider: ProviderId,
        api_key: String,
        account: Option<String>,
        label: Option<String>,
    ) -> Result<()> {
        if api_key.trim().is_empty() {
            anyhow::bail!("api_key cannot be empty");
        }
        if api_key.len() > byokey_types::MAX_API_KEY_BYTES {
            anyhow::bail!(
                "api_key exceeds maximum length of {} bytes",
                byokey_types::MAX_API_KEY_BYTES
            );
        }
        let account_id = account
            .as_deref()
            .unwrap_or(byokey_types::DEFAULT_ACCOUNT)
            .to_string();
        let token = OAuthToken {
            access_token: api_key.trim().to_string(),
            refresh_token: None,
            expires_at: None,
            token_type: Some("api-key".to_string()),
        };
        self.auth
            .save_token_for(&provider, &account_id, label.as_deref(), token)
            .await
            .map_err(|e| anyhow::anyhow!("add-api-key failed: {e}"))?;
        println!("{provider}: API key saved to account '{account_id}'");
        Ok(())
    }

    /// Import the currently-logged-in Claude Code OAuth credentials from
    /// macOS Keychain (or `~/.claude/.credentials.json` on other platforms)
    /// as an Anthropic account.
    pub async fn import_claude_code(
        &self,
        account: Option<String>,
        label: Option<String>,
    ) -> Result<()> {
        let token = byokey_auth::provider::claude_code::load_token()
            .await
            .map_err(|e| anyhow::anyhow!("read Claude Code credentials: {e}"))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no Claude Code credentials found — is Claude Code logged in on this machine?"
                )
            })?;
        let provider = ProviderId::Claude;
        let account_id = account
            .as_deref()
            .unwrap_or(byokey_types::CLAUDE_CODE_ACCOUNT)
            .to_string();
        let label = label.unwrap_or_else(|| "Claude Code".to_string());
        self.auth
            .save_token_for(&provider, &account_id, Some(label.as_str()), token)
            .await
            .map_err(|e| anyhow::anyhow!("save Claude Code token: {e}"))?;
        println!("{provider}: imported Claude Code credentials to account '{account_id}'");
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
