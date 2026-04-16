//! BYOKEY management ``ConnectRPC`` service.
//!
//! Implements [`ManagementService`] from `byokey_proto` and exposes it as
//! a ``ConnectRPC`` tower service. Mounted as the fallback service in
//! [`crate::router::make_router`], so all
//! `POST /byokey.management.ManagementService/{Method}` requests land
//! here regardless of other registered routes.
//!
//! This file replaces the previous REST handlers under `/v0/management/*`.
//! The business logic (auth queries, usage snapshot, rate-limit readout,
//! amp thread parsing) is identical — only the wire format changed.

use std::sync::Arc;

use buffa::MessageField;
use buffa::view::OwnedView;
use buffa_types::google::protobuf::value::Kind;
use buffa_types::google::protobuf::{ListValue, NullValue, Struct, Value};
use byokey_proto::byokey::management as pb;
use byokey_proto::byokey::management::{ManagementService, ManagementServiceExt as _};
use connectrpc::{ConnectError, Context, Router as ConnectRouter};
use serde_json::Value as JsonValue;

use crate::AppState;
use crate::handler::amp::threads as internal_threads;

// ───────────────────────────── service ─────────────────────────────

/// `ConnectRPC` implementation of the BYOKEY management service.
pub struct ManagementServiceImpl {
    state: Arc<AppState>,
}

impl ManagementServiceImpl {
    /// Construct a new service wired to the given shared application state.
    #[must_use]
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

/// Build a [`ConnectRouter`] pre-registered with the management service,
/// ready to be attached to an axum server via `.into_axum_service()`.
#[must_use]
pub fn build_router(state: Arc<AppState>) -> ConnectRouter {
    Arc::new(ManagementServiceImpl::new(state)).register(ConnectRouter::new())
}

// ───────────────────────────── helpers ─────────────────────────────

/// Map [`byokey_types::ByokError`] to a [`ConnectError`] with a sensible
/// grpc status code. Mirrors the axum [`crate::ApiError`] mapping.
fn byok_to_connect_error(e: &byokey_types::ByokError) -> ConnectError {
    use byokey_types::ByokError;
    let msg = e.to_string();
    match e {
        ByokError::Auth(_) | ByokError::TokenNotFound(_) | ByokError::TokenExpired(_) => {
            ConnectError::unauthenticated(msg)
        }
        ByokError::UnsupportedModel(_) | ByokError::UnsupportedProvider(_) => {
            ConnectError::not_found(msg)
        }
        ByokError::Translation(_) => ConnectError::invalid_argument(msg),
        _ => ConnectError::internal(msg),
    }
}

/// Recursively convert a `serde_json::Value` into a `google.protobuf.Value`.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn json_to_pb_value(v: JsonValue) -> Value {
    let kind = match v {
        JsonValue::Null => Kind::NullValue(NullValue::NULL_VALUE.into()),
        JsonValue::Bool(b) => Kind::BoolValue(b),
        JsonValue::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        JsonValue::String(s) => Kind::StringValue(s),
        JsonValue::Array(arr) => {
            let values = arr.into_iter().map(json_to_pb_value).collect();
            Kind::ListValue(Box::new(ListValue {
                values,
                ..Default::default()
            }))
        }
        JsonValue::Object(map) => {
            let fields = map
                .into_iter()
                .map(|(k, v)| (k, json_to_pb_value(v)))
                .collect();
            Kind::StructValue(Box::new(Struct {
                fields,
                ..Default::default()
            }))
        }
    };
    Value {
        kind: Some(kind),
        ..Default::default()
    }
}

/// Convert a `serde_json::Value` into a `google.protobuf.Struct`. Non-object
/// inputs produce an empty struct.
fn json_to_pb_struct(v: JsonValue) -> Struct {
    if let JsonValue::Object(map) = v {
        let fields = map
            .into_iter()
            .map(|(k, v)| (k, json_to_pb_value(v)))
            .collect();
        Struct {
            fields,
            ..Default::default()
        }
    } else {
        Struct::default()
    }
}

fn clamp_to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

// ── conversions: internal amp thread types → proto ────────────────

fn to_pb_amp_thread_summary(s: &internal_threads::AmpThreadSummary) -> pb::AmpThreadSummary {
    pb::AmpThreadSummary {
        id: s.id.clone(),
        created: s.created,
        title: s.title.clone(),
        message_count: clamp_to_u32(s.message_count),
        agent_mode: s.agent_mode.clone(),
        last_model: s.last_model.clone(),
        total_input_tokens: s.total_input_tokens,
        total_output_tokens: s.total_output_tokens,
        file_size_bytes: s.file_size_bytes,
        ..Default::default()
    }
}

fn to_pb_amp_usage(u: internal_threads::AmpUsage) -> pb::AmpUsage {
    pb::AmpUsage {
        model: u.model,
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cache_creation_input_tokens: u.cache_creation_input_tokens,
        cache_read_input_tokens: u.cache_read_input_tokens,
        total_input_tokens: u.total_input_tokens,
        ..Default::default()
    }
}

fn to_pb_amp_state(s: internal_threads::AmpMessageState) -> pb::AmpMessageState {
    pb::AmpMessageState {
        state_type: s.state_type,
        stop_reason: s.stop_reason,
        ..Default::default()
    }
}

fn to_pb_amp_content_block(b: internal_threads::AmpContentBlock) -> pb::AmpContentBlock {
    use pb::amp_content_block::Block;
    let block = Some(match b {
        internal_threads::AmpContentBlock::Text { text } => Block::Text(text),
        internal_threads::AmpContentBlock::Thinking { thinking } => Block::Thinking(thinking),
        internal_threads::AmpContentBlock::ToolUse { id, name, input } => {
            Block::ToolUse(Box::new(pb::AmpToolUse {
                id,
                name,
                input: MessageField::some(json_to_pb_struct(input)),
                ..Default::default()
            }))
        }
        internal_threads::AmpContentBlock::ToolResult { tool_use_id, run } => {
            Block::ToolResult(Box::new(pb::AmpToolResult {
                tool_use_id,
                run: MessageField::some(pb::AmpToolRun {
                    status: run.status,
                    result: run.result.map(json_to_pb_value).into(),
                    error: run.error.map(json_to_pb_value).into(),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }
        internal_threads::AmpContentBlock::Unknown { original_type } => {
            Block::UnknownType(original_type.unwrap_or_default())
        }
    });
    pb::AmpContentBlock {
        block,
        ..Default::default()
    }
}

fn to_pb_amp_message(m: internal_threads::AmpMessage) -> pb::AmpMessage {
    pb::AmpMessage {
        role: m.role,
        message_id: m.message_id,
        content: m.content.into_iter().map(to_pb_amp_content_block).collect(),
        usage: m.usage.map(to_pb_amp_usage).into(),
        state: m.state.map(to_pb_amp_state).into(),
        ..Default::default()
    }
}

fn to_pb_amp_thread_detail(d: internal_threads::AmpThreadDetail) -> pb::AmpThreadDetail {
    pb::AmpThreadDetail {
        id: d.id,
        v: d.v,
        created: d.created,
        title: d.title,
        agent_mode: d.agent_mode,
        messages: d.messages.into_iter().map(to_pb_amp_message).collect(),
        relationships: d
            .relationships
            .into_iter()
            .map(|r| pb::AmpRelationship {
                thread_id: r.thread_id,
                rel_type: r.rel_type,
                role: r.role,
                ..Default::default()
            })
            .collect(),
        env: d.env.map(json_to_pb_struct).into(),
        ..Default::default()
    }
}

// ───────────────────────── ManagementService impl ──────────────────────────

impl ManagementService for ManagementServiceImpl {
    // ── get_status ──────────────────────────────────────────────────

    async fn get_status(
        &self,
        ctx: Context,
        _request: OwnedView<pb::GetStatusRequestView<'static>>,
    ) -> Result<(pb::GetStatusResponse, Context), ConnectError> {
        let snapshot = self.state.config.load();

        let server = pb::ServerInfo {
            host: snapshot.host.clone(),
            port: u32::from(snapshot.port),
            ..Default::default()
        };

        let mut providers = Vec::new();
        for provider_id in byokey_types::ProviderId::all() {
            let config = snapshot.providers.get(provider_id);
            let enabled = config.is_none_or(|c| c.enabled);
            let has_api_key = config.is_some_and(|c| c.api_key.is_some() || !c.api_keys.is_empty());

            let auth_status = if has_api_key || self.state.auth.is_authenticated(provider_id).await
            {
                pb::AuthStatus::AUTH_STATUS_VALID
            } else {
                let accounts = self
                    .state
                    .auth
                    .list_accounts(provider_id)
                    .await
                    .unwrap_or_default();
                if accounts.is_empty() {
                    pb::AuthStatus::AUTH_STATUS_NOT_CONFIGURED
                } else {
                    pb::AuthStatus::AUTH_STATUS_EXPIRED
                }
            };

            let models_count =
                clamp_to_u32(byokey_provider::models_for_provider(provider_id).len());

            providers.push(pb::ProviderStatus {
                id: provider_id.to_string(),
                display_name: provider_id.display_name().to_string(),
                enabled,
                auth_status: auth_status.into(),
                models_count,
                ..Default::default()
            });
        }

        Ok((
            pb::GetStatusResponse {
                server: MessageField::some(server),
                providers,
                ..Default::default()
            },
            ctx,
        ))
    }

    // ── get_usage ───────────────────────────────────────────────────

    async fn get_usage(
        &self,
        ctx: Context,
        _request: OwnedView<pb::GetUsageRequestView<'static>>,
    ) -> Result<(pb::GetUsageResponse, Context), ConnectError> {
        let snap = self.state.usage.snapshot();
        let models = snap
            .models
            .into_iter()
            .map(|(model, m)| {
                (
                    model,
                    pb::ModelStats {
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

        Ok((
            pb::GetUsageResponse {
                total_requests: snap.total_requests,
                success_requests: snap.success_requests,
                failure_requests: snap.failure_requests,
                input_tokens: snap.input_tokens,
                output_tokens: snap.output_tokens,
                models,
                ..Default::default()
            },
            ctx,
        ))
    }

    // ── get_usage_history ───────────────────────────────────────────

    async fn get_usage_history(
        &self,
        ctx: Context,
        request: OwnedView<pb::GetUsageHistoryRequestView<'static>>,
    ) -> Result<(pb::GetUsageHistoryResponse, Context), ConnectError> {
        let req = request.to_owned_message();

        let Some(store) = self.state.usage.store() else {
            // Echo defaults so the client can still parse the response.
            let to_default = now_seconds();
            return Ok((
                pb::GetUsageHistoryResponse {
                    from: to_default - 86400,
                    to: to_default,
                    bucket_seconds: 3600,
                    buckets: Vec::new(),
                    error: Some("no persistent usage store configured".to_string()),
                    ..Default::default()
                },
                ctx,
            ));
        };

        let to = req.to.unwrap_or_else(now_seconds);
        let from = req.from.unwrap_or(to - 86400);
        let range = to - from;
        let bucket_secs = if range <= 86400 {
            3600
        } else if range <= 86400 * 7 {
            21600
        } else {
            86400
        };

        let buckets = store
            .query(from, to, req.model.as_deref(), bucket_secs)
            .await
            .map_err(|e| ConnectError::internal(e.to_string()))?;

        let buckets_pb = buckets
            .into_iter()
            .map(|b| pb::UsageBucket {
                period_start: b.period_start,
                model: b.model,
                request_count: b.request_count,
                input_tokens: b.input_tokens,
                output_tokens: b.output_tokens,
                ..Default::default()
            })
            .collect();

        Ok((
            pb::GetUsageHistoryResponse {
                from,
                to,
                bucket_seconds: bucket_secs,
                buckets: buckets_pb,
                error: None,
                ..Default::default()
            },
            ctx,
        ))
    }

    // ── list_accounts ───────────────────────────────────────────────

    async fn list_accounts(
        &self,
        ctx: Context,
        _request: OwnedView<pb::ListAccountsRequestView<'static>>,
    ) -> Result<(pb::ListAccountsResponse, Context), ConnectError> {
        let mut providers = Vec::new();

        for provider_id in byokey_types::ProviderId::all() {
            let accounts_info = self
                .state
                .auth
                .list_accounts(provider_id)
                .await
                .unwrap_or_default();
            let all_tokens = self
                .state
                .auth
                .get_all_tokens(provider_id)
                .await
                .unwrap_or_default();

            let mut accounts = Vec::new();
            for info in &accounts_info {
                let (token_state, expires_at) = match all_tokens
                    .iter()
                    .find(|(id, _)| id == &info.account_id)
                {
                    Some((_, token)) => {
                        let ts = match token.state() {
                            byokey_types::TokenState::Valid => pb::TokenState::TOKEN_STATE_VALID,
                            byokey_types::TokenState::Expired => {
                                pb::TokenState::TOKEN_STATE_EXPIRED
                            }
                            byokey_types::TokenState::Invalid => {
                                pb::TokenState::TOKEN_STATE_INVALID
                            }
                        };
                        (ts, token.expires_at)
                    }
                    None => (pb::TokenState::TOKEN_STATE_INVALID, None),
                };

                accounts.push(pb::AccountDetail {
                    account_id: info.account_id.clone(),
                    label: info.label.clone(),
                    is_active: info.is_active,
                    token_state: token_state.into(),
                    expires_at,
                    ..Default::default()
                });
            }

            providers.push(pb::ProviderAccounts {
                id: provider_id.to_string(),
                display_name: provider_id.display_name().to_string(),
                accounts,
                ..Default::default()
            });
        }

        Ok((
            pb::ListAccountsResponse {
                providers,
                ..Default::default()
            },
            ctx,
        ))
    }

    // ── remove_account ──────────────────────────────────────────────

    async fn remove_account(
        &self,
        ctx: Context,
        request: OwnedView<pb::RemoveAccountRequestView<'static>>,
    ) -> Result<(pb::RemoveAccountResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let provider_id: byokey_types::ProviderId = req
            .provider
            .parse()
            .map_err(|e: byokey_types::ByokError| byok_to_connect_error(&e))?;
        self.state
            .auth
            .remove_token_for(&provider_id, &req.account_id)
            .await
            .map_err(|e| byok_to_connect_error(&e))?;
        Ok((pb::RemoveAccountResponse::default(), ctx))
    }

    // ── activate_account ────────────────────────────────────────────

    async fn activate_account(
        &self,
        ctx: Context,
        request: OwnedView<pb::ActivateAccountRequestView<'static>>,
    ) -> Result<(pb::ActivateAccountResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let provider_id: byokey_types::ProviderId = req
            .provider
            .parse()
            .map_err(|e: byokey_types::ByokError| byok_to_connect_error(&e))?;
        self.state
            .auth
            .set_active_account(&provider_id, &req.account_id)
            .await
            .map_err(|e| byok_to_connect_error(&e))?;
        Ok((pb::ActivateAccountResponse::default(), ctx))
    }

    // ── get_rate_limits ─────────────────────────────────────────────

    async fn get_rate_limits(
        &self,
        ctx: Context,
        _request: OwnedView<pb::GetRateLimitsRequestView<'static>>,
    ) -> Result<(pb::GetRateLimitsResponse, Context), ConnectError> {
        let all = self.state.ratelimits.all();

        let mut by_provider: std::collections::HashMap<
            byokey_types::ProviderId,
            Vec<pb::AccountRateLimit>,
        > = std::collections::HashMap::new();

        for ((provider, account_id), snapshot) in all {
            by_provider
                .entry(provider)
                .or_default()
                .push(pb::AccountRateLimit {
                    account_id,
                    snapshot: MessageField::some(pb::RateLimitSnapshot {
                        headers: snapshot.headers,
                        captured_at: snapshot.captured_at,
                        ..Default::default()
                    }),
                    ..Default::default()
                });
        }

        let mut providers = Vec::new();
        for provider_id in byokey_types::ProviderId::all() {
            let accounts = by_provider.remove(provider_id).unwrap_or_default();
            if accounts.is_empty() {
                continue;
            }
            providers.push(pb::ProviderRateLimits {
                id: provider_id.to_string(),
                display_name: provider_id.display_name().to_string(),
                accounts,
                ..Default::default()
            });
        }

        Ok((
            pb::GetRateLimitsResponse {
                providers,
                ..Default::default()
            },
            ctx,
        ))
    }

    // ── list_amp_threads ────────────────────────────────────────────

    async fn list_amp_threads(
        &self,
        ctx: Context,
        request: OwnedView<pb::ListAmpThreadsRequestView<'static>>,
    ) -> Result<(pb::ListAmpThreadsResponse, Context), ConnectError> {
        let req = request.to_owned_message();

        let all = self.state.amp_threads.list();

        // has_messages defaults to true (hide empty threads).
        let has_messages_filter = req.has_messages.or(Some(true));

        let filtered: Vec<&internal_threads::AmpThreadSummary> = all
            .iter()
            .filter(|s| match has_messages_filter {
                Some(want) => (s.message_count > 0) == want,
                None => true,
            })
            .collect();

        let total = filtered.len();
        let limit = usize::try_from(req.limit.unwrap_or(50))
            .unwrap_or(50)
            .min(200);
        let offset = usize::try_from(req.offset.unwrap_or(0))
            .unwrap_or(0)
            .min(total);

        let threads: Vec<pb::AmpThreadSummary> = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(to_pb_amp_thread_summary)
            .collect();

        Ok((
            pb::ListAmpThreadsResponse {
                threads,
                total: clamp_to_u32(total),
                ..Default::default()
            },
            ctx,
        ))
    }

    // ── get_amp_thread ──────────────────────────────────────────────

    async fn get_amp_thread(
        &self,
        ctx: Context,
        request: OwnedView<pb::GetAmpThreadRequestView<'static>>,
    ) -> Result<(pb::GetAmpThreadResponse, Context), ConnectError> {
        let req = request.to_owned_message();

        if !internal_threads::is_valid_thread_id(&req.id) {
            return Err(ConnectError::invalid_argument("invalid thread ID format"));
        }

        let path = internal_threads::threads_dir().join(format!("{}.json", req.id));
        #[allow(clippy::result_large_err)]
        let detail = tokio::task::spawn_blocking(move || {
            if !path.exists() {
                return Err(ConnectError::not_found("thread not found"));
            }
            internal_threads::parse_detail(&path).map_err(|e| {
                tracing::error!(error = %e, "failed to parse amp thread");
                ConnectError::internal(format!("failed to parse thread: {e}"))
            })
        })
        .await
        .map_err(|e| ConnectError::internal(format!("spawn_blocking failed: {e}")))??;

        Ok((
            pb::GetAmpThreadResponse {
                thread: MessageField::some(to_pb_amp_thread_detail(detail)),
                ..Default::default()
            },
            ctx,
        ))
    }
}

#[allow(clippy::cast_possible_wrap)]
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
