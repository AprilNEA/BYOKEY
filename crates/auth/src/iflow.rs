//! iFlow platform (Z.ai / GLM) Authorization Code OAuth flow.
//!
//! Token exchange uses an HTTP Basic Auth header.
//! Callback port: 11451.
//! Provides access to GLM and Kimi K2 models.
//!
//! Credentials (`client_id` / `client_secret`) are fetched at login time from
//! `https://assets.byokey.io/oauth/iflow.json`.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use byokey_types::{ByokError, OAuthToken, traits::Result};

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 11451;
/// iFlow authorization endpoint.
pub const AUTH_URL: &str = "https://iflow.cn/oauth";
/// iFlow token endpoint.
pub const TOKEN_URL: &str = "https://iflow.cn/oauth/token";
const REDIRECT_URI: &str = "http://localhost:11451/callback";
const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A11451%2Fcallback";

/// Build the authorization URL.
#[must_use]
pub fn build_auth_url(client_id: &str, state: &str) -> String {
    format!(
        "{AUTH_URL}?response_type=code&client_id={client_id}&redirect_uri={REDIRECT_URI_ENCODED}&state={state}&loginMethod=phone&type=phone",
    )
}

/// Generate the HTTP Basic Auth header value.
///
/// Format: `Basic base64(client_id:client_secret)`.
#[must_use]
pub fn basic_auth_header(client_id: &str, client_secret: &str) -> String {
    let cred = format!("{client_id}:{client_secret}");
    format!("Basic {}", STANDARD.encode(cred.as_bytes()))
}

/// Build form parameters for the token exchange request.
///
/// Note: `client_secret` is sent via the Basic Auth header, not in the form body.
#[must_use]
pub fn token_form_params(client_id: &str, code: &str) -> Vec<(String, String)> {
    vec![
        ("grant_type".into(), "authorization_code".into()),
        ("client_id".into(), client_id.into()),
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

/// iFlow `userInfo` endpoint.
pub const USER_INFO_URL: &str = "https://iflow.cn/api/oauth/getUserInfo";

/// Fetch the iFlow API key by exchanging an OAuth access token via the `userInfo` endpoint.
///
/// # Errors
///
/// Returns [`ByokError::Http`] on network failure or [`ByokError::Auth`] if the
/// response is missing the `apiKey` field.
pub async fn fetch_api_key(oauth_token: &str, http: &rquest::Client) -> Result<String> {
    let url = format!("{USER_INFO_URL}?accessToken={oauth_token}");
    let resp = http
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await?;

    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("iflow userinfo parse error: {e}")))?;

    if !status.is_success() {
        return Err(ByokError::Auth(format!("iflow userinfo {status}")));
    }

    json.pointer("/data/apiKey")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| ByokError::Auth("iflow: missing apiKey in userinfo response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEST_CLIENT_ID: &str = "test-client-id";
    const TEST_CLIENT_SECRET: &str = "test-client-secret";

    #[test]
    fn test_build_auth_url() {
        let url = build_auth_url(TEST_CLIENT_ID, "mystate");
        assert!(url.contains(TEST_CLIENT_ID));
        assert!(url.contains("mystate"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains(REDIRECT_URI_ENCODED));
        assert!(url.contains("loginMethod=phone"));
    }

    #[test]
    fn test_basic_auth_header_format() {
        let header = basic_auth_header(TEST_CLIENT_ID, TEST_CLIENT_SECRET);
        assert!(header.starts_with("Basic "));
        let encoded = header.strip_prefix("Basic ").unwrap();
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, format!("{TEST_CLIENT_ID}:{TEST_CLIENT_SECRET}"));
    }

    #[test]
    fn test_token_form_params() {
        let params = token_form_params(TEST_CLIENT_ID, "mycode");
        let map: std::collections::HashMap<&str, &str> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(map["grant_type"], "authorization_code");
        assert_eq!(map["client_id"], TEST_CLIENT_ID);
        assert_eq!(map["code"], "mycode");
        assert_eq!(map["redirect_uri"], REDIRECT_URI);
    }

    #[test]
    fn test_parse_token_response_ok() {
        let resp = json!({"access_token": "tok", "expires_in": 7200});
        let t = parse_token_response(&resp).unwrap();
        assert_eq!(t.access_token, "tok");
    }

    #[test]
    fn test_parse_token_response_missing() {
        assert!(parse_token_response(&json!({})).is_err());
    }
}
