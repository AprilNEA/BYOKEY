//! BYOKEY management API handlers under `/v0/management/*`.
//!
//! Submodules:
//! - [`accounts`]    — list / remove / activate provider accounts.
//! - [`ratelimits`]  — per-provider rate-limit snapshots.
//! - [`status`]      — server + provider health.
//! - [`usage`]       — aggregate and historical token usage.

pub mod accounts;
pub mod ratelimits;
pub mod status;
pub mod usage;

use axum::{
    Router,
    routing::{delete, get, post},
};
use std::sync::Arc;

use crate::AppState;
use crate::handler::amp;

/// Build the BYOKEY management sub-router (nested at `/v0/management`).
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(status::status_handler))
        .route("/usage", get(usage::usage_handler))
        .route("/usage/history", get(usage::usage_history_handler))
        .route("/accounts", get(accounts::accounts_handler))
        .route(
            "/accounts/{provider}/{account_id}",
            delete(accounts::remove_account_handler),
        )
        .route(
            "/accounts/{provider}/{account_id}/activate",
            post(accounts::activate_account_handler),
        )
        .route("/ratelimits", get(ratelimits::ratelimits_handler))
        .route("/amp/threads", get(amp::threads::list_threads))
        .route("/amp/threads/{id}", get(amp::threads::get_thread))
}
