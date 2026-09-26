//! Headers for requests to the Anthropic API.
//!
//! BYOKEY forwards Anthropic Messages requests as a Claude Code CLI would
//! send them: the API version and betas Claude Code enables, and the
//! device fingerprint headers its SDK attaches.

use crate::device_profile::DeviceProfile;
use http::header::{HeaderMap, HeaderValue, USER_AGENT};

/// Required Anthropic API version header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Beta features to enable.
///
/// `prompt-caching-2024-07-31` is absent: prompt caching is GA since Dec 2024
/// and the API rejects the beta.
pub const ANTHROPIC_BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,redact-thinking-2026-02-12,context-management-2025-06-27,prompt-caching-scope-2026-01-05,advanced-tool-use-2025-11-20,effort-2025-11-24,structured-outputs-2025-12-15,fast-mode-2026-02-01,token-efficient-tools-2026-03-28";

/// The headers a Claude Code CLI sends besides authentication: the device
/// fingerprint (`x-stainless-*`, `user-agent`), session identity
/// (`x-claude-code-session-id`), request identity (`x-client-request-id`)
/// and the `x-app` flag.
///
/// `anthropic-dangerous-direct-browser-access` is sent only with a raw API
/// key (`is_api_key`); the CLI omits it on OAuth.
///
/// # Panics
///
/// Panics if a profile field is not a valid header value. All default
/// profiles are ASCII-only.
#[must_use]
pub fn fingerprint_headers(profile: &DeviceProfile, is_api_key: bool) -> HeaderMap {
    let value = |s: &str| HeaderValue::from_str(s).expect("ASCII header value");
    let mut h = HeaderMap::new();
    if is_api_key {
        h.insert(
            "anthropic-dangerous-direct-browser-access",
            HeaderValue::from_static("true"),
        );
    }
    h.insert("x-app", HeaderValue::from_static("cli"));
    h.insert(USER_AGENT, value(&profile.user_agent));
    // One session id per CLI process, stable across requests.
    h.insert("x-claude-code-session-id", value(&profile.session_id));
    h.insert("x-stainless-lang", HeaderValue::from_static("js"));
    h.insert("x-stainless-runtime", HeaderValue::from_static("node"));
    h.insert(
        "x-stainless-runtime-version",
        value(&profile.runtime_version),
    );
    h.insert(
        "x-stainless-package-version",
        value(&profile.package_version),
    );
    h.insert("x-stainless-os", value(&profile.os));
    h.insert("x-stainless-arch", value(&profile.arch));
    h.insert("x-stainless-retry-count", HeaderValue::from_static("0"));
    h.insert("x-stainless-timeout", HeaderValue::from_static("600"));
    h.insert(
        "x-client-request-id",
        value(&uuid::Uuid::new_v4().to_string()),
    );
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_access_header_only_with_an_api_key() {
        let profile = DeviceProfile::default();
        let oauth = fingerprint_headers(&profile, false);
        assert!(!oauth.contains_key("anthropic-dangerous-direct-browser-access"));
        assert_eq!(oauth["x-claude-code-session-id"], profile.session_id);
        assert_eq!(oauth[USER_AGENT], profile.user_agent);
        let api_key = fingerprint_headers(&profile, true);
        assert_eq!(api_key["anthropic-dangerous-direct-browser-access"], "true");
    }
}
