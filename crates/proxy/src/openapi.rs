//! `OpenAPI` specification aggregation.
//!
//! Only the OpenAI/Anthropic-compatible REST endpoints are documented here.
//! The local management API is served over `ConnectRPC` (see
//! [`crate::handler::management`] and `byokey_proto`); protobuf schemas are
//! the source of truth for those messages.

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::handler::models::list_models,
    ),
    components(schemas(
        crate::handler::models::ModelsResponse,
        crate::handler::models::ModelEntry,
    )),
    tags((name = "api", description = "OpenAI / Anthropic compatible REST API"))
)]
pub struct ApiDoc;

/// Returns the `OpenAPI` specification as JSON.
pub async fn openapi_json() -> axum::Json<utoipa::openapi::OpenApi> {
    axum::Json(ApiDoc::openapi())
}
