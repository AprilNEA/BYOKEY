//! Claude OAuth 2.0 PKCE authorization flow configuration.
//!
//! Implements the Authorization Code + PKCE (S256) flow used by the Claude CLI.
//! Callback port: 54545.

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
