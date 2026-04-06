//! Per-auth device fingerprint caching.
//!
//! Prevents fingerprint drift across requests by pinning a [`DeviceProfile`]
//! to a cache-key derived from the auth scope (e.g. API key or account id).
//! Profiles are lazily created from baseline defaults and cached for
//! [`PROFILE_TTL`] (7 days).  An incoming user-agent with a *newer* semver
//! version will upgrade the cached profile; downgrades are rejected.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cached profiles expire after 7 days.
const PROFILE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

// ── Baseline defaults ───────────────────────────────────────────────
const DEFAULT_USER_AGENT: &str = "claude-cli/2.1.63 (external, sdk-cli)";
const DEFAULT_PACKAGE_VERSION: &str = "0.74.0";
const DEFAULT_RUNTIME_VERSION: &str = "v22.13.1";
const DEFAULT_OS: &str = "MacOS";
const DEFAULT_ARCH: &str = "arm64";

/// Snapshot of a device fingerprint used for Claude API headers.
#[derive(Clone, Debug)]
pub struct DeviceProfile {
    /// Full `User-Agent` string (e.g. `claude-cli/2.1.63 (external, sdk-cli)`).
    pub user_agent: String,
    /// `x-stainless-package-version` value.
    pub package_version: String,
    /// `x-stainless-runtime-version` value.
    pub runtime_version: String,
    /// `x-stainless-os` value (always pinned to baseline).
    pub os: String,
    /// `x-stainless-arch` value (always pinned to baseline).
    pub arch: String,
    /// Parsed `(major, minor, patch)` from the user-agent string, if available.
    version: Option<(u32, u32, u32)>,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self::baseline()
    }
}

impl DeviceProfile {
    /// Build a profile from baseline defaults.
    fn baseline() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            package_version: DEFAULT_PACKAGE_VERSION.to_string(),
            runtime_version: DEFAULT_RUNTIME_VERSION.to_string(),
            os: DEFAULT_OS.to_string(),
            arch: DEFAULT_ARCH.to_string(),
            version: parse_version(DEFAULT_USER_AGENT),
        }
    }

    /// Build a profile from an incoming user-agent string, keeping OS/Arch
    /// pinned to baseline.
    fn from_ua(ua: &str) -> Self {
        Self {
            user_agent: ua.to_string(),
            package_version: DEFAULT_PACKAGE_VERSION.to_string(),
            runtime_version: DEFAULT_RUNTIME_VERSION.to_string(),
            os: DEFAULT_OS.to_string(),
            arch: DEFAULT_ARCH.to_string(),
            version: parse_version(ua),
        }
    }
}

/// Extract `(major, minor, patch)` from a user-agent like `claude-cli/2.1.63`.
///
/// Looks for the first `/` and splits the segment up to the next space or
/// end-of-string on `.` to get three numeric components.
fn parse_version(ua: &str) -> Option<(u32, u32, u32)> {
    let after_slash = ua.split('/').nth(1)?;
    // Take only the version segment (stop at first space or paren).
    let ver_str = after_slash.split([' ', '(']).next()?;
    let mut parts = ver_str.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
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

    /// Like [`resolve`](Self::resolve), but if `incoming_ua` carries a *newer*
    /// version than the cached profile, upgrade in-place (never downgrade).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn resolve_or_upgrade(&self, scope_key: &str, incoming_ua: Option<&str>) -> DeviceProfile {
        let hashed = hash_key(scope_key);
        let mut map = self.inner.lock().expect("profile cache lock poisoned");

        let now = Instant::now();

        // Fetch or create a baseline entry.
        let entry = map.entry(hashed).or_insert_with(|| CachedEntry {
            profile: DeviceProfile::baseline(),
            created: now,
        });

        // Expired — reset to baseline.
        if entry.created.elapsed() >= PROFILE_TTL {
            entry.profile = DeviceProfile::baseline();
            entry.created = now;
        }

        // Attempt upgrade from incoming UA.
        if let Some(ua) = incoming_ua {
            let incoming_ver = parse_version(ua);
            if let (Some(new), Some(old)) = (incoming_ver, entry.profile.version) {
                if new > old {
                    entry.profile = DeviceProfile::from_ua(ua);
                    entry.created = now;
                }
            } else if incoming_ver.is_some() && entry.profile.version.is_none() {
                // No version cached yet — accept the incoming one.
                entry.profile = DeviceProfile::from_ua(ua);
                entry.created = now;
            }
        }

        entry.profile.clone()
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
        assert_eq!(p.version, Some((2, 1, 63)));
    }

    #[test]
    fn parse_version_happy_path() {
        assert_eq!(
            parse_version("claude-cli/2.1.63 (external, sdk-cli)"),
            Some((2, 1, 63))
        );
    }

    #[test]
    fn parse_version_no_slash() {
        assert_eq!(parse_version("no-version-here"), None);
    }

    #[test]
    fn parse_version_partial() {
        assert_eq!(parse_version("cli/1.2"), None); // only 2 parts
    }

    #[test]
    fn resolve_returns_stable_profile() {
        let cache = DeviceProfileCache::new();
        let a = cache.resolve("key-1");
        let b = cache.resolve("key-1");
        assert_eq!(a.user_agent, b.user_agent);
    }

    #[test]
    fn resolve_or_upgrade_upgrades_version() {
        let cache = DeviceProfileCache::new();
        let _ = cache.resolve("k");
        let upgraded = cache.resolve_or_upgrade("k", Some("claude-cli/3.0.0 (external, sdk-cli)"));
        assert_eq!(upgraded.version, Some((3, 0, 0)));
        assert!(upgraded.user_agent.contains("3.0.0"));
    }

    #[test]
    fn resolve_or_upgrade_does_not_downgrade() {
        let cache = DeviceProfileCache::new();
        let _ = cache.resolve("k");
        let after = cache.resolve_or_upgrade("k", Some("claude-cli/1.0.0 (external, sdk-cli)"));
        // Should still be baseline 2.1.63
        assert_eq!(after.version, Some((2, 1, 63)));
    }

    #[test]
    fn different_scope_keys_get_independent_profiles() {
        let cache = DeviceProfileCache::new();
        let _ = cache.resolve_or_upgrade("a", Some("claude-cli/9.9.9 (external, sdk-cli)"));
        let b = cache.resolve("b");
        // "b" should still be baseline
        assert_eq!(b.version, Some((2, 1, 63)));
    }

    #[test]
    fn hash_key_is_deterministic() {
        assert_eq!(hash_key("hello"), hash_key("hello"));
        assert_ne!(hash_key("hello"), hash_key("world"));
    }
}
