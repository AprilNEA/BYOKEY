//! Alibaba Cloud Tongyi Qwen Device Code + PKCE flow.
//!
//! The device code request also includes a PKCE `code_challenge`.
//! Slow-down multiplier: 1.5x.
use byokey_types::{ByokError, OAuthToken, traits::Result};

pub const CLIENT_ID: &str = "f0304373b74a44d2b584a3fb70ca9e56";
pub const DEVICE_CODE_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/device/code";
pub const TOKEN_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/token";
pub const SCOPES: &[&str] = &["openid", "profile", "email", "model.completion"];
pub const SLOW_DOWN_MULTIPLIER: f64 = 1.5;

#[derive(Debug)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[must_use]
pub fn build_device_code_params(code_challenge: &str, scope: &str) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), CLIENT_ID.into()),
        ("scope".into(), scope.into()),
        ("code_challenge".into(), code_challenge.into()),
        ("code_challenge_method".into(), "S256".into()),
    ]
}

#[must_use]
pub fn build_token_poll_params(device_code: &str, code_verifier: &str) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), CLIENT_ID.into()),
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

    if let Some(refresh) = json
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
    {
        token = token.with_refresh(refresh);
    }

    if let Some(expires_in) = json.get("expires_in").and_then(serde_json::Value::as_u64) {
        token = token.with_expiry(expires_in);
    }

    Ok(token)
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

    #[test]
    fn test_build_device_code_params() {
        let params = build_device_code_params("challenge123", "openid profile");
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "client_id" && v == CLIENT_ID)
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
        let params = build_token_poll_params("dc-abc", "verifier-xyz");
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "client_id" && v == CLIENT_ID)
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

    #[test]
    fn test_parse_token_response_full() {
        let resp = json!({
            "access_token": "qwen-at-123",
            "refresh_token": "qwen-rt-456",
            "expires_in": 7200
        });
        let token = parse_token_response(&resp).unwrap();
        assert_eq!(token.access_token, "qwen-at-123");
        assert_eq!(token.refresh_token.as_deref(), Some("qwen-rt-456"));
        assert!(token.expires_at.is_some());
    }

    #[test]
    fn test_parse_token_response_minimal() {
        let resp = json!({"access_token": "qwen-at"});
        let token = parse_token_response(&resp).unwrap();
        assert_eq!(token.access_token, "qwen-at");
        assert!(token.refresh_token.is_none());
        assert!(token.expires_at.is_none());
    }

    #[test]
    fn test_parse_token_response_missing() {
        assert!(parse_token_response(&json!({})).is_err());
    }
}
