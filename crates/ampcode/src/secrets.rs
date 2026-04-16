//! Read bearer tokens from the native Amp CLI secrets file.
//!
//! The Amp CLI stores credentials at `~/.local/share/amp/secrets.json`
//! as a flat key-value map:
//!
//! ```json
//! {
//!   "apiKey@https://ampcode.com/": "sgamp_user_...",
//!   "apiKey@http://localhost:8317": "sgamp_user_..."
//! }
//! ```

use crate::error::{AmpcodeError, Result};
use std::collections::HashMap;
use std::path::PathBuf;

/// The key prefix for API key entries in the secrets file.
const API_KEY_PREFIX: &str = "apiKey@";

/// The canonical Ampcode host URL used as the default lookup key.
const AMPCODE_HOST: &str = "https://ampcode.com/";

/// Resolve the Amp secrets file path.
///
/// Uses `$HOME` — falls back to `/tmp` if `HOME` is unset.
#[must_use]
pub fn secrets_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/share/amp/secrets.json")
}

/// Load the bearer token for `ampcode.com` from the native secrets file.
///
/// Looks for the key `"apiKey@https://ampcode.com/"` in the flat map.
///
/// # Errors
///
/// Returns [`AmpcodeError::NoToken`] if the file exists but does not contain
/// a token for `ampcode.com`. Returns [`AmpcodeError::Io`] if the file is
/// absent or unreadable.
pub async fn load_token() -> Result<String> {
    load_token_from(&secrets_path()).await
}

/// Load the bearer token from a specific file path.
///
/// Use this when the secrets file is not at the default location.
/// Prefer [`load_token`] for the standard path.
///
/// # Errors
///
/// Returns [`AmpcodeError::Io`] if the file is absent or unreadable,
/// [`AmpcodeError::Json`] on malformed JSON, or [`AmpcodeError::NoToken`]
/// if the file lacks an `ampcode.com` entry.
pub async fn load_token_from(path: &std::path::Path) -> Result<String> {
    let content = tokio::fs::read_to_string(path).await?;
    extract_token(&content)
}

/// Extract the `ampcode.com` token from the raw JSON content.
///
/// The secrets file is a flat `HashMap<String, Value>`. API key entries
/// use the format `"apiKey@<host>"` → `"sgamp_user_..."`. We look for
/// `"apiKey@https://ampcode.com/"` first, then fall back to any key
/// containing `"ampcode.com"`.
fn extract_token(content: &str) -> Result<String> {
    let map: HashMap<String, serde_json::Value> = serde_json::from_str(content)?;

    // Primary: exact key "apiKey@https://ampcode.com/"
    let key = format!("{API_KEY_PREFIX}{AMPCODE_HOST}");
    if let Some(val) = map.get(&key)
        && let Some(tok) = val.as_str().filter(|s| !s.is_empty())
    {
        return Ok(tok.to_string());
    }

    // Fallback: any apiKey@ entry whose key contains "ampcode.com"
    for (k, v) in &map {
        if k.starts_with(API_KEY_PREFIX)
            && k.contains("ampcode.com")
            && let Some(tok) = v.as_str().filter(|s| !s.is_empty())
        {
            return Ok(tok.to_string());
        }
    }

    Err(AmpcodeError::NoToken)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_canonical_key() {
        let json = r#"{"apiKey@https://ampcode.com/":"sgamp_user_abc123"}"#;
        let token = extract_token(json).unwrap();
        assert_eq!(token, "sgamp_user_abc123");
    }

    #[test]
    fn extract_fallback_key() {
        let json = r#"{"apiKey@http://localhost:8317":"local_tok","apiKey@https://ampcode.com/other":"sgamp_user_fallback"}"#;
        let token = extract_token(json).unwrap();
        assert_eq!(token, "sgamp_user_fallback");
    }

    #[test]
    fn extract_with_other_entries() {
        let json = r#"{
            "apiKey@http://localhost:8317": "local_tok",
            "apiKey@https://ampcode.com/": "sgamp_user_real",
            "mcp-oauth-token@https://mcp.linear.app/sse#linear": "{}"
        }"#;
        let token = extract_token(json).unwrap();
        assert_eq!(token, "sgamp_user_real");
    }

    #[test]
    fn extract_no_ampcode_key() {
        let json = r#"{"apiKey@http://localhost:8317":"local_tok"}"#;
        let err = extract_token(json).unwrap_err();
        assert!(matches!(err, AmpcodeError::NoToken));
    }

    #[test]
    fn extract_empty_map() {
        let err = extract_token("{}").unwrap_err();
        assert!(matches!(err, AmpcodeError::NoToken));
    }

    #[test]
    fn extract_invalid_json() {
        let err = extract_token("not json").unwrap_err();
        assert!(matches!(err, AmpcodeError::Json(_)));
    }

    #[tokio::test]
    async fn load_from_nonexistent_file() {
        let err = load_token_from(std::path::Path::new("/tmp/nonexistent_ampcode_test.json"))
            .await
            .unwrap_err();
        assert!(matches!(err, AmpcodeError::Io(_)));
    }
}
