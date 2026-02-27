//! Factory (Droid) device code authorization flow via `WorkOS`.
//!
//! Implements the OAuth 2.0 Device Authorization Grant used by Factory.
//! After initial login, an org-scoped token is obtained via `/api/cli/org`
//! resolution and token refresh with the `organization_id`.

use byokey_types::{ByokError, OAuthToken, traits::Result};

/// `WorkOS` device code request endpoint.
pub const DEVICE_CODE_URL: &str = "https://api.workos.com/user_management/authorize/device";

/// `WorkOS` token exchange / refresh endpoint.
pub const TOKEN_URL: &str = "https://api.workos.com/user_management/authenticate";

/// Public OAuth client ID for Factory CLI.
pub const CLIENT_ID: &str = "client_01HNM792M5G5G1A2THWPXKFMXB";

/// Factory API base URL for org resolution.
pub const FACTORY_API_BASE: &str = "https://api.factory.ai";

/// Parsed response from the device code request.
#[derive(Debug)]
pub struct DeviceCodeResponse {
    /// Unique device verification code.
    pub device_code: String,
    /// Short code the user enters at the verification URI.
    pub user_code: String,
    /// Full URL including the user code for one-click authorization.
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
            .get("verification_uri_complete")
            .or_else(|| json.get("verification_uri"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("https://api.workos.com/user_management/authorize/device")
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

/// Parse the token endpoint JSON response into an [`OAuthToken`].
///
/// Factory tokens are JWTs with an `exp` claim. The expiry is extracted
/// from the JWT payload to enable automatic refresh.
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

    let mut token = OAuthToken::new(access_token.clone());

    if let Some(rt) = json
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
    {
        token = token.with_refresh(rt);
    }

    // Try to extract expiry from JWT `exp` claim.
    if let Some(exp) = decode_jwt_exp(&access_token) {
        token.expires_at = Some(exp);
    } else if let Some(exp) = json.get("expires_in").and_then(serde_json::Value::as_u64) {
        token = token.with_expiry(exp);
    }

    Ok(token)
}

/// Resolve the user's `WorkOS` organization ID from Factory.
///
/// # Errors
///
/// Returns an error if the API call fails or no organization is found.
pub async fn resolve_org(access_token: &str, http: &rquest::Client) -> Result<String> {
    let resp = http
        .get(format!("{FACTORY_API_BASE}/api/cli/org"))
        .header("authorization", format!("Bearer {access_token}"))
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ByokError::Auth(format!(
            "Factory org resolution failed: {text}"
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse org response: {e}")))?;

    json.get("workosOrgIds")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(serde_json::Value::as_str)
        .map(String::from)
        .ok_or_else(|| {
            ByokError::Auth(
                "no organization found; visit https://app.factory.ai/cli-onboarding".into(),
            )
        })
}

/// Decode the `exp` claim from a JWT without verifying the signature.
fn decode_jwt_exp(token: &str) -> Option<u64> {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("exp").and_then(serde_json::Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_device_code() {
        let resp = json!({
            "device_code": "dc-123",
            "user_code": "ABCD-1234",
            "verification_uri_complete": "https://auth.workos.com/device?code=ABCD-1234",
            "expires_in": 300,
            "interval": 5
        });
        let dc = parse_device_code_response(&resp).unwrap();
        assert_eq!(dc.user_code, "ABCD-1234");
        assert_eq!(dc.expires_in, 300);
    }

    #[test]
    fn test_parse_token_ok() {
        let resp = json!({
            "access_token": "eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjk5OTk5OTk5OTl9.signature",
            "refresh_token": "rt_abc"
        });
        let t = parse_token_response(&resp).unwrap();
        assert!(t.access_token.starts_with("eyJ"));
        assert_eq!(t.refresh_token, Some("rt_abc".into()));
    }

    #[test]
    fn test_parse_token_missing() {
        assert!(parse_token_response(&json!({})).is_err());
    }

    #[test]
    fn test_decode_jwt_exp() {
        // {"exp": 9999999999} base64url-encoded
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjk5OTk5OTk5OTl9.signature";
        assert_eq!(decode_jwt_exp(token), Some(9999999999));
    }

    #[test]
    fn test_decode_jwt_exp_invalid() {
        assert_eq!(decode_jwt_exp("not-a-jwt"), None);
        assert_eq!(decode_jwt_exp("a.b"), None);
    }
}
