//! Moonshot AI (Kimi) Device Code flow.
//!
//! Requires additional `X-Msh-*` request headers for platform information.
use byokey_types::{ByokError, OAuthToken, traits::Result};
use rand::RngCore as _;

pub const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub const DEVICE_CODE_URL: &str = "https://auth.kimi.com/api/oauth/device_authorization";
pub const TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";
pub const SCOPES: &[&str] = &["openid", "offline_access"];
pub const PLATFORM: &str = "mac";
pub const VERSION: &str = "0.13.0";
pub const DEVICE_MODEL: &str = "MacBookPro";

#[derive(Debug)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[must_use]
pub fn device_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    // Set UUID version 4 and variant bits
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

#[must_use]
pub fn device_name() -> String {
    "byok-client".to_string()
}

#[must_use]
pub fn x_msh_headers() -> Vec<(&'static str, String)> {
    vec![
        ("X-Msh-Platform", PLATFORM.to_string()),
        ("X-Msh-Version", VERSION.to_string()),
        ("X-Msh-Device-Name", device_name()),
        ("X-Msh-Device-Model", DEVICE_MODEL.to_string()),
        ("X-Msh-Device-Id", device_id()),
    ]
}

#[must_use]
pub fn build_device_code_params(scope: &str) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), CLIENT_ID.into()),
        ("scope".into(), scope.into()),
    ]
}

#[must_use]
pub fn build_token_poll_params(device_code: &str) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), CLIENT_ID.into()),
        ("device_code".into(), device_code.into()),
        (
            "grant_type".into(),
            "urn:ietf:params:oauth:grant-type:device_code".into(),
        ),
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
    fn test_device_id_format() {
        let id = device_id();
        // UUID v4 format: 8-4-4-4-12 hex chars
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        // version 4: third segment starts with '4'
        assert!(parts[2].starts_with('4'));
        // variant: first char of fourth segment is 8/9/a/b
        let variant_char = parts[3].chars().next().unwrap();
        assert!(
            "89ab".contains(variant_char),
            "variant char should be 8/9/a/b, got {variant_char}"
        );
    }

    #[test]
    fn test_device_id_unique() {
        let id1 = device_id();
        let id2 = device_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_device_name() {
        assert_eq!(device_name(), "byok-client");
    }

    #[test]
    fn test_x_msh_headers() {
        let headers = x_msh_headers();
        assert_eq!(headers.len(), 5);
        assert_eq!(headers[0].0, "X-Msh-Platform");
        assert_eq!(headers[0].1, "mac");
        assert_eq!(headers[1].0, "X-Msh-Version");
        assert_eq!(headers[1].1, "0.13.0");
        assert_eq!(headers[2].0, "X-Msh-Device-Name");
        assert_eq!(headers[2].1, "byok-client");
        assert_eq!(headers[3].0, "X-Msh-Device-Model");
        assert_eq!(headers[3].1, "MacBookPro");
        assert_eq!(headers[4].0, "X-Msh-Device-Id");
        // device id is random, only check format
        assert_eq!(headers[4].1.len(), 36);
    }

    #[test]
    fn test_build_device_code_params() {
        let params = build_device_code_params("openid offline_access");
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "client_id" && v == CLIENT_ID)
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "scope" && v == "openid offline_access")
        );
    }

    #[test]
    fn test_build_token_poll_params() {
        let params = build_token_poll_params("kimi-dc-abc");
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "client_id" && v == CLIENT_ID)
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "device_code" && v == "kimi-dc-abc")
        );
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "grant_type"
                    && v == "urn:ietf:params:oauth:grant-type:device_code")
        );
    }

    #[test]
    fn test_parse_device_code_success() {
        let resp = json!({
            "device_code": "kimi-dc-123",
            "user_code": "KIMI-5678",
            "verification_uri": "https://kimi.moonshot.cn/device",
            "expires_in": 1800,
            "interval": 5
        });
        let dc = parse_device_code_response(&resp).unwrap();
        assert_eq!(dc.device_code, "kimi-dc-123");
        assert_eq!(dc.user_code, "KIMI-5678");
        assert_eq!(dc.verification_uri, "https://kimi.moonshot.cn/device");
        assert_eq!(dc.expires_in, 1800);
        assert_eq!(dc.interval, 5);
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
    fn test_parse_token_response_full() {
        let resp = json!({
            "access_token": "kimi-at-123",
            "refresh_token": "kimi-rt-456",
            "expires_in": 3600
        });
        let token = parse_token_response(&resp).unwrap();
        assert_eq!(token.access_token, "kimi-at-123");
        assert_eq!(token.refresh_token.as_deref(), Some("kimi-rt-456"));
        assert!(token.expires_at.is_some());
    }

    #[test]
    fn test_parse_token_response_minimal() {
        let resp = json!({"access_token": "kimi-at"});
        let token = parse_token_response(&resp).unwrap();
        assert_eq!(token.access_token, "kimi-at");
        assert!(token.refresh_token.is_none());
        assert!(token.expires_at.is_none());
    }

    #[test]
    fn test_parse_token_response_missing() {
        assert!(parse_token_response(&json!({})).is_err());
    }
}
