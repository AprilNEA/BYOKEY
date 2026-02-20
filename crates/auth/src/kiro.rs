//! Kiro Device Code + Authorization Code flow.
//!
//! Auth endpoint: `prod.us-east-1.auth.desktop.kiro.dev`.
//! Callback port: 9876.
use byok_types::{ByokError, OAuthToken, traits::Result};

pub const CALLBACK_PORT: u16 = 9876;
pub const AUTH_HOST: &str = "prod.us-east-1.auth.desktop.kiro.dev";

/// Device code response.
#[derive(Debug)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// # Errors
///
/// Returns an error if the response is missing required fields (`device_code` or `user_code`).
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
            .unwrap_or("")
            .to_string(),
        expires_in: json
            .get("expires_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(300),
        interval: json
            .get("interval")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5),
    })
}

/// # Errors
///
/// Returns an error if the response is missing the `access_token` field.
pub fn parse_token_response(json: &serde_json::Value) -> Result<OAuthToken> {
    let access_token = json
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ByokError::Auth("missing access_token".into()))?
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
    fn test_parse_device_code() {
        let resp = json!({
            "device_code": "dc123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://kiro.dev/activate",
            "expires_in": 300,
            "interval": 5
        });
        let dc = parse_device_code_response(&resp).unwrap();
        assert_eq!(dc.device_code, "dc123");
        assert_eq!(dc.user_code, "ABCD-1234");
        assert_eq!(dc.expires_in, 300);
        assert_eq!(dc.interval, 5);
    }

    #[test]
    fn test_parse_device_code_missing_field() {
        assert!(parse_device_code_response(&json!({"user_code": "x"})).is_err());
    }

    #[test]
    fn test_parse_token() {
        let resp = json!({"access_token": "kt", "refresh_token": "kr", "expires_in": 3600});
        let t = parse_token_response(&resp).unwrap();
        assert_eq!(t.access_token, "kt");
        assert_eq!(t.refresh_token, Some("kr".into()));
    }
}
