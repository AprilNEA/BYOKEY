use std::sync::Arc;

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};

use byokey_proto::byokey::status as stat;

use crate::AppState;

pub(super) struct StatusServiceImpl(pub(super) Arc<AppState>);

impl stat::StatusService for StatusServiceImpl {
    async fn get_status(
        &self,
        _ctx: RequestContext,
        _: ServiceRequest<'_, stat::GetStatusRequest>,
    ) -> ServiceResult<stat::GetStatusResponse> {
        let snapshot = self.0.config.load();
        let server = stat::ServerInfo {
            host: snapshot.host.clone(),
            port: u32::from(snapshot.port),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            api_version: byokey_proto::API_VERSION,
            executable: std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            ..Default::default()
        };

        let mut providers = Vec::new();
        for pid in byokey_types::ProviderId::all() {
            let cfg = snapshot.providers.get(&pid);
            let has_key = cfg.is_some_and(|c| c.api_key.is_some());
            let auth = if has_key || self.0.auth.is_authenticated(pid).await {
                stat::AuthStatus::AUTH_STATUS_VALID
            } else {
                let accts = self.0.auth.list_accounts(pid).await.unwrap_or_default();
                if accts.is_empty() {
                    stat::AuthStatus::AUTH_STATUS_NOT_CONFIGURED
                } else {
                    stat::AuthStatus::AUTH_STATUS_EXPIRED
                }
            };
            providers.push(stat::ProviderStatus {
                id: pid.to_string(),
                display_name: pid.display_name().to_string(),
                enabled: cfg.is_none_or(|c| c.enabled),
                auth_status: auth.into(),
                ..Default::default()
            });
        }
        Response::ok(stat::GetStatusResponse {
            server: server.into(),
            providers,
            ..Default::default()
        })
    }

    async fn get_usage(
        &self,
        _ctx: RequestContext,
        _: ServiceRequest<'_, stat::GetUsageRequest>,
    ) -> ServiceResult<stat::GetUsageResponse> {
        let s = self.0.usage.snapshot();
        let models = s
            .models
            .into_iter()
            .map(|(k, m)| {
                (
                    k,
                    stat::ModelStats {
                        requests: m.requests,
                        success: m.success,
                        failure: m.failure,
                        input_tokens: m.input_tokens,
                        output_tokens: m.output_tokens,
                        ..Default::default()
                    },
                )
            })
            .collect();
        Response::ok(stat::GetUsageResponse {
            total_requests: s.total_requests,
            success_requests: s.success_requests,
            failure_requests: s.failure_requests,
            input_tokens: s.input_tokens,
            output_tokens: s.output_tokens,
            models,
            ..Default::default()
        })
    }
}
