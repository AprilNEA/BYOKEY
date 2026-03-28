//! HTTP proxy layer — axum router, route handlers, and error mapping.
//!
//! ## Module layout
//!
//! - [`handler`]  — HTTP route handlers (API, Amp, management).
//! - [`router`]   — Axum router construction and route registration.
//! - [`error`]    — [`ApiError`] type for OpenAI-compatible error responses.
//! - [`openapi`]  — `OpenAPI` specification generation.
//! - [`usage`]    — In-memory request/token usage tracking.

pub mod error;
pub mod handler;
#[allow(clippy::needless_for_each)]
pub mod openapi;
pub mod router;
pub mod usage;

pub use error::ApiError;
pub use openapi::ApiDoc;
pub use router::make_router;
pub use usage::{UsageRecorder, UsageStats};

use arc_swap::ArcSwap;
use byokey_auth::AuthManager;
use byokey_types::{RateLimitStore, UsageStore};
use std::sync::Arc;

/// Shared application state passed to all route handlers.
pub struct AppState {
    /// Server configuration (providers, listen address, etc.).
    /// Atomically swappable for hot-reloading.
    pub config: Arc<ArcSwap<byokey_config::Config>>,
    /// Token manager for OAuth-based providers.
    pub auth: Arc<AuthManager>,
    /// HTTP client for upstream requests.
    pub http: rquest::Client,
    /// In-memory usage statistics with optional persistent backing.
    pub usage: Arc<UsageRecorder>,
    /// Per-provider, per-account rate limit snapshots from upstream responses.
    pub ratelimits: Arc<RateLimitStore>,
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
    ) -> Arc<Self> {
        let snapshot = config.load();
        let http = build_http_client(snapshot.proxy_url.as_deref());
        Arc::new(Self {
            config,
            auth,
            http,
            usage: Arc::new(UsageRecorder::new(usage_store)),
            ratelimits: Arc::new(RateLimitStore::new()),
        })
    }
}

/// Build an HTTP client, optionally configured with a proxy URL.
fn build_http_client(proxy_url: Option<&str>) -> rquest::Client {
    if let Some(url) = proxy_url {
        match rquest::Proxy::all(url) {
            Ok(proxy) => {
                return rquest::Client::builder()
                    .proxy(proxy)
                    .build()
                    .unwrap_or_else(|_| rquest::Client::new());
            }
            Err(e) => {
                tracing::warn!(url = url, error = %e, "invalid proxy_url, using direct connection");
            }
        }
    }
    rquest::Client::new()
}
