//! Google Cloud Code Assistant (Antigravity) OAuth 2.0 PKCE authorization flow configuration.
//!
//! Uses Google's OAuth 2.0 endpoint with PKCE (S256) and offline access.
//! Callback port: 51121.

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 51121;

/// Google OAuth 2.0 authorization endpoint.
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

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

// ── AuthCodeFlow implementation ───────────────────────────────────────────────

use async_trait::async_trait;
use byokey_types::{ByokError, OAuthToken, ProviderId, traits::Result};

use crate::credentials::OAuthCredentials;
use crate::flow::auth_code::{self, AuthCodeFlow};

/// Antigravity auth-code provider.
pub struct Antigravity;

#[async_trait]
impl AuthCodeFlow for Antigravity {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Antigravity
    }
    fn provider_name(&self) -> &'static str {
        "antigravity"
    }
    fn callback_port(&self) -> u16 {
        CALLBACK_PORT
    }

    fn build_auth_url(&self, client_id: &str, challenge: &str, state: &str) -> String {
        build_auth_url(client_id, challenge, state)
    }

    async fn exchange_code(
        &self,
        http: &rquest::Client,
        creds: &OAuthCredentials,
        code: &str,
        verifier: &str,
        _state: &str,
    ) -> Result<OAuthToken> {
        let token_url = creds
            .token_url
            .as_deref()
            .ok_or_else(|| ByokError::Auth("antigravity credentials missing token_url".into()))?;
        let client_secret = creds.client_secret.as_deref().ok_or_else(|| {
            ByokError::Auth("antigravity credentials missing client_secret".into())
        })?;
        let params = token_form_params(&creds.client_id, client_secret, code, verifier);
        let resp = http.post(token_url).form(&params).send().await?;
        auth_code::send_and_parse_token(resp).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_constants() {
        assert_eq!(CALLBACK_PORT, 51121);
        assert_eq!(AUTH_URL, "https://accounts.google.com/o/oauth2/v2/auth");
    }
}
