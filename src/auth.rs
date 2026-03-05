use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_daemon::process::ServerStatus;
use byokey_types::ProviderId;
use std::{path::PathBuf, sync::Arc};

pub async fn cmd_login(
    provider: ProviderId,
    account: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let auth = AuthManager::new(
        Arc::new(crate::open_store(db).await?),
        rquest::Client::new(),
    );
    byokey_auth::flow::login(&provider, &auth, account.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!("login failed: {e}"))?;
    Ok(())
}

pub async fn cmd_logout(
    provider: ProviderId,
    account: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let auth = AuthManager::new(
        Arc::new(crate::open_store(db).await?),
        rquest::Client::new(),
    );
    if let Some(account_id) = &account {
        auth.remove_token_for(&provider, account_id)
            .await
            .map_err(|e| anyhow::anyhow!("logout failed: {e}"))?;
        println!("{provider} account '{account_id}' logged out");
    } else {
        auth.remove_token(&provider)
            .await
            .map_err(|e| anyhow::anyhow!("logout failed: {e}"))?;
        println!("{provider} logged out");
    }
    Ok(())
}

pub async fn cmd_status(db: Option<PathBuf>) -> Result<()> {
    // Server running status
    match byokey_daemon::process::status() {
        Ok(ServerStatus::Running { pid }) => println!("server: running (pid {pid})"),
        Ok(ServerStatus::Stale { .. }) => println!("server: not running (stale pid file)"),
        Ok(ServerStatus::Stopped) | Err(_) => println!("server: not running"),
    }
    println!();

    let auth = AuthManager::new(
        Arc::new(crate::open_store(db).await?),
        rquest::Client::new(),
    );
    for provider in ProviderId::all() {
        let accounts = auth.list_accounts(provider).await.unwrap_or_default();
        if accounts.is_empty() {
            println!("{provider}: not authenticated");
        } else if accounts.len() == 1 {
            let status = if auth.is_authenticated(provider).await {
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

pub async fn cmd_accounts(provider: ProviderId, db: Option<PathBuf>) -> Result<()> {
    let auth = AuthManager::new(
        Arc::new(crate::open_store(db).await?),
        rquest::Client::new(),
    );
    let accounts = auth
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

pub async fn cmd_switch(provider: ProviderId, account: String, db: Option<PathBuf>) -> Result<()> {
    let auth = AuthManager::new(
        Arc::new(crate::open_store(db).await?),
        rquest::Client::new(),
    );
    auth.set_active_account(&provider, &account)
        .await
        .map_err(|e| anyhow::anyhow!("switch failed: {e}"))?;
    println!("{provider}: switched to account '{account}'");
    Ok(())
}
