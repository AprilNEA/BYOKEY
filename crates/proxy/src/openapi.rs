//! `OpenAPI` specification aggregation.

use utoipa::OpenApi;
#[derive(OpenApi)]
#[openapi(
    paths(crate::status::status_handler),
    components(schemas(
        crate::status::StatusResponse,
        crate::status::ServerInfo,
        crate::status::ProviderStatus,
        crate::status::AuthStatus,
    )),
    tags((name = "management", description = "Daemon management API"))
)]
pub struct ApiDoc;

/// Returns the `OpenAPI` specification as JSON.
pub async fn openapi_json() -> axum::Json<utoipa::openapi::OpenApi> {
    axum::Json(ApiDoc::openapi())
}
