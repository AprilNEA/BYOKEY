//! Shared token response parsing and device code response types.
//!
//! Provides a single [`parse_token_response`] implementation used by all
//! providers and the token refresh path in [`AuthManager`](crate::AuthManager),
//! eliminating the per-provider duplication.

use byokey_types::{ByokError, OAuthToken, Result};

/// Parsed response from an OAuth 2.0 Device Authorization Grant.
///
/// Used by the Copilot device code flow.
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

/// Configuration for parsing device code responses, accounting for provider differences.
pub struct DeviceCodeParseConfig {
    /// Fallback for `verification_uri` if not present. `None` means it's required.
    pub verification_uri_fallback: Option<&'static str>,
    /// Default `expires_in` if not present.
    pub default_expires_in: u64,
}

/// Parse a device code response JSON with provider-specific configuration.
///
/// # Errors
///
/// Returns an error if required fields are missing.
pub fn parse_device_code_json(
    json: &serde_json::Value,
    config: &DeviceCodeParseConfig,
) -> Result<DeviceCodeResponse> {
    let device_code = json
        .get("device_code")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ByokError::Auth("missing device_code".into()))?
        .to_string();
    let user_code = json
        .get("user_code")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ByokError::Auth("missing user_code".into()))?
        .to_string();
    let verification_uri = match json
        .get("verification_uri")
        .and_then(serde_json::Value::as_str)
    {
        Some(uri) => uri.to_string(),
        None => match config.verification_uri_fallback {
            Some(fallback) => fallback.to_string(),
            None => return Err(ByokError::Auth("missing verification_uri".into())),
        },
    };
    let expires_in = json
        .get("expires_in")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(config.default_expires_in);
    let interval = json
        .get("interval")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(5);

    Ok(DeviceCodeResponse {
        device_code,
        user_code,
        verification_uri,
        expires_in,
        interval,
    })
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

    #[test]
    fn test_parse_device_code_json_full() {
        let json = json!({
            "device_code": "dc",
            "user_code": "ABCD",
            "verification_uri": "https://example.com",
            "expires_in": 600,
            "interval": 10
        });
        let config = DeviceCodeParseConfig {
            verification_uri_fallback: None,
            default_expires_in: 900,
        };
        let dc = parse_device_code_json(&json, &config).unwrap();
        assert_eq!(dc.device_code, "dc");
        assert_eq!(dc.user_code, "ABCD");
        assert_eq!(dc.verification_uri, "https://example.com");
        assert_eq!(dc.expires_in, 600);
        assert_eq!(dc.interval, 10);
    }

    #[test]
    fn test_parse_device_code_json_fallback_uri() {
        let json = json!({
            "device_code": "dc",
            "user_code": "ABCD"
        });
        let config = DeviceCodeParseConfig {
            verification_uri_fallback: Some("https://fallback.com"),
            default_expires_in: 300,
        };
        let dc = parse_device_code_json(&json, &config).unwrap();
        assert_eq!(dc.verification_uri, "https://fallback.com");
        assert_eq!(dc.expires_in, 300);
    }

    #[test]
    fn test_parse_device_code_json_required_uri_missing() {
        let json = json!({
            "device_code": "dc",
            "user_code": "ABCD"
        });
        let config = DeviceCodeParseConfig {
            verification_uri_fallback: None,
            default_expires_in: 600,
        };
        assert!(parse_device_code_json(&json, &config).is_err());
    }
}
