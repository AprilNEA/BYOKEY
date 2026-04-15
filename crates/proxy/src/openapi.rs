//! `OpenAPI` specification aggregation.

use utoipa::OpenApi;
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::handler::management::status::status_handler,
        crate::handler::management::accounts::accounts_handler,
        crate::handler::management::accounts::remove_account_handler,
        crate::handler::management::accounts::activate_account_handler,
        crate::handler::management::ratelimits::ratelimits_handler,
        crate::handler::management::usage::usage_handler,
        crate::handler::management::usage::usage_history_handler,
        crate::handler::models::list_models,
        crate::handler::amp::threads::list_threads,
        crate::handler::amp::threads::get_thread,
    ),
    components(schemas(
        crate::handler::management::status::StatusResponse,
        crate::handler::management::status::ServerInfo,
        crate::handler::management::status::ProviderStatus,
        crate::handler::management::status::AuthStatus,
        crate::handler::management::accounts::AccountsResponse,
        crate::handler::management::accounts::ProviderAccounts,
        crate::handler::management::accounts::AccountDetail,
        crate::handler::management::accounts::TokenStateDto,
        crate::handler::management::ratelimits::RateLimitsResponse,
        crate::handler::management::ratelimits::ProviderRateLimits,
        crate::handler::management::ratelimits::AccountRateLimit,
        byokey_types::RateLimitSnapshot,
        crate::usage::UsageSnapshot,
        crate::usage::ModelStats,
        crate::handler::management::usage::UsageHistoryQuery,
        crate::handler::management::usage::UsageHistoryResponse,
        byokey_types::UsageBucket,
        crate::handler::models::ModelsResponse,
        crate::handler::models::ModelEntry,
        crate::handler::amp::threads::AmpThreadListResponse,
        crate::handler::amp::threads::AmpThreadListQuery,
        crate::handler::amp::threads::AmpThreadSummary,
        crate::handler::amp::threads::AmpThreadDetail,
        crate::handler::amp::threads::AmpMessage,
        crate::handler::amp::threads::AmpContentBlock,
        crate::handler::amp::threads::AmpToolRun,
        crate::handler::amp::threads::AmpUsage,
        crate::handler::amp::threads::AmpMessageState,
        crate::handler::amp::threads::AmpRelationship,
    )),
    tags((name = "management", description = "Daemon management API"))
)]
pub struct ApiDoc;

/// Returns the `OpenAPI` specification as JSON.
pub async fn openapi_json() -> axum::Json<utoipa::openapi::OpenApi> {
    axum::Json(ApiDoc::openapi())
}
