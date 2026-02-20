//! GitHub Copilot device code authorization flow.
//!
//! Implements the OAuth 2.0 Device Authorization Grant used by GitHub Copilot.
//! No local callback port is needed for this flow.

use byok_types::{ByokError, OAuthToken, traits::Result};

/// GitHub device code request endpoint.
pub const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";

/// GitHub OAuth token endpoint.
pub const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// Public OAuth client ID for GitHub Copilot.
pub const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &["read:user"];

/// Parsed response from the device code request.
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

/// Parse the token endpoint JSON response into an [`OAuthToken`].
///
/// GitHub may return form-encoded or JSON responses; this handles the JSON
/// format. Copilot tokens have no expiration time.
///
/// # Errors
///
/// Returns an error if the response is missing the `access_token` field.
pub fn parse_token_response(json: &serde_json::Value) -> Result<OAuthToken> {
    let access_token = json
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ByokError::Auth("missing access_token".into()))?
        .to_string();

    Ok(OAuthToken::new(access_token))
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
    fn test_parse_token_ok() {
        let resp = json!({"access_token": "ghu_abc"});
        let t = parse_token_response(&resp).unwrap();
        assert_eq!(t.access_token, "ghu_abc");
        assert!(t.expires_at.is_none()); // Copilot tokens do not expire.
    }

    #[test]
    fn test_parse_token_missing() {
        assert!(parse_token_response(&json!({})).is_err());
    }
}
