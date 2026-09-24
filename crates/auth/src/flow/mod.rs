//! Interactive login flow dispatcher for all supported providers.
//!
//! Delegates to [`auth_code::run`] or [`device_code::run`] via the
//! [`AuthCodeFlow`](auth_code::AuthCodeFlow) and
//! [`DeviceCodeFlow`](device_code::DeviceCodeFlow) traits.

pub mod auth_code;
pub mod device_code;

use byokey_types::{ByokError, CopilotClient, OAuthToken, ProviderId, Result};
use tokio::sync::mpsc;

use crate::AuthManager;
use crate::provider::{antigravity, claude, codex, copilot, gemini, iflow, kimi, qwen};

/// Progress event emitted by streaming login flows.
///
/// Consumers receive these on a [`mpsc::Receiver`] while the flow runs.
/// Terminal states (Done / Failed) are emitted by the caller — not by the
/// flow itself — so the flow only reports intermediate stages here.
#[derive(Debug, Clone)]
pub enum LoginProgress {
    /// The flow has begun (credentials fetched, about to open the browser
    /// or request a device code).
    Started,
    /// Browser opened for OAuth Auth Code flow (`url` is the authorization URL),
    /// or device-code verification page opened (`url` is the bare verification URI).
    /// For device-code flows, `user_code` carries the short code the user must enter.
    OpenedBrowser {
        url: String,
        user_code: Option<String>,
    },
    /// OAuth callback received; about to exchange the code (Auth Code flow only —
    /// Device Code flow has no distinct "got code" stage).
    GotCode,
    /// Received the callback / poll response. About to exchange the code
    /// for a token.
    Exchanging,
}

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
    provider: &ProviderId,
    auth: &AuthManager,
    options: LoginOptions<'_>,
) -> Result<()> {
    login_with_events(provider, auth, options, None).await
}

/// Run the login flow and emit progress events to the optional channel.
///
/// Identical to [`login`] but additionally forwards [`LoginProgress`] events
/// to the given channel. Used by the streaming management RPC so UIs can
/// render live progress.
///
/// # Errors
///
/// Same as [`login`].
pub async fn login_with_events(
    provider: &ProviderId,
    auth: &AuthManager,
    options: LoginOptions<'_>,
    events: Option<mpsc::Sender<LoginProgress>>,
) -> Result<()> {
    let http = wreq::Client::new();
    let ev = events.as_ref();
    let account = options.account;
    if let Some(client) = options.client
        && *provider != ProviderId::Copilot
    {
        return Err(ByokError::Auth(format!(
            "{provider} has a single login client; '{client}' is not selectable"
        )));
    }
    match provider {
        // Authorization Code + PKCE flows
        ProviderId::Claude => auth_code::run(&claude::Claude, auth, &http, account, ev).await,
        ProviderId::Codex => auth_code::run(&codex::Codex, auth, &http, account, ev).await,
        ProviderId::Gemini => auth_code::run(&gemini::Gemini, auth, &http, account, ev).await,
        ProviderId::Antigravity => {
            auth_code::run(&antigravity::Antigravity, auth, &http, account, ev).await
        }
        ProviderId::IFlow => auth_code::run(&iflow::IFlow, auth, &http, account, ev).await,
        // Device Code flows
        ProviderId::Copilot => {
            let client = options
                .client
                .map_or(Ok(CopilotClient::default()), str::parse)?;
            device_code::run(&copilot::Copilot { client }, auth, &http, account, ev).await
        }
        ProviderId::Qwen => device_code::run(&qwen::Qwen::new(), auth, &http, account, ev).await,
        ProviderId::Kimi => device_code::run(&kimi::Kimi, auth, &http, account, ev).await,
        ProviderId::Kiro => Err(ByokError::Auth(
            "Kiro OAuth login not yet implemented".into(),
        )),
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Save a token for a provider, routing to the named account if specified.
pub(crate) async fn save_login_token(
    auth: &AuthManager,
    provider: &ProviderId,
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

/// Best-effort event emit — swallows the error if the receiver has dropped.
pub(crate) async fn emit(events: Option<&mpsc::Sender<LoginProgress>>, p: LoginProgress) {
    if let Some(tx) = events {
        let _ = tx.send(p).await;
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
        assert!(login(&ProviderId::Claude, &auth(), options).await.is_err());
    }

    #[tokio::test]
    async fn unknown_copilot_client_is_rejected_before_any_request() {
        let options = LoginOptions {
            client: Some("jetbrains"),
            ..LoginOptions::default()
        };
        assert!(login(&ProviderId::Copilot, &auth(), options).await.is_err());
    }
}
