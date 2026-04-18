//! Interactive login flow dispatcher for all supported providers.
//!
//! Delegates to [`auth_code::run`] or [`device_code::run`] via the
//! [`AuthCodeFlow`](auth_code::AuthCodeFlow) and
//! [`DeviceCodeFlow`](device_code::DeviceCodeFlow) traits.

pub mod auth_code;
pub mod device_code;

use byokey_types::{ByokError, OAuthToken, ProviderId, Result};
use tokio::sync::mpsc;

use crate::AuthManager;
use crate::provider::{amp, antigravity, claude, codex, copilot, gemini, iflow, kimi, qwen};

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
    login_with_events(provider, auth, account, None).await
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
    account: Option<&str>,
    events: Option<mpsc::Sender<LoginProgress>>,
) -> Result<()> {
    let http = rquest::Client::new();
    let ev = events.as_ref();
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
        ProviderId::Copilot => device_code::run(&copilot::Copilot, auth, &http, account, ev).await,
        ProviderId::Qwen => device_code::run(&qwen::Qwen::new(), auth, &http, account, ev).await,
        ProviderId::Kimi => device_code::run(&kimi::Kimi, auth, &http, account, ev).await,
        ProviderId::Amp => auth_code::run(&amp::Amp, auth, &http, account, ev).await,
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
