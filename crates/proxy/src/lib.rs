//! HTTP proxy layer — axum router, route handlers, and error mapping.
//!
//! ## Module layout
//!
//! - [`handler`]  — HTTP route handlers (Anthropic Messages API, management).
//! - [`router`]   — Axum router construction and route registration.
//! - [`error`]    — [`ApiError`], rendered in the Anthropic error envelope.
//! - `exchange`   — One upstream exchange, logged once when it ends.
//! - [`http`]     — The upstream HTTP client and its connection probing.
//! - [`usage`]    — In-memory request/token usage tracking.

pub mod error;
pub(crate) mod exchange;
pub mod handler;
pub mod http;
pub mod middleware;
pub mod router;
#[cfg(test)]
mod test_logs;
pub mod usage;
pub(crate) mod util;

pub use error::ApiError;
pub use http::upstream_client;
pub use router::make_router;
pub use usage::{UsageRecorder, UsageStats};

use arc_swap::ArcSwap;
use byokey_auth::AuthManager;
use byokey_provider::{CopilotIdentity, DeviceProfileCache};
use byokey_types::UsageStore;
use std::{sync::Arc, time::Duration};

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
    /// The client Copilot requests present themselves as: the compile-time
    /// versions until the published ones arrive (see
    /// [`AppState::spawn_copilot_identity_fetch`]).
    pub copilot_identity: ArcSwap<CopilotIdentity>,
}

/// How long to wait before asking for the Copilot client versions again
/// after a failed fetch.
const COPILOT_VERSIONS_RETRY: Duration = Duration::from_mins(5);

impl AppState {
    /// Creates a new shared application state wrapped in an `Arc`.
    ///
    /// `http` is the upstream client (see [`upstream_client`]). An optional
    /// [`UsageStore`] enables persistent usage tracking.
    pub fn new(
        config: Arc<ArcSwap<byokey_config::Config>>,
        auth: Arc<AuthManager>,
        http: reqwest::Client,
        usage_store: Option<Arc<dyn UsageStore>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            auth,
            http,
            usage: Arc::new(UsageRecorder::new(usage_store)),
            device_profiles: Arc::new(DeviceProfileCache::new()),
            copilot_identity: ArcSwap::from_pointee(CopilotIdentity::default()),
        })
    }

    /// Fetch the published Copilot client versions in the background and
    /// present them once they arrive, retrying every
    /// [`COPILOT_VERSIONS_RETRY`] until then, so serving never waits on the
    /// network for them.
    pub fn spawn_copilot_identity_fetch(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match CopilotIdentity::fetch(&state.http).await {
                    Ok(identity) => {
                        state.copilot_identity.store(Arc::new(identity));
                        return;
                    }
                    Err(e) => tracing::warn!(
                        error = %e,
                        retry_secs = COPILOT_VERSIONS_RETRY.as_secs(),
                        "Copilot client versions unavailable, using the built-in ones"
                    ),
                }
                tokio::time::sleep(COPILOT_VERSIONS_RETRY).await;
            }
        })
    }
}
