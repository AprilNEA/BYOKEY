//! Interactive login flow dispatcher.
//!
//! Delegates to [`auth_code::run`] or [`device_code::run`] via the
//! [`AuthCodeFlow`](auth_code::AuthCodeFlow) and
//! [`DeviceCodeFlow`](device_code::DeviceCodeFlow) traits.

pub mod auth_code;
pub mod device_code;

use byokey_types::{ByokError, CopilotClient, OAuthToken, ProviderId, Result};

use crate::AuthManager;
use crate::provider::{claude, copilot, cursor};

/// A login step the user has to act on or wait through.
///
/// The flow reports these instead of printing or opening a browser itself, so
/// the CLI and the management API each present them their own way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStep {
    /// Open `url` in a browser. Device-code flows also carry the `user_code`
    /// to enter there.
    Visit {
        url: String,
        user_code: Option<String>,
    },
    /// The browser half is done and the grant is being traded for a token.
    Exchanging,
}

/// Receives each [`LoginStep`] as the flow reaches it.
pub type OnStep<'a> = &'a (dyn Fn(LoginStep) + Send + Sync);

/// What to log in as.
#[derive(Debug, Clone, Default)]
pub struct LoginOptions<'a> {
    /// Store the token under this account instead of the default active one.
    pub account: Option<&'a str>,
    /// Log in as this client, for providers that support more than one
    /// (Copilot: `opencode`, `vscode`). `None` picks the provider's default.
    pub client: Option<&'a str>,
}

/// Run the full interactive login flow for the given provider.
///
/// # Errors
///
/// Returns an error if the login flow fails for any reason (e.g., network error,
/// state mismatch, missing callback parameters, or token parse failure), or if
/// `options.client` is not a client of `provider`.
pub async fn login(
    provider: ProviderId,
    auth: &AuthManager,
    options: LoginOptions<'_>,
    on_step: OnStep<'_>,
) -> Result<()> {
    let http = reqwest::Client::new();
    let account = options.account;
    if let Some(client) = options.client
        && provider != ProviderId::Copilot
    {
        return Err(ByokError::Auth(format!(
            "{provider} has a single login client; '{client}' is not selectable"
        )));
    }
    match provider {
        ProviderId::Claude => auth_code::run(&claude::Claude, auth, &http, account, on_step).await,
        ProviderId::Copilot => {
            let client = options
                .client
                .map_or(Ok(CopilotClient::default()), str::parse)?;
            device_code::run(&copilot::Copilot { client }, auth, &http, account, on_step).await
        }
        ProviderId::Cursor => cursor::login(auth, &http, account, on_step).await,
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Save a token for a provider, routing to the named account if specified.
pub(crate) async fn save_login_token(
    auth: &AuthManager,
    provider: ProviderId,
    token: OAuthToken,
    account: Option<&str>,
) -> Result<()> {
    if let Some(account_id) = account {
        auth.save_token_for(provider, account_id, None, token).await
    } else {
        auth.save_token(provider, token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;
    use std::sync::Arc;

    fn auth() -> AuthManager {
        AuthManager::new(Arc::new(InMemoryTokenStore::new()), reqwest::Client::new())
    }

    #[tokio::test]
    async fn client_is_rejected_for_single_client_providers() {
        let options = LoginOptions {
            client: Some("vscode"),
            ..LoginOptions::default()
        };
        assert!(
            login(ProviderId::Claude, &auth(), options, &|_| {})
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unknown_copilot_client_is_rejected_before_any_request() {
        let options = LoginOptions {
            client: Some("jetbrains"),
            ..LoginOptions::default()
        };
        assert!(
            login(ProviderId::Copilot, &auth(), options, &|_| {})
                .await
                .is_err()
        );
    }
}
