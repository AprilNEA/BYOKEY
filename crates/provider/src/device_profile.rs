//! Per-scope device fingerprint caching.
//!
//! Prevents fingerprint drift across requests by pinning a [`DeviceProfile`]
//! to a cache key derived from the auth scope. Profiles are created from
//! baseline defaults and cached for `PROFILE_TTL` (7 days).

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Cached profiles expire after 7 days.
// `Duration::from_days` is not yet a const fn on stable.
#[allow(clippy::duration_suboptimal_units)]
const PROFILE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

// ── Baseline defaults ───────────────────────────────────────────────
const DEFAULT_USER_AGENT: &str = "claude-cli/2.1.109 (external, cli)";
const DEFAULT_PACKAGE_VERSION: &str = "0.74.0";
const DEFAULT_RUNTIME_VERSION: &str = "v24.14.1";
const DEFAULT_OS: &str = "MacOS";
const DEFAULT_ARCH: &str = "arm64";

/// Snapshot of a device fingerprint used for Claude API headers.
#[derive(Clone, Debug)]
pub struct DeviceProfile {
    /// Full `User-Agent` string (e.g. `claude-cli/2.1.109 (external, cli)`).
    pub user_agent: String,
    /// `x-stainless-package-version` value.
    pub package_version: String,
    /// `x-stainless-runtime-version` value.
    pub runtime_version: String,
    /// `x-stainless-os` value (always pinned to baseline).
    pub os: String,
    /// `x-stainless-arch` value (always pinned to baseline).
    pub arch: String,
    /// Stable session UUID — one per scope, mimics a CLI process session.
    pub session_id: String,
    /// Stable device ID — 64-char hex, mimics Claude Code's `getOrCreateUserID()`.
    pub device_id: String,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self::baseline()
    }
}

impl DeviceProfile {
    /// Build a profile from baseline defaults with fresh session/device IDs.
    fn baseline() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            package_version: DEFAULT_PACKAGE_VERSION.to_string(),
            runtime_version: DEFAULT_RUNTIME_VERSION.to_string(),
            os: DEFAULT_OS.to_string(),
            arch: DEFAULT_ARCH.to_string(),
            session_id: Uuid::new_v4().to_string(),
            device_id: hex::encode(rand::random::<[u8; 32]>()),
        }
    }
}

/// A single entry in the profile cache.
struct CachedEntry {
    profile: DeviceProfile,
    created: Instant,
}

/// Thread-safe, TTL-aware device-profile cache keyed by SHA-256 of the scope.
pub struct DeviceProfileCache {
    inner: Mutex<HashMap<String, CachedEntry>>,
}

impl DeviceProfileCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Return a cached profile for `scope_key`, creating one from baseline
    /// defaults if none exists or the TTL has elapsed.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn resolve(&self, scope_key: &str) -> DeviceProfile {
        let hashed = hash_key(scope_key);
        let mut map = self.inner.lock().expect("profile cache lock poisoned");

        if let Some(entry) = map.get(&hashed)
            && entry.created.elapsed() < PROFILE_TTL
        {
            return entry.profile.clone();
        }

        let profile = DeviceProfile::baseline();
        map.insert(
            hashed,
            CachedEntry {
                profile: profile.clone(),
                created: Instant::now(),
            },
        );
        profile
    }
}

impl Default for DeviceProfileCache {
    fn default() -> Self {
        Self::new()
    }
}

/// SHA-256 hex digest of a scope key.
fn hash_key(scope_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope_key.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_profile_has_expected_values() {
        let p = DeviceProfile::baseline();
        assert_eq!(p.user_agent, DEFAULT_USER_AGENT);
        assert_eq!(p.os, DEFAULT_OS);
        assert_eq!(p.arch, DEFAULT_ARCH);
        assert!(!p.session_id.is_empty());
        assert_eq!(p.device_id.len(), 64);
    }

    #[test]
    fn resolve_returns_stable_profile() {
        let cache = DeviceProfileCache::new();
        let a = cache.resolve("key-1");
        let b = cache.resolve("key-1");
        assert_eq!(a.user_agent, b.user_agent);
    }

    #[test]
    fn session_id_is_stable_within_cache() {
        let cache = DeviceProfileCache::new();
        let a = cache.resolve("k");
        let b = cache.resolve("k");
        assert_eq!(a.session_id, b.session_id);
        assert_eq!(a.device_id, b.device_id);
    }

    #[test]
    fn different_scopes_get_different_ids() {
        let cache = DeviceProfileCache::new();
        let a = cache.resolve("scope-a");
        let b = cache.resolve("scope-b");
        assert_ne!(a.session_id, b.session_id);
        assert_ne!(a.device_id, b.device_id);
    }

    #[test]
    fn hash_key_is_deterministic() {
        assert_eq!(hash_key("hello"), hash_key("hello"));
        assert_ne!(hash_key("hello"), hash_key("world"));
    }
}
