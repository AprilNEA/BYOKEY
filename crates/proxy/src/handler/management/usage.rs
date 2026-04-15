//! Usage statistics endpoints — current counters and historical data.

use crate::AppState;
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for the usage history endpoint.
#[derive(Deserialize, ToSchema, IntoParams)]
pub struct UsageHistoryQuery {
    /// Start of the time range (unix timestamp). Defaults to 24 hours ago.
    pub from: Option<i64>,
    /// End of the time range (unix timestamp). Defaults to now.
    pub to: Option<i64>,
    /// Optional model name filter.
    pub model: Option<String>,
}

/// Response for the usage history endpoint.
#[derive(Serialize, ToSchema)]
pub struct UsageHistoryResponse {
    pub from: i64,
    pub to: i64,
    pub bucket_seconds: i64,
    pub buckets: Vec<byokey_types::UsageBucket>,
}

/// Returns current in-memory usage counters.
#[utoipa::path(
    get,
    path = "/v0/management/usage",
    responses((status = 200, body = crate::usage::UsageSnapshot)),
    tag = "management"
)]
pub async fn usage_handler(
    State(state): State<Arc<AppState>>,
) -> Json<crate::usage::UsageSnapshot> {
    Json(state.usage.snapshot())
}

/// Returns bucketed usage history from the persistent store.
#[utoipa::path(
    get,
    path = "/v0/management/usage/history",
    params(UsageHistoryQuery),
    responses(
        (status = 200, body = UsageHistoryResponse),
    ),
    tag = "management"
)]
pub async fn usage_history_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<UsageHistoryQuery>,
) -> Json<serde_json::Value> {
    let Some(store) = state.usage.store() else {
        return Json(serde_json::json!({ "error": "no persistent usage store configured" }));
    };

    #[allow(clippy::cast_possible_wrap)]
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let to = q.to.unwrap_or(now);
    let from = q.from.unwrap_or(to - 86400);

    // Auto-select bucket size based on range.
    let range = to - from;
    let bucket_secs = if range <= 86400 {
        3600 // hourly
    } else if range <= 86400 * 7 {
        21600 // 6-hour
    } else {
        86400 // daily
    };

    match store.query(from, to, q.model.as_deref(), bucket_secs).await {
        Ok(buckets) => Json(serde_json::json!({
            "from": from,
            "to": to,
            "bucket_seconds": bucket_secs,
            "buckets": buckets,
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}
