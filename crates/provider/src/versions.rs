//! Remote version/fingerprint config fetched from `assets.byokey.io/versions/`.
//!
//! Each provider's CLI version and user-agent string can change with every
//! upstream release. Instead of hardcoding them, we fetch at startup and
//! fall back to compile-time defaults if the network is unreachable.

use byokey_types::ProviderId;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

const BASE_URL: &str = "https://assets.byokey.io/versions";

/// Version/identity info for a single provider.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderVersions {
    /// CLI release version (e.g. `"2.1.109"`, `"0.120.0"`).
    #[serde(default)]
    pub cli_version: Option<String>,
    /// Full User-Agent header value.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Stainless SDK package version (`x-stainless-package-version`).
    #[serde(default)]
    pub stainless_package_version: Option<String>,
    /// Stainless runtime version (`x-stainless-runtime-version`).
    #[serde(default)]
    pub stainless_runtime_version: Option<String>,
    /// Copilot: VS Code editor version.
    #[serde(default)]
    pub editor_version: Option<String>,
    /// Copilot: chat plugin version.
    #[serde(default)]
    pub plugin_version: Option<String>,
    /// Copilot: GitHub API version.
    #[serde(default)]
    pub github_api_version: Option<String>,
}

/// Shared, read-only store of provider version info loaded at startup.
#[derive(Debug, Clone)]
pub struct VersionStore(Arc<HashMap<ProviderId, ProviderVersions>>);

impl VersionStore {
    /// Fetch version info for all known providers.
    ///
    /// Failures are logged and silently skipped — the store will simply
    /// be empty for that provider, and callers fall back to compile-time defaults.
    pub async fn fetch(http: &rquest::Client) -> Self {
        let providers = [
            (ProviderId::Claude, "claude"),
            (ProviderId::Codex, "codex"),
            (ProviderId::Copilot, "copilot"),
            (ProviderId::Antigravity, "antigravity"),
            (ProviderId::Kimi, "kimi"),
            (ProviderId::Qwen, "qwen"),
            (ProviderId::IFlow, "iflow"),
        ];

        let mut map = HashMap::new();
        for (id, name) in providers {
            match fetch_one(http, name).await {
                Ok(v) => {
                    map.insert(id, v);
                }
                Err(e) => {
                    tracing::debug!(provider = name, %e, "failed to fetch version info, using defaults");
                }
            }
        }

        Self(Arc::new(map))
    }

    /// Create an empty store (all providers use compile-time defaults).
    #[must_use]
    pub fn empty() -> Self {
        Self(Arc::new(HashMap::new()))
    }

    /// Look up a provider's version info.
    #[must_use]
    pub fn get(&self, provider: &ProviderId) -> Option<&ProviderVersions> {
        self.0.get(provider)
    }

    /// Get a specific string field with compile-time fallback.
    #[must_use]
    pub fn user_agent(&self, provider: &ProviderId, default: &str) -> String {
        self.get(provider)
            .and_then(|v| v.user_agent.as_deref())
            .unwrap_or(default)
            .to_string()
    }

    /// Get CLI version with fallback.
    #[must_use]
    pub fn cli_version(&self, provider: &ProviderId, default: &str) -> String {
        self.get(provider)
            .and_then(|v| v.cli_version.as_deref())
            .unwrap_or(default)
            .to_string()
    }

    /// Get stainless runtime version with fallback.
    #[must_use]
    pub fn stainless_runtime(&self, provider: &ProviderId, default: &str) -> String {
        self.get(provider)
            .and_then(|v| v.stainless_runtime_version.as_deref())
            .unwrap_or(default)
            .to_string()
    }

    /// Get stainless package version with fallback.
    #[must_use]
    pub fn stainless_package(&self, provider: &ProviderId, default: &str) -> String {
        self.get(provider)
            .and_then(|v| v.stainless_package_version.as_deref())
            .unwrap_or(default)
            .to_string()
    }
}

async fn fetch_one(http: &rquest::Client, provider_name: &str) -> Result<ProviderVersions, String> {
    let url = format!("{BASE_URL}/{provider_name}.json");
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<ProviderVersions>()
        .await
        .map_err(|e| format!("parse failed: {e}"))
}
