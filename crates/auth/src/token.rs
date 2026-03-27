//! Shared token response parsing and device code response types.
//!
//! Provides a single [`parse_token_response`] implementation used by all
//! providers and the token refresh path in [`AuthManager`](crate::AuthManager),
//! eliminating the per-provider duplication.

use byokey_types::{ByokError, OAuthToken, traits::Result};

/// Parsed response from an OAuth 2.0 Device Authorization Grant.
///
/// Shared across device code flows: Copilot, Qwen, Kimi, Kiro.
#[derive(Debug)]
pub struct DeviceCodeResponse {
    /// Unique device verification code.
    pub device_code: String,
    /// Short code the user enters at the verification URI.
    pub user_code: String,
    /// URL where the user authorizes the device.
    pub verification_uri: String,
    /// Seconds until the device code expires.
    pub expires_in: u64,
    /// Minimum polling interval in seconds.
    pub interval: u64,
}

/// Parse a standard OAuth token response JSON into an [`OAuthToken`].
///
/// Handles `access_token` (required), `refresh_token` (optional), and
/// `expires_in` (optional).
///
/// # Errors
///
/// Returns an error if the response is missing the `access_token` field.
pub fn parse_token_response(json: &serde_json::Value) -> Result<OAuthToken> {
    let access_token = json
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ByokError::Auth("missing access_token in response".into()))?
        .to_string();

    let mut token = OAuthToken::new(access_token);
    if let Some(r) = json
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
    {
        token = token.with_refresh(r);
    }
    if let Some(exp) = json.get("expires_in").and_then(serde_json::Value::as_u64) {
        token = token.with_expiry(exp);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_token_response_full() {
        let resp = json!({
            "access_token": "at123",
            "refresh_token": "rt456",
            "expires_in": 3600
        });
        let tok = parse_token_response(&resp).unwrap();
        assert_eq!(tok.access_token, "at123");
        assert_eq!(tok.refresh_token, Some("rt456".into()));
        assert!(tok.expires_at.is_some());
    }

    #[test]
    fn test_parse_token_response_minimal() {
        let resp = json!({"access_token": "at_only"});
        let tok = parse_token_response(&resp).unwrap();
        assert_eq!(tok.access_token, "at_only");
        assert_eq!(tok.refresh_token, None);
        assert!(tok.expires_at.is_none());
    }

    #[test]
    fn test_parse_token_response_missing_access_token() {
        let resp = json!({"refresh_token": "rt"});
        assert!(parse_token_response(&resp).is_err());
    }

    #[test]
    fn test_parse_token_response_empty() {
        assert!(parse_token_response(&json!({})).is_err());
    }
}
