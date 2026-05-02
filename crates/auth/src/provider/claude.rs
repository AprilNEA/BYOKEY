//! Claude OAuth 2.0 PKCE authorization flow configuration.
//!
//! Implements the Authorization Code + PKCE (S256) flow used by the Claude CLI.
//! Callback port: 54545.

/// Local callback port for the OAuth redirect.
pub const CALLBACK_PORT: u16 = 54545;

/// Claude OAuth authorization endpoint.
pub const AUTH_URL: &str = "https://claude.ai/oauth/authorize";

/// OAuth scopes requested during authorization.
pub const SCOPES: &[&str] = &[
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];

// Scope encoding: `:` -> %3A, space -> +
const SCOPE_ENCODED: &str = "user%3Aprofile+user%3Ainference+user%3Asessions%3Aclaude_code+user%3Amcp_servers+user%3Afile_upload";
const REDIRECT_URI_ENCODED: &str = "http%3A%2F%2Flocalhost%3A54545%2Fcallback";
const REDIRECT_URI: &str = "http://localhost:54545/callback";

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

// ── AuthCodeFlow implementation ───────────────────────────────────────────────

use async_trait::async_trait;
use byokey_types::{ByokError, OAuthToken, ProviderId, Result};

use crate::credentials::OAuthCredentials;
use crate::flow::auth_code::{self, AuthCodeFlow};

/// Claude auth-code provider.
pub struct Claude;

#[async_trait]
impl AuthCodeFlow for Claude {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Claude
    }
    fn provider_name(&self) -> &'static str {
        "claude"
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
        state: &str,
    ) -> Result<OAuthToken> {
        let token_url = creds
            .token_url
            .as_deref()
            .ok_or_else(|| ByokError::Auth("claude credentials missing token_url".into()))?;
        let body = build_token_request(&creds.client_id, code, verifier, state);
        let resp = http
            .post(token_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        auth_code::send_and_parse_token(resp).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
