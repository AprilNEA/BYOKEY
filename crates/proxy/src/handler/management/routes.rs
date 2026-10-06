//! `RoutesService`: which provider serves each Anthropic model.

use std::sync::Arc;

use byokey_config::RouteSource;
use byokey_types::ClaudeModel;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};

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
        let catalog = Catalog::fetch(&self.0, &config).await;
        let routes = &config.anthropic.routes;
        let mut offered = catalog.models();
        for &model in routes.models.keys() {
            offered.entry(model).or_default();
        }
        let mut models: Vec<(ClaudeModel, Vec<_>)> = offered.into_iter().collect();
        lineup::sort(&mut models, |(m, _)| *m);
        Response::ok(rt::ListRoutesResponse {
            models: models
                .into_iter()
                .map(|(model, offered_by)| {
                    let (provider, source) = routes.resolve(model);
                    rt::ModelRoute {
                        model: model.to_string(),
                        provider: provider.to_string(),
                        source: wire_source(source).into(),
                        offered_by: offered_by.iter().map(ToString::to_string).collect(),
                        ..Default::default()
                    }
                })
                .collect(),
            default_provider: routes.default.map(|p| p.to_string()),
            families: routes
                .families
                .iter()
                .map(|(f, p)| (f.to_string(), p.to_string()))
                .collect(),
            ..Default::default()
        })
    }
}
