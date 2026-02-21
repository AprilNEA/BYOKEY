//! Remote OAuth app credential loader.
//!
//! Fetches `client_id` / `client_secret` from
//! `https://assets.byokey.io/oauth/{provider}.json` at login time so that no
//! secrets are baked into the binary.

use byokey_types::ByokError;
use serde::Deserialize;

const BASE_URL: &str = "https://assets.byokey.io/oauth";

/// Minimal credential fields returned by the remote JSON files.
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthCredentials {
    /// OAuth 2.0 client ID.
    pub client_id: String,
    /// OAuth 2.0 client secret (absent for public clients).
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Fetch credentials for `provider_name` (e.g. `"gemini"`, `"antigravity"`).
///
/// # Errors
///
/// Returns [`ByokError::Auth`] if the request fails or the JSON cannot be parsed.
pub async fn fetch(
    provider_name: &str,
    http: &rquest::Client,
) -> Result<OAuthCredentials, ByokError> {
    let url = format!("{BASE_URL}/{provider_name}.json");

    let resp = http.get(&url).send().await.map_err(|e| {
        ByokError::Auth(format!(
            "failed to fetch credentials for {provider_name}: {e}"
        ))
    })?;

    if !resp.status().is_success() {
        return Err(ByokError::Auth(format!(
            "credentials endpoint returned {} for {provider_name}",
            resp.status()
        )));
    }

    resp.json::<OAuthCredentials>().await.map_err(|e| {
        ByokError::Auth(format!(
            "failed to parse credentials for {provider_name}: {e}"
        ))
    })
}
