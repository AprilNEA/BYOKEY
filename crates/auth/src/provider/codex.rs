//! `OpenAI` Codex CLI JWT OAuth authorization flow configuration.
//!
//! Implements the Authorization Code + PKCE (S256) flow extracted from the
//! Codex CLI binary. Callback port: 1455.

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 1455;

/// `OpenAI` OAuth authorization endpoint.
pub const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &["openid", "email", "profile", "offline_access"];

const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

/// Build the authorization URL with PKCE parameters.
#[must_use]
pub fn build_auth_url(client_id: &str, code_challenge: &str, state: &str) -> String {
    let scope = SCOPES.join("+");
    format!(
        "{AUTH_URL}?client_id={client_id}&code_challenge={code_challenge}&code_challenge_method=S256&codex_cli_simplified_flow=true&id_token_add_organizations=true&prompt=login&redirect_uri={REDIRECT_URI_ENCODED}&response_type=code&scope={scope}&state={state}",
    )
}

/// Build the form-urlencoded parameters for the token exchange request.
#[must_use]
pub fn token_form_params<'a>(
    client_id: &'a str,
    code: &'a str,
    code_verifier: &'a str,
) -> [(&'static str, &'a str); 5] {
    [
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("redirect_uri", REDIRECT_URI),
        ("code_verifier", code_verifier),
    ]
}

// ── AuthCodeFlow implementation ───────────────────────────────────────────────

use async_trait::async_trait;
use byokey_types::{ByokError, OAuthToken, ProviderId, traits::Result};

use crate::credentials::OAuthCredentials;
use crate::flow::auth_code::{self, AuthCodeFlow};

/// Codex auth-code provider.
pub struct Codex;

#[async_trait]
impl AuthCodeFlow for Codex {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Codex
    }
    fn provider_name(&self) -> &'static str {
        "codex"
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
            .ok_or_else(|| ByokError::Auth("codex credentials missing token_url".into()))?;
        let params = token_form_params(&creds.client_id, code, verifier);
        let resp = http
            .post(token_url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await?;
        auth_code::send_and_parse_token(resp).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CLIENT_ID: &str = "test-codex-client-id";

    #[test]
    fn test_auth_url_contains_client_id() {
        let url = build_auth_url(TEST_CLIENT_ID, "mychallenge", "mystate");
        assert!(url.contains(TEST_CLIENT_ID));
        assert!(url.contains("mychallenge"));
        assert!(url.contains("mystate"));
        assert!(url.contains(&CALLBACK_PORT.to_string()));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("S256"));
    }
}
