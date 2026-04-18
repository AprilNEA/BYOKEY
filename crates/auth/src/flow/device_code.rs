//! Device Authorization Grant login flow.
//!
//! Defines the [`DeviceCodeFlow`] trait that each device-code provider implements,
//! and a generic [`run`] function that orchestrates the common polling loop.

use async_trait::async_trait;
use byokey_types::{ByokError, OAuthToken, ProviderId, Result};
use std::time::Duration;
use tokio::sync::mpsc;

use super::{LoginProgress, emit, open_browser, save_login_token};
use crate::{AuthManager, credentials::OAuthCredentials, token, token::DeviceCodeResponse};

/// Result of a single token poll attempt.
pub enum PollResult {
    /// Token exchange succeeded.
    Success(OAuthToken),
    /// Authorization is still pending — keep polling.
    Pending,
    /// Server asked to slow down — increase interval.
    SlowDown,
}

fn device_code_prompt(dc: &DeviceCodeResponse) -> String {
    format!(
        "Open this URL in your browser: {}\nEnter this code: {}",
        dc.verification_uri, dc.user_code
    )
}

/// Provider-specific behavior for the Device Authorization Grant flow.
#[async_trait]
pub trait DeviceCodeFlow: Send + Sync {
    /// The provider identifier for token storage.
    fn provider_id(&self) -> ProviderId;

    /// Provider name used for credential lookup from CDN (e.g. `"copilot"`).
    fn provider_name(&self) -> &'static str;

    /// Send the device code request and return the parsed response.
    async fn request_device_code(
        &self,
        http: &rquest::Client,
        creds: &OAuthCredentials,
    ) -> Result<DeviceCodeResponse>;

    /// Send a single token poll request.
    async fn poll_token(
        &self,
        http: &rquest::Client,
        creds: &OAuthCredentials,
        device_code: &str,
    ) -> Result<PollResult>;

    /// Adjust the polling interval on `slow_down`. Default: add 5 seconds.
    fn apply_slow_down(&self, current_interval: f64) -> f64 {
        current_interval + 5.0
    }
}

/// Run the Device Code flow for any provider implementing [`DeviceCodeFlow`].
///
/// # Errors
///
/// Returns an error on network failure, device code expiration, or token parse failure.
#[allow(clippy::cast_precision_loss)]
pub async fn run<P: DeviceCodeFlow>(
    provider: &P,
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
    events: Option<&mpsc::Sender<LoginProgress>>,
) -> Result<()> {
    emit(events, LoginProgress::Started).await;
    let creds = crate::credentials::fetch(provider.provider_name(), http).await?;
    let dc = provider.request_device_code(http, &creds).await?;
    let provider_id = provider.provider_id();

    tracing::info!(
        uri = %dc.verification_uri,
        code = %dc.user_code,
        "visit URL and enter verification code"
    );
    println!("{}", device_code_prompt(&dc));
    open_browser(&dc.verification_uri);
    emit(
        events,
        LoginProgress::OpenedBrowser {
            url: device_code_prompt(&dc),
        },
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in);
    let mut interval = dc.interval as f64;

    loop {
        tokio::time::sleep(Duration::from_secs_f64(interval)).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(ByokError::Auth("device code expired".into()));
        }

        match provider.poll_token(http, &creds, &dc.device_code).await? {
            PollResult::Success(tok) => {
                emit(events, LoginProgress::Exchanging).await;
                save_login_token(auth, &provider_id, tok, account).await?;
                println!("{provider_id} login successful");
                tracing::info!(provider = %provider_id, "login successful");
                return Ok(());
            }
            PollResult::Pending => {}
            PollResult::SlowDown => {
                interval = provider.apply_slow_down(interval);
            }
        }
    }
}

/// Parse a token poll response into a [`PollResult`].
///
/// Shared helper for [`DeviceCodeFlow::poll_token`] implementations.
///
/// # Errors
///
/// Returns an error if the server returns a terminal error or the token cannot be parsed.
pub fn parse_poll_response(json: &serde_json::Value) -> Result<PollResult> {
    match json.get("error").and_then(|v| v.as_str()) {
        Some("authorization_pending") => Ok(PollResult::Pending),
        Some("slow_down") => Ok(PollResult::SlowDown),
        Some(e) => Err(ByokError::Auth(format!("device flow error: {e}"))),
        None => {
            let tok = token::parse_token_response(json)?;
            Ok(PollResult::Success(tok))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_code_prompt_includes_url_and_code() {
        let dc = DeviceCodeResponse {
            device_code: "device-code".into(),
            user_code: "ABCD-1234".into(),
            verification_uri: "https://github.com/login/device".into(),
            expires_in: 900,
            interval: 5,
        };

        let prompt = device_code_prompt(&dc);
        assert!(prompt.contains("https://github.com/login/device"));
        assert!(prompt.contains("ABCD-1234"));
    }
}
