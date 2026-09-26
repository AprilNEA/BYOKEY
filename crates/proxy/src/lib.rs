//! HTTP proxy layer — axum router, route handlers, and error mapping.
//!
//! ## Module layout
//!
//! - [`handler`]  — HTTP route handlers (Anthropic Messages API, management).
//! - [`router`]   — Axum router construction and route registration.
//! - [`error`]    — [`ApiError`], rendered in the Anthropic error envelope.
//! - [`usage`]    — In-memory request/token usage tracking.

pub mod error;
pub mod handler;
pub mod middleware;
pub mod router;
pub mod usage;
pub(crate) mod util;

pub use error::ApiError;
pub use router::make_router;
pub use usage::{UsageRecorder, UsageStats};

use arc_swap::ArcSwap;
use byokey_auth::AuthManager;
use byokey_provider::{CopilotIdentity, DeviceProfileCache};
use byokey_types::UsageStore;
use std::sync::Arc;

/// Shared application state passed to all route handlers.
pub struct AppState {
    /// Server configuration (providers, listen address, etc.).
    /// Atomically swappable for hot-reloading.
    pub config: Arc<ArcSwap<byokey_config::Config>>,
    /// Token manager for OAuth-based providers.
    pub auth: Arc<AuthManager>,
    /// HTTP client for upstream requests.
    pub http: reqwest::Client,
    /// In-memory usage statistics with optional persistent backing.
    pub usage: Arc<UsageRecorder>,
    /// Per-auth device fingerprint cache for Claude API headers.
    pub device_profiles: Arc<DeviceProfileCache>,
    /// The client Copilot requests present themselves as.
    pub copilot_identity: CopilotIdentity,
}

impl AppState {
    /// Creates a new shared application state wrapped in an `Arc`.
    ///
    /// If the config specifies a `proxy_url`, the HTTP client is built with that proxy.
    /// An optional [`UsageStore`] enables persistent usage tracking.
    pub fn new(
        config: Arc<ArcSwap<byokey_config::Config>>,
        auth: Arc<AuthManager>,
        usage_store: Option<Arc<dyn UsageStore>>,
        copilot_identity: CopilotIdentity,
    ) -> Arc<Self> {
        let snapshot = config.load();
        let http = build_http_client(snapshot.proxy_url.as_deref());
        Arc::new(Self {
            config,
            auth,
            http,
            usage: Arc::new(UsageRecorder::new(usage_store)),
            device_profiles: Arc::new(DeviceProfileCache::new()),
            copilot_identity,
        })
    }
}

/// Build an HTTP client, optionally configured with a proxy URL.
fn build_http_client(proxy_url: Option<&str>) -> reqwest::Client {
    if let Some(url) = proxy_url {
        match reqwest::Proxy::all(url) {
            Ok(proxy) => {
                return reqwest::Client::builder()
                    .proxy(proxy)
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());
            }
            Err(e) => {
                tracing::warn!(url = url, error = %e, "invalid proxy_url, using direct connection");
            }
        }
    }
    reqwest::Client::new()
}
