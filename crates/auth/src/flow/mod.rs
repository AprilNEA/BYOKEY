//! Interactive login flow dispatcher for all supported providers.
//!
//! Delegates to [`auth_code::run`] or [`device_code::run`] via the
//! [`AuthCodeFlow`](auth_code::AuthCodeFlow) and
//! [`DeviceCodeFlow`](device_code::DeviceCodeFlow) traits.

pub mod auth_code;
pub mod device_code;

use byokey_types::{ByokError, OAuthToken, ProviderId, traits::Result};

use crate::AuthManager;
use crate::provider::{antigravity, claude, codex, copilot, gemini, iflow, kimi, qwen};

/// Run the full interactive login flow for the given provider.
///
/// When `account` is `Some`, the token is stored under that account identifier
/// instead of the default active account.
///
/// # Errors
///
/// Returns an error if the login flow fails for any reason (e.g., network error,
/// state mismatch, missing callback parameters, or token parse failure).
pub async fn login(provider: &ProviderId, auth: &AuthManager, account: Option<&str>) -> Result<()> {
    let http = rquest::Client::new();
    match provider {
        // Authorization Code + PKCE flows
        ProviderId::Claude => auth_code::run(&claude::Claude, auth, &http, account).await,
        ProviderId::Codex => auth_code::run(&codex::Codex, auth, &http, account).await,
        ProviderId::Gemini => auth_code::run(&gemini::Gemini, auth, &http, account).await,
        ProviderId::Antigravity => {
            auth_code::run(&antigravity::Antigravity, auth, &http, account).await
        }
        ProviderId::IFlow => auth_code::run(&iflow::IFlow, auth, &http, account).await,
        // Device Code flows
        ProviderId::Copilot => device_code::run(&copilot::Copilot, auth, &http, account).await,
        ProviderId::Qwen => device_code::run(&qwen::Qwen::new(), auth, &http, account).await,
        ProviderId::Kimi => device_code::run(&kimi::Kimi, auth, &http, account).await,
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
