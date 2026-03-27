//! GitHub Copilot device code authorization flow configuration.
//!
//! Implements the OAuth 2.0 Device Authorization Grant used by GitHub Copilot.
//! No local callback port is needed for this flow.

use crate::token::DeviceCodeResponse;
use byokey_types::{ByokError, traits::Result};

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
