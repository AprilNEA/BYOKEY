//! Alibaba Cloud Tongyi Qwen Device Code + PKCE flow configuration.
//!
//! The device code request also includes a PKCE `code_challenge`.
//! Slow-down multiplier: 1.5x.

use crate::token::DeviceCodeResponse;
use byokey_types::{ByokError, traits::Result};

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
