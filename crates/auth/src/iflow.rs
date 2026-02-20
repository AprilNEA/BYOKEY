//! iFlow platform (Z.ai / GLM) Authorization Code OAuth flow.
//!
//! Token exchange uses an HTTP Basic Auth header.
//! Callback port: 11451.
//! Provides access to GLM and Kimi K2 models.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use byok_types::{ByokError, OAuthToken, traits::Result};

pub const CLIENT_ID: &str = "10009311001";
pub const CLIENT_SECRET: &str = "4Z3YjXycVsQvyGF1etiNlIBB4RsqSDtW";
pub const CALLBACK_PORT: u16 = 11451;
pub const AUTH_URL: &str = "https://iflow.cn/oauth";
pub const TOKEN_URL: &str = "https://iflow.cn/oauth/token";
const REDIRECT_URI: &str = "http://localhost:11451/callback";
const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A11451%2Fcallback";

/// Build the authorization URL.
#[must_use]
pub fn build_auth_url(state: &str) -> String {
    format!(
        "{AUTH_URL}?response_type=code&client_id={CLIENT_ID}&redirect_uri={REDIRECT_URI_ENCODED}&state={state}&loginMethod=phone&type=phone",
    )
}

/// Generate the HTTP Basic Auth header value.
///
/// Format: `Basic base64(client_id:client_secret)`.
#[must_use]
pub fn basic_auth_header() -> String {
    let cred = format!("{CLIENT_ID}:{CLIENT_SECRET}");
    format!("Basic {}", STANDARD.encode(cred.as_bytes()))
}

/// Build form parameters for the token exchange request.
///
/// Note: `client_secret` is sent via the Basic Auth header, not in the form body.
#[must_use]
pub fn token_form_params(code: &str) -> Vec<(String, String)> {
    vec![
        ("grant_type".into(), "authorization_code".into()),
        ("client_id".into(), CLIENT_ID.into()),
        ("code".into(), code.into()),
        ("redirect_uri".into(), REDIRECT_URI.into()),
    ]
}

/// Parse the token endpoint response into an [`OAuthToken`].
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
    fn test_build_auth_url() {
        let url = build_auth_url("mystate");
        assert!(url.contains(CLIENT_ID));
        assert!(url.contains("mystate"));
        assert!(url.contains("loginMethod=phone"));
        assert!(url.contains("type=phone"));
        assert!(url.contains(&CALLBACK_PORT.to_string()));
    }

    #[test]
    fn test_basic_auth_header() {
        let h = basic_auth_header();
        assert!(h.starts_with("Basic "));
        // Decode and verify
        let encoded = h.strip_prefix("Basic ").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let s = String::from_utf8(decoded).unwrap();
        assert_eq!(s, format!("{CLIENT_ID}:{CLIENT_SECRET}"));
    }

    #[test]
    fn test_token_form_params() {
        let params = token_form_params("mycode");
        let map: std::collections::HashMap<_, _> = params.into_iter().collect();
        assert_eq!(map["grant_type"], "authorization_code");
        assert_eq!(map["client_id"], CLIENT_ID);
        assert_eq!(map["code"], "mycode");
        assert!(map.contains_key("redirect_uri"));
    }

    #[test]
    fn test_parse_token_response_ok() {
        let resp = json!({
            "access_token": "iflow_token",
            "refresh_token": "iflow_refresh",
            "expires_in": 86400
        });
        let t = parse_token_response(&resp).unwrap();
        assert_eq!(t.access_token, "iflow_token");
        assert_eq!(t.refresh_token, Some("iflow_refresh".into()));
        assert!(t.expires_at.is_some());
    }

    #[test]
    fn test_parse_token_response_missing() {
        assert!(parse_token_response(&json!({})).is_err());
    }
}
