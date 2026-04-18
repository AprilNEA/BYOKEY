//! Sentry telemetry initialization.
//!
//! BYOKEY is an AI API proxy: request bodies contain user prompts. We
//! configure Sentry to avoid leaking that data:
//!
//! - `send_default_pii: false` — strips the hardcoded sensitive header
//!   blocklist (`Authorization`, `Cookie`, `X-Api-Key`, etc.)
//! - `before_send` — clears the URL query string, strips additional
//!   provider-specific auth headers not in the default blocklist, and
//!   truncates long messages.
//! - `ByokError::Upstream`'s `Display` impl omits the response body, so
//!   `tracing::error!("{err}")` never forwards a potentially-sensitive
//!   body into Sentry.

use byokey_config::TelemetryConfig;
use sentry::ClientInitGuard;
use sentry::protocol::{Breadcrumb, Event, Request};
use std::sync::Arc;

/// Provider-specific auth headers that aren't in Sentry's default
/// sensitive-header blocklist. Matched case-insensitively.
const EXTRA_SENSITIVE_HEADERS: &[&str] = &[
    "x-goog-api-key",
    "x-amp-token",
    "api-key",
    "anthropic-version",
    "openai-organization",
    "openai-project",
    "x-session-id",
];

/// Maximum length for event `message` / exception `value` fields. Longer
/// strings (often serialized upstream error bodies) are truncated.
const MAX_MESSAGE_LEN: usize = 1024;

/// Compile-time default DSN, baked in by CI for release builds.
/// `None` when the `BYOKEY_SENTRY_DSN` env var was unset during `cargo build`.
const COMPILE_TIME_DSN: Option<&str> = option_env!("BYOKEY_SENTRY_DSN");

/// Initializes Sentry if a DSN is configured and the user hasn't opted out.
///
/// DSN resolution order:
/// 1. `SENTRY_DSN` environment variable
/// 2. `telemetry.sentry_dsn` from config
/// 3. Compile-time `BYOKEY_SENTRY_DSN` (baked in by CI)
///
/// Returns `None` if `telemetry.disabled` is true or no DSN is found.
#[must_use]
pub fn init(cfg: &TelemetryConfig) -> Option<ClientInitGuard> {
    if cfg.disabled {
        return None;
    }

    let dsn = std::env::var("SENTRY_DSN")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.sentry_dsn.clone().filter(|s| !s.is_empty()))
        .or_else(|| COMPILE_TIME_DSN.filter(|s| !s.is_empty()).map(String::from))?;

    let release = cfg
        .release
        .clone()
        .unwrap_or_else(|| format!("byokey@{}", env!("CARGO_PKG_VERSION")));
    let environment = cfg.environment.clone().unwrap_or_else(|| {
        if cfg!(debug_assertions) {
            "development".to_string()
        } else {
            "production".to_string()
        }
    });

    let options = sentry::ClientOptions {
        dsn: dsn.parse().ok(),
        release: Some(release.into()),
        environment: Some(environment.into()),
        sample_rate: cfg.sample_rate.clamp(0.0, 1.0),
        // AI proxy: never send PII.
        send_default_pii: false,
        // Disable performance tracing — spans could capture upstream
        // provider calls containing user prompts.
        traces_sample_rate: 0.0,
        attach_stacktrace: true,
        before_send: Some(Arc::new(|mut event| {
            scrub_event(&mut event);
            Some(event)
        })),
        before_breadcrumb: Some(Arc::new(|mut crumb| {
            scrub_breadcrumb(&mut crumb);
            Some(crumb)
        })),
        ..Default::default()
    };

    let guard = sentry::init(options);
    Some(guard)
}

fn scrub_event(event: &mut Event<'static>) {
    if let Some(req) = event.request.as_mut() {
        scrub_request(req);
    }
    truncate(&mut event.message, MAX_MESSAGE_LEN);
    for exc in &mut event.exception.values {
        truncate(&mut exc.value, MAX_MESSAGE_LEN);
    }
}

fn scrub_breadcrumb(crumb: &mut Breadcrumb) {
    truncate(&mut crumb.message, MAX_MESSAGE_LEN);
    // Strip URL query strings from HTTP breadcrumbs.
    if let Some(url) = crumb.data.get_mut("url")
        && let Some(s) = url.as_str()
        && let Some((base, _)) = s.split_once('?')
    {
        *url = serde_json::Value::String(base.to_string());
    }
}

fn scrub_request(req: &mut Request) {
    // Strip query string from URL.
    if let Some(url) = req.url.as_mut()
        && url.query().is_some()
    {
        url.set_query(None);
    }
    req.query_string = None;
    // Remove provider-specific auth headers not in Sentry's default blocklist.
    req.headers
        .retain(|name, _| !is_extra_sensitive(name.as_str()));
    // `sentry-tower` doesn't populate `data`, but scrub defensively: bodies
    // in an AI proxy are always user prompts.
    req.data = None;
    req.cookies = None;
}

fn is_extra_sensitive(header: &str) -> bool {
    EXTRA_SENSITIVE_HEADERS
        .iter()
        .any(|s| s.eq_ignore_ascii_case(header))
}

fn truncate(s: &mut Option<String>, max: usize) {
    if let Some(v) = s.as_mut()
        && v.len() > max
    {
        v.truncate(max);
        v.push_str("…[truncated]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_extra_sensitive_matches_case_insensitively() {
        assert!(is_extra_sensitive("X-Goog-Api-Key"));
        assert!(is_extra_sensitive("x-goog-api-key"));
        assert!(is_extra_sensitive("Anthropic-Version"));
        assert!(!is_extra_sensitive("content-type"));
        assert!(!is_extra_sensitive("user-agent"));
    }

    #[test]
    fn truncate_shortens_long_strings() {
        let mut s = Some("x".repeat(2000));
        truncate(&mut s, 10);
        let v = s.unwrap();
        assert!(v.starts_with("xxxxxxxxxx"));
        assert!(v.ends_with("[truncated]"));
    }

    #[test]
    fn truncate_leaves_short_strings() {
        let mut s = Some("hello".to_string());
        truncate(&mut s, 100);
        assert_eq!(s.as_deref(), Some("hello"));
    }

    #[test]
    fn scrub_request_removes_query_string() {
        let mut req = Request {
            url: Some(
                "https://example.com/v1/chat?api_key=secret&model=gpt-4"
                    .parse()
                    .unwrap(),
            ),
            query_string: Some("api_key=secret&model=gpt-4".to_string()),
            ..Default::default()
        };
        scrub_request(&mut req);
        assert_eq!(
            req.url.as_ref().unwrap().as_str(),
            "https://example.com/v1/chat"
        );
        assert!(req.query_string.is_none());
    }

    #[test]
    fn scrub_request_removes_extra_sensitive_headers() {
        let mut req = Request::default();
        req.headers
            .insert("x-goog-api-key".to_string(), "secret".to_string());
        req.headers
            .insert("user-agent".to_string(), "ua".to_string());
        scrub_request(&mut req);
        assert!(!req.headers.contains_key("x-goog-api-key"));
        assert!(req.headers.contains_key("user-agent"));
    }

    #[test]
    fn scrub_request_clears_body_data() {
        let mut req = Request {
            data: Some("{\"prompt\": \"secret\"}".to_string()),
            cookies: Some("session=abc".to_string()),
            ..Default::default()
        };
        scrub_request(&mut req);
        assert!(req.data.is_none());
        assert!(req.cookies.is_none());
    }

    #[test]
    fn scrub_breadcrumb_strips_url_query() {
        let mut crumb = Breadcrumb::default();
        crumb.data.insert(
            "url".to_string(),
            serde_json::Value::String("https://example.com/x?k=v".to_string()),
        );
        scrub_breadcrumb(&mut crumb);
        assert_eq!(
            crumb.data.get("url").and_then(|v| v.as_str()),
            Some("https://example.com/x")
        );
    }
}
