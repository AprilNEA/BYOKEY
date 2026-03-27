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
    )),
    tags((name = "management", description = "Daemon management API"))
)]
pub struct ApiDoc;

/// Returns the `OpenAPI` specification as JSON.
pub async fn openapi_json() -> axum::Json<utoipa::openapi::OpenApi> {
    axum::Json(ApiDoc::openapi())
}
