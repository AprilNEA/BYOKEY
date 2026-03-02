//! Claude OAuth 2.0 PKCE authorization flow.
//!
//! Implements the Authorization Code + PKCE (S256) flow used by the Claude CLI.
//! Callback port: 54545.

use byokey_types::{ByokError, OAuthToken, traits::Result};

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 54545;

/// Claude OAuth authorization endpoint.
pub const AUTH_URL: &str = "https://claude.ai/oauth/authorize";

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &["org:create_api_key", "user:profile", "user:inference"];

// Scope encoding: `:` -> %3A, space -> +
const SCOPE_ENCODED: &str = "org%3Acreate_api_key+user%3Aprofile+user%3Ainference";
const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A54545%2Fcallback";
const REDIRECT_URI: &str = "http://localhost:54545/callback";

/// Generate a PKCE `code_verifier` and `code_challenge` (S256).
#[must_use]
pub fn generate_pkce() -> (String, String) {
    crate::pkce::generate_pkce()
}

/// Build the authorization URL with PKCE parameters.
#[must_use]
pub fn build_auth_url(client_id: &str, code_challenge: &str, state: &str) -> String {
    format!(
        "{AUTH_URL}?client_id={client_id}&code=true&code_challenge={code_challenge}&code_challenge_method=S256&redirect_uri={REDIRECT_URI_ENCODED}&response_type=code&scope={SCOPE_ENCODED}&state={state}",
    )
}

/// Build the JSON body for exchanging an authorization code for an access token.
#[must_use]
pub fn build_token_request(
    client_id: &str,
    code: &str,
    code_verifier: &str,
    state: &str,
) -> serde_json::Value {
    serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": client_id,
        "code": code,
        "redirect_uri": REDIRECT_URI,
        "code_verifier": code_verifier,
        "state": state,
    })
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

    const TEST_CLIENT_ID: &str = "test-claude-client-id";

    #[test]
    fn test_build_auth_url_contains_client_id() {
        let url = build_auth_url(TEST_CLIENT_ID, "challenge123", "state456");
        assert!(url.contains(TEST_CLIENT_ID));
        assert!(url.contains("challenge123"));
        assert!(url.contains("state456"));
        assert!(url.contains("S256"));
    }

    #[test]
    fn test_build_auth_url_contains_port() {
        let url = build_auth_url(TEST_CLIENT_ID, "ch", "st");
        assert!(url.contains(&CALLBACK_PORT.to_string()));
    }

    #[test]
    fn test_build_token_request_fields() {
        let req = build_token_request(TEST_CLIENT_ID, "mycode", "myverifier", "mystate");
        assert_eq!(req["grant_type"], "authorization_code");
        assert_eq!(req["client_id"], TEST_CLIENT_ID);
        assert_eq!(req["code"], "mycode");
        assert_eq!(req["code_verifier"], "myverifier");
        assert_eq!(req["state"], "mystate");
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
    fn test_parse_token_response_missing_access_token() {
        let resp = json!({"refresh_token": "rt"});
        assert!(parse_token_response(&resp).is_err());
    }

    #[test]
    fn test_generate_pkce_different_each_call() {
        let (v1, c1) = generate_pkce();
        let (v2, _) = generate_pkce();
        assert_ne!(
            v1, v2,
            "successive calls should produce different verifiers"
        );
        assert_ne!(v1, c1, "verifier and challenge should differ");
    }
}
