//! `RoutesService`: which provider serves each Anthropic model.

use std::sync::Arc;

use byokey_config::RouteSource;
use byokey_types::ClaudeModel;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};

use byokey_proto::byokey::routes as rt;

use crate::AppState;
use crate::handler::catalog::Catalog;
use crate::handler::models::lineup;

pub(super) struct RoutesServiceImpl(pub(super) Arc<AppState>);

fn wire_source(source: RouteSource) -> rt::RouteSource {
    match source {
        RouteSource::Model => rt::RouteSource::ROUTE_SOURCE_MODEL,
        RouteSource::Family => rt::RouteSource::ROUTE_SOURCE_FAMILY,
        RouteSource::Default => rt::RouteSource::ROUTE_SOURCE_DEFAULT,
        RouteSource::Unset => rt::RouteSource::ROUTE_SOURCE_UNSET,
    }
}

impl rt::RoutesService for RoutesServiceImpl {
    async fn list_routes(
        &self,
        _ctx: RequestContext,
        _: ServiceRequest<'_, rt::ListRoutesRequest>,
    ) -> ServiceResult<rt::ListRoutesResponse> {
        let config = self.0.config.load();
        let catalog = Catalog::fetch(&self.0, &config)
            .await
            .map_err(|error| ConnectError::internal(error.error.to_string()))?;
        let routes = &config.anthropic.routes;
        let mut offered = catalog.models();
        for &model in routes.models.keys() {
            offered.entry(model.to_string()).or_default();
        }
        let mut models: Vec<_> = offered.into_iter().collect();
        lineup::sort(&mut models, |(m, _)| {
            ClaudeModel::parse_id(m).map(|id| id.model)
        });
        Response::ok(rt::ListRoutesResponse {
            models: models
                .into_iter()
                .map(|(model, offered_by)| {
                    let (provider, source) = routes.resolve_id(&model);
                    rt::ModelRoute {
                        model,
                        provider: provider.to_string(),
                        source: wire_source(source).into(),
                        offered_by,
                        ..Default::default()
                    }
                })
                .collect(),
            default_provider: routes.default.clone(),
            families: routes
                .families
                .iter()
                .map(|(f, p)| (f.to_string(), p.clone()))
                .collect(),
            ..Default::default()
        })
    }
}
