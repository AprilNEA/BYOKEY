//! Google Cloud Code Assistant (Antigravity) OAuth 2.0 PKCE authorization flow.
//!
//! Uses Google's OAuth 2.0 endpoint with PKCE (S256) and offline access.
//! Callback port: 51121.
//!
//! Credentials (`client_id` / `client_secret`) are fetched at login time from
//! `https://assets.byokey.io/oauth/antigravity.json`.

use byok_types::{ByokError, OAuthToken, traits::Result};

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 51121;

/// Google OAuth 2.0 authorization endpoint.
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Google OAuth 2.0 token endpoint.
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
];
const REDIRECT_URI: &str = "http://localhost:51121/callback";
const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A51121%2Fcallback";

/// Build the authorization URL with PKCE S256 parameters.
#[must_use]
pub fn build_auth_url(client_id: &str, code_challenge: &str, state: &str) -> String {
    let scope = SCOPES.join("%20");
    format!(
        "{AUTH_URL}?response_type=code&client_id={client_id}&redirect_uri={REDIRECT_URI_ENCODED}&scope={scope}&state={state}&code_challenge={code_challenge}&code_challenge_method=S256&access_type=offline&prompt=consent",
    )
}

/// Build the form parameters for the token exchange request.
#[must_use]
pub fn token_form_params(
    client_id: &str,
    client_secret: &str,
    code: &str,
    code_verifier: &str,
) -> Vec<(String, String)> {
    vec![
        ("grant_type".into(), "authorization_code".into()),
        ("client_id".into(), client_id.into()),
        ("client_secret".into(), client_secret.into()),
        ("code".into(), code.into()),
        ("redirect_uri".into(), REDIRECT_URI.into()),
        ("code_verifier".into(), code_verifier.into()),
    ]
}

/// Parse the token endpoint JSON response into an [`OAuthToken`].
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

    const TEST_CLIENT_ID: &str = "test-client-id.apps.googleusercontent.com";
    const TEST_CLIENT_SECRET: &str = "test-client-secret";

    #[test]
    fn test_build_auth_url_contains_required_params() {
        let url = build_auth_url(TEST_CLIENT_ID, "challenge123", "state456");
        assert!(url.contains(TEST_CLIENT_ID));
        assert!(url.contains("challenge123"));
        assert!(url.contains("state456"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains(REDIRECT_URI_ENCODED));
    }

    #[test]
    fn test_build_auth_url_scopes_encoded() {
        let url = build_auth_url(TEST_CLIENT_ID, "ch", "st");
        for scope in SCOPES {
            assert!(url.contains(scope), "URL should contain scope: {scope}");
        }
        assert!(url.contains("%20"));
    }

    #[test]
    fn test_token_form_params_fields() {
        let params = token_form_params(TEST_CLIENT_ID, TEST_CLIENT_SECRET, "mycode", "myverifier");
        assert_eq!(params.len(), 6);

        let map: std::collections::HashMap<&str, &str> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(map["grant_type"], "authorization_code");
        assert_eq!(map["client_id"], TEST_CLIENT_ID);
        assert_eq!(map["client_secret"], TEST_CLIENT_SECRET);
        assert_eq!(map["code"], "mycode");
        assert_eq!(map["redirect_uri"], REDIRECT_URI);
        assert_eq!(map["code_verifier"], "myverifier");
    }

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
    fn test_constants() {
        assert_eq!(CALLBACK_PORT, 51121);
        assert_eq!(AUTH_URL, "https://accounts.google.com/o/oauth2/v2/auth");
        assert_eq!(TOKEN_URL, "https://oauth2.googleapis.com/token");
    }
}
