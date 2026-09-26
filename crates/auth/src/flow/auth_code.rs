//! Authorization Code + PKCE login flow.
//!
//! Defines the [`AuthCodeFlow`] trait that each auth-code provider implements,
//! and a generic [`run`] function that orchestrates the common flow:
//! fetch credentials → PKCE → browser redirect → callback → code exchange → save.

use async_trait::async_trait;
use byokey_types::{ByokError, OAuthToken, ProviderId, Result};
use serde_json::Value;

use super::{open_browser, save_login_token};
use crate::{AuthManager, callback, credentials::OAuthCredentials, pkce, token};

/// Provider-specific behavior for the Authorization Code + PKCE OAuth flow.
#[async_trait]
pub trait AuthCodeFlow: Send + Sync {
    /// The provider identifier for token storage.
    fn provider_id(&self) -> ProviderId;

    /// Provider name used for credential lookup from CDN (e.g. `"claude"`).
    fn provider_name(&self) -> &'static str;

    /// Local port for the OAuth callback redirect.
    fn callback_port(&self) -> u16;

    /// Build the authorization URL opened in the user's browser.
    fn build_auth_url(&self, client_id: &str, pkce_challenge: &str, state: &str) -> String;

    /// Exchange the authorization code for an access token.
    ///
    /// Each provider builds its own HTTP request (JSON vs form, headers, etc.)
    /// and parses the response.
    async fn exchange_code(
        &self,
        http: &reqwest::Client,
        creds: &OAuthCredentials,
        code: &str,
        pkce_verifier: &str,
        state: &str,
    ) -> Result<OAuthToken>;
}

/// Run the Authorization Code flow for any provider implementing [`AuthCodeFlow`].
///
/// # Errors
///
/// Returns an error on network failure, state mismatch, missing callback parameters,
/// token parse failure, or if the underlying store fails to save.
pub async fn run<P: AuthCodeFlow>(
    provider: &P,
    auth: &AuthManager,
    http: &reqwest::Client,
    account: Option<&str>,
) -> Result<()> {
    tracing::info!(provider = %provider.provider_name(), "starting OAuth login");
    eprintln!(
        "[login] fetching credentials for {}...",
        provider.provider_name()
    );
    let creds = crate::credentials::fetch(provider.provider_name(), http).await?;

    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = provider.build_auth_url(&creds.client_id, &challenge, &state);

    let listeners = callback::bind_callback(provider.callback_port()).await?;
    eprintln!(
        "[login] opening browser for {}...",
        provider.provider_name()
    );
    eprintln!();
    eprintln!("If your browser does not open, copy and paste this URL:");
    eprintln!("  {auth_url}");
    eprintln!();
    eprintln!(
        "Listening for the OAuth callback on http://localhost:{}/...",
        provider.callback_port()
    );
    open_browser(&auth_url);
    tracing::info!(provider = %provider.provider_name(), "waiting for OAuth callback");
    eprintln!("[login] waiting for OAuth callback...");
    let params = callback::accept_callback(listeners).await?;

    verify_state(&params, &state)?;
    let code = extract_code(&params)?;
    tracing::info!(provider = %provider.provider_name(), "received OAuth code, exchanging");
    eprintln!("[login] received OAuth code, exchanging for token...");
    let tok = provider
        .exchange_code(http, &creds, code, &verifier, &state)
        .await?;

    save_login_token(auth, provider.provider_id(), tok, account).await?;
    Ok(())
}

/// Send an HTTP response and parse it as a standard OAuth token JSON.
///
/// Shared helper for [`AuthCodeFlow::exchange_code`] implementations.
///
/// # Errors
///
/// Returns an error if the response body cannot be parsed as JSON or is missing
/// the `access_token` field.
pub async fn send_and_parse_token(resp: reqwest::Response) -> Result<OAuthToken> {
    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    if !status.is_success() {
        let error = json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let description = json
            .get("error_description")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(ByokError::Auth(format!(
            "token exchange failed ({status}): {error}: {description}"
        )));
    }

    token::parse_token_response(&json)
}

fn verify_state(params: &std::collections::HashMap<String, String>, expected: &str) -> Result<()> {
    let received = params.get("state").map_or("", String::as_str);
    if received != expected {
        return Err(ByokError::Auth(
            "state mismatch, possible CSRF attack".into(),
        ));
    }
    Ok(())
}

fn extract_code(params: &std::collections::HashMap<String, String>) -> Result<&str> {
    params
        .get("code")
        .map(String::as_str)
        .ok_or_else(|| ByokError::Auth("missing code parameter in callback".into()))
}
