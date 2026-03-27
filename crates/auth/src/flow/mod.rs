//! Interactive login flow dispatcher for all supported providers.
//!
//! Delegates to [`auth_code`] for Authorization Code + PKCE flows and
//! [`device_code`] for Device Authorization Grant flows.

mod auth_code;
mod device_code;

use byokey_types::{ByokError, OAuthToken, ProviderId, traits::Result};

use crate::AuthManager;

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
        ProviderId::Claude => auth_code::login_claude(auth, &http, account).await,
        ProviderId::Codex => auth_code::login_codex(auth, &http, account).await,
        ProviderId::Gemini => auth_code::login_gemini(auth, &http, account).await,
        ProviderId::Antigravity => auth_code::login_antigravity(auth, &http, account).await,
        ProviderId::IFlow => auth_code::login_iflow(auth, &http, account).await,
        ProviderId::Copilot => device_code::login_copilot(auth, &http, account).await,
        ProviderId::Qwen => device_code::login_qwen(auth, &http, account).await,
        ProviderId::Kimi => device_code::login_kimi(auth, &http, account).await,
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
