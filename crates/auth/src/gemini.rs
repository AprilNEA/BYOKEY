//! Google Gemini OAuth 2.0 authorization flow.
//!
//! Uses Google's OAuth 2.0 endpoint with PKCE (S256) and offline access.
//! Callback port: 8085.

use byok_types::{ByokError, OAuthToken, traits::Result};

/// OAuth 2.0 client ID for Gemini.
pub const CLIENT_ID: &str =
    "REDACTED_GEMINI_CLIENT_ID";

/// OAuth 2.0 client secret for Gemini.
pub const CLIENT_SECRET: &str = "REDACTED_GEMINI_CLIENT_SECRET";

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 8085;

/// Google OAuth 2.0 authorization endpoint.
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Google OAuth 2.0 token endpoint.
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &[
    "openid",
    "email",
    "https://www.googleapis.com/auth/generative-language.retriever",
];
const REDIRECT_URI: &str = "http://localhost:8085/callback";
const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A8085%2Fcallback";

/// Build the authorization URL with PKCE S256 parameters.
#[must_use]
pub fn build_auth_url(code_challenge: &str, state: &str) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        AUTH_URL,
        CLIENT_ID,
        REDIRECT_URI_ENCODED,
        SCOPES.join("%20"),
        state,
        code_challenge,
    )
}

/// Build the form parameters for the token exchange request.
#[must_use]
pub fn token_form_params(code: &str, code_verifier: &str) -> Vec<(String, String)> {
    vec![
        ("grant_type".into(), "authorization_code".into()),
        ("client_id".into(), CLIENT_ID.into()),
        ("client_secret".into(), CLIENT_SECRET.into()),
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
    fn test_auth_url_contains_required_params() {
        let url = build_auth_url("challenge123", "state456");
        assert!(url.contains(CLIENT_ID));
        assert!(url.contains("challenge123"));
        assert!(url.contains("state456"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains(REDIRECT_URI_ENCODED));
    }

    #[test]
    fn test_auth_url_contains_port() {
        let url = build_auth_url("ch", "st");
        assert!(url.contains(&CALLBACK_PORT.to_string()));
    }

    #[test]
    fn test_token_form_params_fields() {
        let params = token_form_params("mycode", "myverifier");
        assert_eq!(params.len(), 6);

        let map: std::collections::HashMap<&str, &str> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(map["grant_type"], "authorization_code");
        assert_eq!(map["client_id"], CLIENT_ID);
        assert_eq!(map["client_secret"], CLIENT_SECRET);
        assert_eq!(map["code"], "mycode");
        assert_eq!(map["redirect_uri"], REDIRECT_URI);
        assert_eq!(map["code_verifier"], "myverifier");
    }

    #[test]
    fn test_parse_ok() {
        let resp = json!({"access_token": "ga", "refresh_token": "gr", "expires_in": 3600});
        let t = parse_token_response(&resp).unwrap();
        assert_eq!(t.access_token, "ga");
        assert_eq!(t.refresh_token, Some("gr".into()));
    }
}
