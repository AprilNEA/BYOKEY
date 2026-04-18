//! Read OAuth credentials from a locally-installed Claude Code CLI.
//!
//! Claude Code persists its login token in platform-native storage:
//! - **macOS**: Keychain, service `Claude Code-credentials`, account = `$USER`.
//! - **Linux / WSL / Windows fallback**: plain-text file at
//!   `~/.claude/.credentials.json`.
//!
//! The JSON shape is (as of Claude Code 2.x):
//!
//! ```json
//! {
//!   "claudeAiOauth": {
//!     "accessToken": "sk-ant-oat01-…",
//!     "refreshToken": "sk-ant-ort01-…",
//!     "expiresAt": 1712345678000,
//!     "scopes": ["user:inference", "user:profile"],
//!     "subscriptionType": "max"
//!   }
//! }
//! ```
//!
//! We convert it to an [`OAuthToken`] and let the caller persist it through
//! the usual [`TokenStore`](byokey_types::traits::TokenStore) path.

use byokey_types::{ByokError, OAuthToken};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Secrets {
    #[serde(rename = "claudeAiOauth")]
    oauth: OAuthBlock,
}

#[derive(Debug, Deserialize)]
struct OAuthBlock {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    /// Milliseconds since the Unix epoch (Claude Code uses JS `Date.now()`).
    #[serde(rename = "expiresAt")]
    expires_at_ms: Option<u64>,
    #[serde(rename = "subscriptionType")]
    #[allow(dead_code)]
    subscription_type: Option<String>,
}

/// Read and parse the local Claude Code credentials.
///
/// Returns `Ok(None)` if no credentials are present (not an error — just
/// means Claude Code isn't logged in on this machine).
///
/// # Errors
///
/// Returns an error if credentials exist but can't be parsed.
pub async fn load_token() -> Result<Option<OAuthToken>, ByokError> {
    let Some(raw) = load_raw().await? else {
        return Ok(None);
    };
    let secrets: Secrets = serde_json::from_str(&raw).map_err(|e| {
        ByokError::Auth(format!("failed to parse Claude Code credentials JSON: {e}"))
    })?;
    Ok(Some(OAuthToken {
        access_token: secrets.oauth.access_token,
        refresh_token: secrets.oauth.refresh_token,
        expires_at: secrets.oauth.expires_at_ms.map(|ms| ms / 1000),
        token_type: Some("Bearer".to_string()),
    }))
}

/// Load raw credential JSON from the platform-specific source.
#[cfg(target_os = "macos")]
async fn load_raw() -> Result<Option<String>, ByokError> {
    // Shell out to the system `security` CLI rather than pulling in a
    // security-framework crate dependency.
    let user = std::env::var("USER")
        .map_err(|_| ByokError::Auth("USER environment variable not set".into()))?;
    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-a",
            &user,
            "-w",
        ])
        .output()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to spawn security: {e}")))?;
    match output.status.code() {
        Some(0) => {}                // fall through
        Some(44) => return Ok(None), // errSecItemNotFound
        Some(code) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ByokError::Auth(format!(
                "security failed (exit {code}): {}",
                stderr.trim()
            )));
        }
        None => {
            return Err(ByokError::Auth("security terminated by signal".into()));
        }
    }
    let raw = String::from_utf8(output.stdout)
        .map_err(|e| ByokError::Auth(format!("security returned non-UTF8: {e}")))?;
    Ok(Some(raw.trim().to_string()))
}

#[cfg(not(target_os = "macos"))]
async fn load_raw() -> Result<Option<String>, ByokError> {
    let home = std::env::var("HOME")
        .map_err(|_| ByokError::Auth("HOME environment variable not set".into()))?;
    let path = std::path::PathBuf::from(home).join(".claude/.credentials.json");
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
    fn parses_expected_json_shape() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-xyz",
                "refreshToken": "sk-ant-ort01-abc",
                "expiresAt": 1712345678000,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "max"
            }
        }"#;
        let secrets: Secrets = serde_json::from_str(raw).unwrap();
        assert_eq!(secrets.oauth.access_token, "sk-ant-oat01-xyz");
        assert_eq!(
            secrets.oauth.refresh_token.as_deref(),
            Some("sk-ant-ort01-abc")
        );
        assert_eq!(secrets.oauth.expires_at_ms, Some(1_712_345_678_000));
    }

    #[test]
    fn expires_at_converts_ms_to_secs() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "t",
                "expiresAt": 1712345678000
            }
        }"#;
        let secrets: Secrets = serde_json::from_str(raw).unwrap();
        let token = OAuthToken {
            access_token: secrets.oauth.access_token,
            refresh_token: secrets.oauth.refresh_token,
            expires_at: secrets.oauth.expires_at_ms.map(|ms| ms / 1000),
            token_type: Some("Bearer".to_string()),
        };
        assert_eq!(token.expires_at, Some(1_712_345_678));
    }

    #[test]
    fn parses_without_refresh_token() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "t"
            }
        }"#;
        let secrets: Secrets = serde_json::from_str(raw).unwrap();
        assert!(secrets.oauth.refresh_token.is_none());
        assert!(secrets.oauth.expires_at_ms.is_none());
    }
}
