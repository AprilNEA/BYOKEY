//! Read OAuth credentials from a locally-installed `OpenAI` Codex CLI.
//!
//! Codex CLI persists its login token at `~/.codex/auth.json` on every
//! platform — unlike Claude Code, it does not use the macOS Keychain.
//!
//! The JSON shape is (as of Codex CLI 0.x with ChatGPT-mode auth):
//!
//! ```json
//! {
//!   "auth_mode": "chatgpt",
//!   "OPENAI_API_KEY": null,
//!   "tokens": {
//!     "id_token": "eyJ…",
//!     "access_token": "eyJ…",
//!     "refresh_token": "rt_…",
//!     "account_id": "…"
//!   },
//!   "last_refresh": "2026-…"
//! }
//! ```
//!
//! In API-key mode, `tokens` is absent and `OPENAI_API_KEY` carries the raw
//! key. That mode is **not** importable: byokey's Codex executor is wired
//! to the ChatGPT-mode Codex Responses endpoint, where a raw `sk-...` won't
//! authenticate. Users with an API-key-mode auth.json should use
//! `byokey add-api-key codex <key>` against the standard OpenAI API instead.

use base64::Engine;
use byokey_types::{ByokError, OAuthToken};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AuthFile {
    tokens: Option<Tokens>,
}

#[derive(Debug, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
}

/// Read and parse the local Codex CLI credentials.
///
/// Returns `Ok(None)` if no credentials are present (not an error — just
/// means Codex CLI isn't logged in on this machine).
///
/// # Errors
///
/// Returns an error if credentials exist but can't be parsed.
pub async fn load_token() -> Result<Option<OAuthToken>, ByokError> {
    let Some(raw) = load_raw().await? else {
        return Ok(None);
    };
    let auth: AuthFile = serde_json::from_str(&raw)
        .map_err(|e| ByokError::Auth(format!("failed to parse Codex CLI credentials JSON: {e}")))?;

    let Some(t) = auth.tokens else {
        return Err(ByokError::Auth(
            "Codex CLI is in API-key mode (~/.codex/auth.json has no `tokens` block). \
             byokey's Codex executor targets the ChatGPT-mode Codex Responses endpoint \
             and cannot authenticate with a raw OpenAI API key. \
             Use `byokey add-api-key codex <sk-...>` if you want a static key."
                .into(),
        ));
    };

    // ChatGPT-mode OAuth token: access_token is a JWT — decode `exp` so
    // the AuthManager knows when to refresh.
    let expires_at = decode_jwt_exp(&t.access_token);
    Ok(Some(OAuthToken {
        access_token: t.access_token,
        refresh_token: t.refresh_token,
        expires_at,
        token_type: Some("Bearer".to_string()),
    }))
}

/// Decode the `exp` claim from a JWT access token. Best-effort: returns
/// `None` if the token isn't a JWT or the payload can't be parsed.
fn decode_jwt_exp(jwt: &str) -> Option<u64> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("exp").and_then(serde_json::Value::as_u64)
}

async fn load_raw() -> Result<Option<String>, ByokError> {
    let home = std::env::var("HOME")
        .map_err(|_| ByokError::Auth("HOME environment variable not set".into()))?;
    let path = std::path::PathBuf::from(home).join(".codex/auth.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ByokError::Auth(format!(
            "failed to read {}: {e}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chatgpt_mode_oauth() {
        // JWT payload `{"exp":9999999999}` URL-safe base64-encoded.
        let payload_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":9999999999}"#);
        let fake_jwt = format!("h.{payload_b64}.s");
        let raw = format!(
            r#"{{
                "auth_mode": "chatgpt",
                "OPENAI_API_KEY": null,
                "tokens": {{
                    "id_token": "id-tok",
                    "access_token": "{fake_jwt}",
                    "refresh_token": "rt-abc",
                    "account_id": "acct-xyz"
                }},
                "last_refresh": "2026-01-01T00:00:00Z"
            }}"#
        );
        let auth: AuthFile = serde_json::from_str(&raw).unwrap();
        let t = auth.tokens.unwrap();
        assert_eq!(t.access_token, fake_jwt);
        assert_eq!(t.refresh_token.as_deref(), Some("rt-abc"));
        assert_eq!(decode_jwt_exp(&fake_jwt), Some(9_999_999_999));
    }

    #[tokio::test]
    async fn rejects_api_key_mode_with_clear_error() {
        // load_token reads the actual file at ~/.codex/auth.json, so we test
        // the parsing path through serde directly: an api_key-mode file has
        // no `tokens` block, which we explicitly reject in load_token via
        // the `let Some(t) = auth.tokens else { return Err(...) }` branch.
        let raw = r#"{
            "auth_mode": "api_key",
            "OPENAI_API_KEY": "sk-foo",
            "tokens": null
        }"#;
        let auth: AuthFile = serde_json::from_str(raw).unwrap();
        assert!(
            auth.tokens.is_none(),
            "api_key mode must surface as `tokens: None`"
        );
    }

    #[test]
    fn jwt_decode_returns_none_for_non_jwt() {
        assert_eq!(decode_jwt_exp("not-a-jwt"), None);
        assert_eq!(decode_jwt_exp(""), None);
    }
}
