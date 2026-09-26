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
) -> Result<()> {
    let http = wreq::Client::new();
    let account = options.account;
    if let Some(client) = options.client
        && provider != ProviderId::Copilot
    {
        return Err(ByokError::Auth(format!(
            "{provider} has a single login client; '{client}' is not selectable"
        )));
    }
    match provider {
        ProviderId::Claude => auth_code::run(&claude::Claude, auth, &http, account).await,
        ProviderId::Copilot => {
            let client = options
                .client
                .map_or(Ok(CopilotClient::default()), str::parse)?;
            device_code::run(&copilot::Copilot { client }, auth, &http, account).await
        }
        ProviderId::Cursor => cursor::login(auth, &http, account).await,
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

pub(crate) fn open_browser(url: &str) {
    tracing::info!(url = %url, "opening browser for OAuth login");
    if let Err(e) = open::that(url) {
        tracing::warn!(error = %e, url = %url, "failed to open browser, open URL manually");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;
    use std::sync::Arc;

    fn auth() -> AuthManager {
        AuthManager::new(Arc::new(InMemoryTokenStore::new()), wreq::Client::new())
    }

    #[tokio::test]
    async fn client_is_rejected_for_single_client_providers() {
        let options = LoginOptions {
            client: Some("vscode"),
            ..LoginOptions::default()
        };
        assert!(login(ProviderId::Claude, &auth(), options).await.is_err());
    }

    #[tokio::test]
    async fn unknown_copilot_client_is_rejected_before_any_request() {
        let options = LoginOptions {
            client: Some("jetbrains"),
            ..LoginOptions::default()
        };
        assert!(login(ProviderId::Copilot, &auth(), options).await.is_err());
    }
}
