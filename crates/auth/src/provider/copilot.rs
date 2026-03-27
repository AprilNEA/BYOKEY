//! GitHub Copilot device code authorization flow configuration.
//!
//! Implements the OAuth 2.0 Device Authorization Grant used by GitHub Copilot.
//! No local callback port is needed for this flow.

use async_trait::async_trait;
use byokey_types::{ByokError, ProviderId, traits::Result};

use crate::token::DeviceCodeResponse;

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &["read:user"];

/// Parse the device code endpoint JSON response.
///
/// # Errors
///
/// Returns an error if `device_code` or `user_code` is missing.
pub fn parse_device_code_response(json: &serde_json::Value) -> Result<DeviceCodeResponse> {
    Ok(DeviceCodeResponse {
        device_code: json
            .get("device_code")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ByokError::Auth("missing device_code".into()))?
            .to_string(),
        user_code: json
            .get("user_code")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ByokError::Auth("missing user_code".into()))?
            .to_string(),
        verification_uri: json
            .get("verification_uri")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("https://github.com/login/device")
            .to_string(),
        expires_in: json
            .get("expires_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(900),
        interval: json
            .get("interval")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5),
    })
}

// ── DeviceCodeFlow implementation ─────────────────────────────────────────────

use crate::credentials::OAuthCredentials;
use crate::flow::device_code::{self, DeviceCodeFlow, PollResult};
use crate::token::DeviceCodeResponse as DcResp;

/// Copilot device-code provider.
pub struct Copilot;

#[async_trait]
impl DeviceCodeFlow for Copilot {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Copilot
    }
    fn provider_name(&self) -> &'static str {
        "copilot"
    }

    async fn request_device_code(
        &self,
        http: &rquest::Client,
        creds: &OAuthCredentials,
    ) -> Result<DcResp> {
        let device_code_url = creds
            .device_code_url
            .as_deref()
            .ok_or_else(|| ByokError::Auth("copilot credentials missing device_code_url".into()))?;
        let scope_str = SCOPES.join(" ");
        let params = [
            ("client_id", creds.client_id.as_str()),
            ("scope", scope_str.as_str()),
        ];
        let resp = http
            .post(device_code_url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await?;
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ByokError::Auth(format!("failed to parse device code response: {e}")))?;
        parse_device_code_response(&json)
    }

    async fn poll_token(
        &self,
        http: &rquest::Client,
        creds: &OAuthCredentials,
        device_code: &str,
    ) -> Result<PollResult> {
        let token_url = creds
            .token_url
            .as_deref()
            .ok_or_else(|| ByokError::Auth("copilot credentials missing token_url".into()))?;
        let params = [
            ("client_id", creds.client_id.as_str()),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];
        let resp = http
            .post(token_url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await?;
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;
        device_code::parse_poll_response(&json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_device_code() {
        let resp = json!({
            "device_code": "dc",
            "user_code": "XXXX-YYYY",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900,
            "interval": 5
        });
        let dc = parse_device_code_response(&resp).unwrap();
        assert_eq!(dc.user_code, "XXXX-YYYY");
        assert_eq!(dc.expires_in, 900);
    }

    #[test]
    fn test_parse_device_code_missing() {
        assert!(parse_device_code_response(&json!({})).is_err());
    }
}
