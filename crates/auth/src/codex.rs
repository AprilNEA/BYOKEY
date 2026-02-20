//! `OpenAI` Codex CLI JWT OAuth authorization flow.
//!
//! Implements the Authorization Code + PKCE (S256) flow extracted from the
//! Codex CLI binary. Callback port: 1455.

use byokey_types::{ByokError, OAuthToken, traits::Result};

/// OAuth 2.0 client ID for Codex CLI.
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 1455;

/// `OpenAI` OAuth authorization endpoint.
pub const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";

/// `OpenAI` OAuth token endpoint.
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &["openid", "email", "profile", "offline_access"];

const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

/// Build the authorization URL with PKCE parameters.
#[must_use]
pub fn build_auth_url(code_challenge: &str, state: &str) -> String {
    let scope = SCOPES.join("+");
    format!(
        "{AUTH_URL}?client_id={CLIENT_ID}&code_challenge={code_challenge}&code_challenge_method=S256&codex_cli_simplified_flow=true&id_token_add_organizations=true&prompt=login&redirect_uri={REDIRECT_URI_ENCODED}&response_type=code&scope={scope}&state={state}",
    )
}

/// Build the form-urlencoded parameters for the token exchange request.
#[must_use]
pub fn token_form_params<'a>(
    code: &'a str,
    code_verifier: &'a str,
) -> [(&'static str, &'a str); 5] {
    [
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", code),
        ("redirect_uri", REDIRECT_URI),
        ("code_verifier", code_verifier),
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
    fn test_auth_url_contains_client_id() {
        let url = build_auth_url("mychallenge", "mystate");
        assert!(url.contains(CLIENT_ID));
        assert!(url.contains("mychallenge"));
        assert!(url.contains("mystate"));
        assert!(url.contains(&CALLBACK_PORT.to_string()));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("S256"));
    }

    #[test]
    fn test_parse_ok() {
        let resp = json!({"access_token": "tok", "expires_in": 7200});
        let t = parse_token_response(&resp).unwrap();
        assert_eq!(t.access_token, "tok");
        assert!(t.expires_at.is_some());
    }

    #[test]
    fn test_parse_missing() {
        assert!(parse_token_response(&json!({})).is_err());
    }
}
