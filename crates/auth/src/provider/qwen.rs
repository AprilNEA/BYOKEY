//! Alibaba Cloud Tongyi Qwen Device Code + PKCE flow configuration.
//!
//! The device code request also includes a PKCE `code_challenge`.
//! Slow-down multiplier: 1.5x.

use async_trait::async_trait;
use byokey_types::{ByokError, ProviderId, traits::Result};

use crate::token::DeviceCodeResponse;

pub const SCOPES: &[&str] = &["openid", "profile", "email", "model.completion"];
pub const SLOW_DOWN_MULTIPLIER: f64 = 1.5;

#[must_use]
pub fn build_device_code_params(
    client_id: &str,
    code_challenge: &str,
    scope: &str,
) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), client_id.into()),
        ("scope".into(), scope.into()),
        ("code_challenge".into(), code_challenge.into()),
        ("code_challenge_method".into(), "S256".into()),
    ]
}

#[must_use]
pub fn build_token_poll_params(
    client_id: &str,
    device_code: &str,
    code_verifier: &str,
) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), client_id.into()),
        ("device_code".into(), device_code.into()),
        (
            "grant_type".into(),
            "urn:ietf:params:oauth:grant-type:device_code".into(),
        ),
        ("code_verifier".into(), code_verifier.into()),
    ]
}

/// # Errors
///
/// Returns an error if the response is missing required fields (`device_code`, `user_code`, or `verification_uri`).
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
            .ok_or_else(|| ByokError::Auth("missing verification_uri".into()))?
            .to_string(),
        expires_in: json
            .get("expires_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(600),
        interval: json
            .get("interval")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5),
    })
}

// ── DeviceCodeFlow implementation ─────────────────────────────────────────────

use crate::credentials::OAuthCredentials;
use crate::flow::device_code::{self, DeviceCodeFlow, PollResult};
use crate::pkce;
use crate::token::DeviceCodeResponse as DcResp;

/// Qwen device-code provider (with PKCE).
pub struct Qwen {
    verifier: String,
}

impl Default for Qwen {
    fn default() -> Self {
        Self::new()
    }
}

impl Qwen {
    /// Creates a new Qwen provider with a fresh PKCE pair.
    #[must_use]
    pub fn new() -> Self {
        let (verifier, _) = pkce::generate_pkce();
        Self { verifier }
    }
}

#[async_trait]
impl DeviceCodeFlow for Qwen {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Qwen
    }
    fn provider_name(&self) -> &'static str {
        "qwen"
    }
    fn apply_slow_down(&self, current_interval: f64) -> f64 {
        current_interval * SLOW_DOWN_MULTIPLIER
    }

    async fn request_device_code(
        &self,
        http: &rquest::Client,
        creds: &OAuthCredentials,
    ) -> Result<DcResp> {
        let device_code_url = creds
            .device_code_url
            .as_deref()
            .ok_or_else(|| ByokError::Auth("qwen credentials missing device_code_url".into()))?;
        let challenge = pkce::challenge_for(&self.verifier);
        let scope_str = SCOPES.join(" ");
        let params = build_device_code_params(&creds.client_id, &challenge, &scope_str);
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
            .ok_or_else(|| ByokError::Auth("qwen credentials missing token_url".into()))?;
        let params = build_token_poll_params(&creds.client_id, device_code, &self.verifier);
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
    fn test_parse_device_code_success() {
        let resp = json!({
            "device_code": "qwen-dc-123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://chat.qwen.ai/device",
            "expires_in": 300,
            "interval": 10
        });
        let dc = parse_device_code_response(&resp).unwrap();
        assert_eq!(dc.device_code, "qwen-dc-123");
        assert_eq!(dc.user_code, "ABCD-1234");
        assert_eq!(dc.verification_uri, "https://chat.qwen.ai/device");
        assert_eq!(dc.expires_in, 300);
        assert_eq!(dc.interval, 10);
    }

    #[test]
    fn test_parse_device_code_defaults() {
        let resp = json!({
            "device_code": "dc",
            "user_code": "UC",
            "verification_uri": "https://example.com"
        });
        let dc = parse_device_code_response(&resp).unwrap();
        assert_eq!(dc.expires_in, 600);
        assert_eq!(dc.interval, 5);
    }

    #[test]
    fn test_parse_device_code_missing_device_code() {
        let resp = json!({
            "user_code": "UC",
            "verification_uri": "https://example.com"
        });
        assert!(parse_device_code_response(&resp).is_err());
    }

    #[test]
    fn test_parse_device_code_missing_user_code() {
        let resp = json!({
            "device_code": "dc",
            "verification_uri": "https://example.com"
        });
        assert!(parse_device_code_response(&resp).is_err());
    }

    #[test]
    fn test_parse_device_code_missing_verification_uri() {
        let resp = json!({
            "device_code": "dc",
            "user_code": "UC"
        });
        assert!(parse_device_code_response(&resp).is_err());
    }

    const TEST_CLIENT_ID: &str = "test-qwen-client-id";

    #[test]
    fn test_build_device_code_params() {
        let params = build_device_code_params(TEST_CLIENT_ID, "challenge123", "openid profile");
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "client_id" && v == TEST_CLIENT_ID)
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "scope" && v == "openid profile")
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "code_challenge" && v == "challenge123")
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "code_challenge_method" && v == "S256")
        );
    }

    #[test]
    fn test_build_token_poll_params() {
        let params = build_token_poll_params(TEST_CLIENT_ID, "dc-abc", "verifier-xyz");
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "client_id" && v == TEST_CLIENT_ID)
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "device_code" && v == "dc-abc")
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "grant_type"
                    && v == "urn:ietf:params:oauth:grant-type:device_code")
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "code_verifier" && v == "verifier-xyz")
        );
    }
}
