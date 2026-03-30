//! `OpenAPI` specification aggregation.

use utoipa::OpenApi;
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::handler::status::status_handler,
        crate::handler::accounts::accounts_handler,
        crate::handler::accounts::remove_account_handler,
        crate::handler::accounts::activate_account_handler,
        crate::handler::ratelimits::ratelimits_handler,
        crate::handler::usage::usage_handler,
        crate::handler::usage::usage_history_handler,
        crate::handler::models::list_models,
        crate::handler::amp_threads::list_threads,
        crate::handler::amp_threads::get_thread,
    ),
    components(schemas(
        crate::handler::status::StatusResponse,
        crate::handler::status::ServerInfo,
        crate::handler::status::ProviderStatus,
        crate::handler::status::AuthStatus,
        crate::handler::accounts::AccountsResponse,
        crate::handler::accounts::ProviderAccounts,
        crate::handler::accounts::AccountDetail,
        crate::handler::accounts::TokenStateDto,
        crate::handler::ratelimits::RateLimitsResponse,
        crate::handler::ratelimits::ProviderRateLimits,
        crate::handler::ratelimits::AccountRateLimit,
        byokey_types::RateLimitSnapshot,
        crate::usage::UsageSnapshot,
        crate::usage::ModelStats,
        crate::handler::usage::UsageHistoryQuery,
        crate::handler::usage::UsageHistoryResponse,
        byokey_types::UsageBucket,
        crate::handler::models::ModelsResponse,
        crate::handler::models::ModelEntry,
        crate::handler::amp_threads::AmpThreadListResponse,
        crate::handler::amp_threads::AmpThreadListQuery,
        crate::handler::amp_threads::AmpThreadSummary,
        crate::handler::amp_threads::AmpThreadDetail,
        crate::handler::amp_threads::AmpMessage,
        crate::handler::amp_threads::AmpContentBlock,
        crate::handler::amp_threads::AmpToolRun,
        crate::handler::amp_threads::AmpUsage,
        crate::handler::amp_threads::AmpMessageState,
        crate::handler::amp_threads::AmpRelationship,
    )),
    tags((name = "management", description = "Daemon management API"))
)]
pub struct ApiDoc;

/// Returns the `OpenAPI` specification as JSON.
pub async fn openapi_json() -> axum::Json<utoipa::openapi::OpenApi> {
    axum::Json(ApiDoc::openapi())
}
