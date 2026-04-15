//! `AmpCode` quota management endpoint — exposes cached free-tier and balance data.

use crate::AppState;
use axum::{Json, extract::State};
use byokey_types::AmpQuotaSnapshot;
use std::sync::Arc;

/// Returns the latest cached `AmpCode` quota snapshot.
///
/// Data is populated passively by intercepting `/api/internal` responses
/// for `getUserFreeTierStatus` and `userDisplayBalanceInfo`.
#[utoipa::path(
    get,
    path = "/v0/management/amp/quota",
    responses((status = 200, body = AmpQuotaSnapshot)),
    tag = "management"
)]
pub async fn amp_quota_handler(State(state): State<Arc<AppState>>) -> Json<AmpQuotaSnapshot> {
    Json(state.amp_quota.snapshot())
}
