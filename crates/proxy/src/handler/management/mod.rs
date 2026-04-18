//! BYOKEY management `ConnectRPC` services.
//!
//! Three services split by domain:
//! - [`StatusServiceImpl`] — server health, usage, rate limits
//! - [`AccountsServiceImpl`] — provider account CRUD
//! - [`AmpServiceImpl`] — local Amp CLI thread browsing

use std::sync::Arc;

use buffa::MessageField;
use buffa::view::OwnedView;
use buffa_types::google::protobuf::value::Kind;
use buffa_types::google::protobuf::{ListValue, NullValue, Struct, Value};
use connectrpc::{ConnectError, Context, Router as ConnectRouter};
use serde_json::Value as JsonValue;

use byokey_proto::byokey::accounts as acct;
use byokey_proto::byokey::amp as amp_pb;
use byokey_proto::byokey::status as stat;

use crate::AppState;
use crate::handler::amp::threads as internal_threads;

// ───────────────────────── public entry point ─────────────────────

/// Build a [`ConnectRouter`] with all three management services registered.
#[must_use]
pub fn build_router(state: Arc<AppState>) -> ConnectRouter {
    use acct::AccountsServiceExt as _;
    use amp_pb::AmpServiceExt as _;
    use stat::StatusServiceExt as _;

    let router = ConnectRouter::new();
    let router = Arc::new(StatusServiceImpl(state.clone())).register(router);
    let router = Arc::new(AccountsServiceImpl(state.clone())).register(router);
    Arc::new(AmpServiceImpl(state)).register(router)
}

// ───────────────────────── helpers ────────────────────────────────

fn byok_to_connect_error(e: &byokey_types::ByokError) -> ConnectError {
    use byokey_types::ByokError;
    let msg = e.to_string();
    match e {
        ByokError::Auth(_) | ByokError::TokenNotFound(_) | ByokError::TokenExpired(_) => {
            ConnectError::unauthenticated(msg)
        }
        ByokError::UnsupportedModel(_) => ConnectError::not_found(msg),
        ByokError::UnsupportedProvider(_) | ByokError::Translation(_) => {
            ConnectError::invalid_argument(msg)
        }
        ByokError::ProviderUnavailable(_) => ConnectError::unavailable(msg),
        _ => ConnectError::internal(msg),
    }
}

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

const THIRTY_DAYS_SECS: i64 = 30 * 24 * 3600;
// Largest time range a single `GetUsageByAccount` request can ask for.
// Prevents adversarial queries that would scan the entire usage table.
const MAX_USAGE_RANGE_SECS: i64 = 365 * 24 * 3600;

fn policy_strategy_to_proto(kind: byokey_config::PolicyStrategyKind) -> stat::RoutingStrategy {
    use byokey_config::PolicyStrategyKind as K;
    match kind {
        K::RoundRobin => stat::RoutingStrategy::ROUTING_STRATEGY_ROUND_ROBIN,
        K::WeightedRoundRobin => stat::RoutingStrategy::ROUTING_STRATEGY_WEIGHTED_ROUND_ROBIN,
        K::Random => stat::RoutingStrategy::ROUTING_STRATEGY_RANDOM,
        K::WeightedRandom => stat::RoutingStrategy::ROUTING_STRATEGY_WEIGHTED_RANDOM,
        K::Priority => stat::RoutingStrategy::ROUTING_STRATEGY_PRIORITY,
    }
}

#[allow(clippy::cast_possible_wrap)]
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ═══════════════════════ StatusService ════════════════════════════

struct StatusServiceImpl(Arc<AppState>);

impl stat::StatusService for StatusServiceImpl {
    async fn get_status(
        &self,
        ctx: Context,
        _: OwnedView<stat::GetStatusRequestView<'static>>,
    ) -> Result<(stat::GetStatusResponse, Context), ConnectError> {
        let snapshot = self.0.config.load();
        let server = stat::ServerInfo {
            host: snapshot.host.clone(),
            port: u32::from(snapshot.port),
            ..Default::default()
        };

        let mut providers = Vec::new();
        for pid in byokey_types::ProviderId::all() {
            let cfg = snapshot.providers.get(pid);
            let has_key = cfg.is_some_and(|c| c.api_key.is_some() || !c.api_keys.is_empty());
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
                models_count: clamp_to_u32(byokey_provider::models_for_provider(pid).len()),
                ..Default::default()
            });
        }
        Ok((
            stat::GetStatusResponse {
                server: server.into(),
                providers,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn get_usage(
        &self,
        ctx: Context,
        _: OwnedView<stat::GetUsageRequestView<'static>>,
    ) -> Result<(stat::GetUsageResponse, Context), ConnectError> {
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
        Ok((
            stat::GetUsageResponse {
                total_requests: s.total_requests,
                success_requests: s.success_requests,
                failure_requests: s.failure_requests,
                input_tokens: s.input_tokens,
                output_tokens: s.output_tokens,
                models,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn get_usage_history(
        &self,
        ctx: Context,
        request: OwnedView<stat::GetUsageHistoryRequestView<'static>>,
    ) -> Result<(stat::GetUsageHistoryResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let Some(store) = self.0.usage.store() else {
            let to = now_seconds();
            return Ok((
                stat::GetUsageHistoryResponse {
                    from: to - 86400,
                    to,
                    bucket_seconds: 3600,
                    error: Some("no persistent usage store configured".into()),
                    ..Default::default()
                },
                ctx,
            ));
        };
        let to = req.to.unwrap_or_else(now_seconds);
        let from = req.from.unwrap_or(to - 86400);
        let range = to - from;
        let bs = if range <= 86400 {
            3600
        } else if range <= 86400 * 7 {
            21600
        } else {
            86400
        };
        let buckets = store
            .query(from, to, req.model.as_deref(), bs)
            .await
            .map_err(|e| ConnectError::internal(e.to_string()))?
            .into_iter()
            .map(|b| stat::UsageBucket {
                period_start: b.period_start,
                model: b.model,
                request_count: b.request_count,
                input_tokens: b.input_tokens,
                output_tokens: b.output_tokens,
                ..Default::default()
            })
            .collect();
        Ok((
            stat::GetUsageHistoryResponse {
                from,
                to,
                bucket_seconds: bs,
                buckets,
                error: None,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn get_usage_by_account(
        &self,
        ctx: Context,
        request: OwnedView<stat::GetUsageByAccountRequestView<'static>>,
    ) -> Result<(stat::GetUsageByAccountResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let Some(store) = self.0.usage.store() else {
            return Ok((
                stat::GetUsageByAccountResponse {
                    rows: Vec::new(),
                    error: Some("no persistent usage store configured".into()),
                    ..Default::default()
                },
                ctx,
            ));
        };
        let to = req.to.unwrap_or_else(now_seconds);
        let from = req
            .from
            .unwrap_or_else(|| to.saturating_sub(THIRTY_DAYS_SECS));
        if from > to {
            return Err(ConnectError::invalid_argument(
                "from must be less than or equal to to",
            ));
        }
        // Reject unbounded ranges so adversarial clients can't force a
        // full-table scan by requesting, e.g., epoch-0-to-now.
        if to.saturating_sub(from) > MAX_USAGE_RANGE_SECS {
            return Err(ConnectError::invalid_argument(format!(
                "requested range exceeds maximum of {} days",
                MAX_USAGE_RANGE_SECS / 86_400
            )));
        }
        let totals = store
            .totals_by_account(Some(from), Some(to))
            .await
            .map_err(|e| ConnectError::internal(e.to_string()))?;
        let rows = totals
            .into_iter()
            .map(|t| stat::AccountUsageRow {
                provider: t.provider,
                account_id: t.account_id,
                model: t.model,
                request_count: t.request_count,
                success_count: t.success_count,
                input_tokens: t.input_tokens,
                output_tokens: t.output_tokens,
                ..Default::default()
            })
            .collect();
        Ok((
            stat::GetUsageByAccountResponse {
                rows,
                error: None,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn list_routing_policies(
        &self,
        ctx: Context,
        _: OwnedView<stat::ListRoutingPoliciesRequestView<'static>>,
    ) -> Result<(stat::ListRoutingPoliciesResponse, Context), ConnectError> {
        let snapshot = self.0.config.load();
        let policies = snapshot
            .routing_policies
            .iter()
            .map(|entry| stat::RoutingPolicy {
                provider: entry.provider.to_string(),
                family: entry.family.clone().unwrap_or_default(),
                strategy: policy_strategy_to_proto(entry.strategy).into(),
                accounts: entry.accounts.clone(),
                weights: entry.weights.clone(),
                ..Default::default()
            })
            .collect();
        Ok((
            stat::ListRoutingPoliciesResponse {
                policies,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn set_routing_policy(
        &self,
        _ctx: Context,
        _: OwnedView<stat::SetRoutingPolicyRequestView<'static>>,
    ) -> Result<(stat::SetRoutingPolicyResponse, Context), ConnectError> {
        // TODO(slice-7): wire to ConfigWatcher hot-reload
        // Server-side mutation of settings.json is not yet implemented.
        // Edit settings.json directly until hot-reload is wired up.
        Err(ConnectError::unimplemented(
            "SetRoutingPolicy is not yet implemented; edit settings.json directly",
        ))
    }

    async fn get_rate_limits(
        &self,
        ctx: Context,
        _: OwnedView<stat::GetRateLimitsRequestView<'static>>,
    ) -> Result<(stat::GetRateLimitsResponse, Context), ConnectError> {
        let all = self.0.ratelimits.all();
        let mut by_prov: std::collections::HashMap<
            byokey_types::ProviderId,
            Vec<stat::AccountRateLimit>,
        > = std::collections::HashMap::new();
        for ((prov, aid), snap) in all {
            by_prov
                .entry(prov)
                .or_default()
                .push(stat::AccountRateLimit {
                    account_id: aid,
                    snapshot: MessageField::some(stat::RateLimitSnapshot {
                        headers: snap.headers,
                        captured_at: snap.captured_at,
                        ..Default::default()
                    }),
                    ..Default::default()
                });
        }
        let providers = byokey_types::ProviderId::all()
            .iter()
            .filter_map(|pid| {
                let accts = by_prov.remove(pid)?;
                Some(stat::ProviderRateLimits {
                    id: pid.to_string(),
                    display_name: pid.display_name().to_string(),
                    accounts: accts,
                    ..Default::default()
                })
            })
            .collect();
        Ok((
            stat::GetRateLimitsResponse {
                providers,
                ..Default::default()
            },
            ctx,
        ))
    }
}

// ═══════════════════════ AccountsService ══════════════════════════

struct AccountsServiceImpl(Arc<AppState>);

impl acct::AccountsService for AccountsServiceImpl {
    async fn list_accounts(
        &self,
        ctx: Context,
        _: OwnedView<acct::ListAccountsRequestView<'static>>,
    ) -> Result<(acct::ListAccountsResponse, Context), ConnectError> {
        let mut providers = Vec::new();
        for pid in byokey_types::ProviderId::all() {
            let infos = self.0.auth.list_accounts(pid).await.unwrap_or_default();
            let tokens = self.0.auth.get_all_tokens(pid).await.unwrap_or_default();
            let accounts = infos
                .iter()
                .map(|info| {
                    let (ts, exp) = match tokens.iter().find(|(id, _)| id == &info.account_id) {
                        Some((_, tok)) => {
                            let s = match tok.state() {
                                byokey_types::TokenState::Valid => {
                                    acct::TokenState::TOKEN_STATE_VALID
                                }
                                byokey_types::TokenState::Expired => {
                                    acct::TokenState::TOKEN_STATE_EXPIRED
                                }
                                byokey_types::TokenState::Invalid => {
                                    acct::TokenState::TOKEN_STATE_INVALID
                                }
                            };
                            (s, tok.expires_at)
                        }
                        None => (acct::TokenState::TOKEN_STATE_INVALID, None),
                    };
                    acct::AccountDetail {
                        account_id: info.account_id.clone(),
                        label: info.label.clone(),
                        is_active: info.is_active,
                        token_state: ts.into(),
                        expires_at: exp,
                        ..Default::default()
                    }
                })
                .collect();
            providers.push(acct::ProviderAccounts {
                id: pid.to_string(),
                display_name: pid.display_name().to_string(),
                accounts,
                ..Default::default()
            });
        }
        Ok((
            acct::ListAccountsResponse {
                providers,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn remove_account(
        &self,
        ctx: Context,
        request: OwnedView<acct::RemoveAccountRequestView<'static>>,
    ) -> Result<(acct::RemoveAccountResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let pid: byokey_types::ProviderId = req
            .provider
            .parse()
            .map_err(|e: byokey_types::ByokError| byok_to_connect_error(&e))?;
        self.0
            .auth
            .remove_token_for(&pid, &req.account_id)
            .await
            .map_err(|e| byok_to_connect_error(&e))?;
        Ok((acct::RemoveAccountResponse::default(), ctx))
    }

    async fn activate_account(
        &self,
        ctx: Context,
        request: OwnedView<acct::ActivateAccountRequestView<'static>>,
    ) -> Result<(acct::ActivateAccountResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let pid: byokey_types::ProviderId = req
            .provider
            .parse()
            .map_err(|e: byokey_types::ByokError| byok_to_connect_error(&e))?;
        self.0
            .auth
            .set_active_account(&pid, &req.account_id)
            .await
            .map_err(|e| byok_to_connect_error(&e))?;
        Ok((acct::ActivateAccountResponse::default(), ctx))
    }

    async fn add_api_key(
        &self,
        ctx: Context,
        request: OwnedView<acct::AddApiKeyRequestView<'static>>,
    ) -> Result<(acct::AddApiKeyResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let pid: byokey_types::ProviderId = req
            .provider
            .parse()
            .map_err(|e: byokey_types::ByokError| byok_to_connect_error(&e))?;
        if req.api_key.trim().is_empty() {
            return Err(ConnectError::invalid_argument("api_key cannot be empty"));
        }
        let account_id = req
            .account_id
            .unwrap_or_else(|| byokey_types::DEFAULT_ACCOUNT.to_string());
        let token = byokey_types::OAuthToken {
            access_token: req.api_key.trim().to_string(),
            refresh_token: None,
            expires_at: None,
            token_type: Some("api-key".to_string()),
        };
        self.0
            .auth
            .save_token_for(&pid, &account_id, req.label.as_deref(), token)
            .await
            .map_err(|e| byok_to_connect_error(&e))?;
        Ok((
            acct::AddApiKeyResponse {
                account_id,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn import_claude_code(
        &self,
        ctx: Context,
        request: OwnedView<acct::ImportClaudeCodeRequestView<'static>>,
    ) -> Result<(acct::ImportClaudeCodeResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let token = byokey_auth::provider::claude_code::load_token()
            .await
            .map_err(|e| byok_to_connect_error(&e))?
            .ok_or_else(|| {
                ConnectError::failed_precondition(
                    "no Claude Code credentials found — is Claude Code logged in on this machine?",
                )
            })?;
        let pid = byokey_types::ProviderId::Claude;
        let account_id = req.account_id.unwrap_or_else(|| "claude-code".to_string());
        let label = req.label.unwrap_or_else(|| "Claude Code".to_string());
        self.0
            .auth
            .save_token_for(&pid, &account_id, Some(label.as_str()), token)
            .await
            .map_err(|e| byok_to_connect_error(&e))?;
        Ok((
            acct::ImportClaudeCodeResponse {
                account_id,
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn login(
        &self,
        ctx: Context,
        request: OwnedView<acct::LoginRequestView<'static>>,
    ) -> Result<
        (
            std::pin::Pin<
                Box<dyn futures_util::Stream<Item = Result<acct::LoginEvent, ConnectError>> + Send>,
            >,
            Context,
        ),
        ConnectError,
    > {
        use futures_util::StreamExt as _;
        use tokio_stream::wrappers::ReceiverStream;

        let req = request.to_owned_message();
        let pid: byokey_types::ProviderId = req
            .provider
            .parse()
            .map_err(|e: byokey_types::ByokError| byok_to_connect_error(&e))?;
        let account = req.account_id;

        let (progress_tx, progress_rx) =
            tokio::sync::mpsc::channel::<byokey_auth::flow::LoginProgress>(8);
        let (event_tx, event_rx) =
            tokio::sync::mpsc::channel::<Result<acct::LoginEvent, ConnectError>>(16);

        let auth = self.0.auth.clone();
        let event_tx_drive = event_tx.clone();
        tokio::spawn(async move {
            let mut progress_rx = progress_rx;
            let account_ref = account.as_deref();
            let login_fut =
                byokey_auth::flow::login_with_events(&pid, &auth, account_ref, Some(progress_tx));
            tokio::pin!(login_fut);

            loop {
                tokio::select! {
                    biased;
                    Some(p) = progress_rx.recv() => {
                        let ev = progress_to_pb(&p);
                        if event_tx_drive.send(Ok(ev)).await.is_err() { return; }
                    }
                    res = &mut login_fut => {
                        // Drain any remaining progress events before emitting terminal.
                        while let Ok(p) = progress_rx.try_recv() {
                            let ev = progress_to_pb(&p);
                            let _ = event_tx_drive.send(Ok(ev)).await;
                        }
                        let terminal = match res {
                            Ok(()) => acct::LoginEvent {
                                stage: acct::LoginStage::LOGIN_STAGE_DONE.into(),
                                ..Default::default()
                            },
                            Err(e) => acct::LoginEvent {
                                stage: acct::LoginStage::LOGIN_STAGE_FAILED.into(),
                                error: Some(e.to_string()),
                                ..Default::default()
                            },
                        };
                        let _ = event_tx_drive.send(Ok(terminal)).await;
                        return;
                    }
                }
            }
        });

        let stream = ReceiverStream::new(event_rx).boxed();
        Ok((stream, ctx))
    }
}

fn progress_to_pb(p: &byokey_auth::flow::LoginProgress) -> acct::LoginEvent {
    use byokey_auth::flow::LoginProgress as P;
    let (stage, message, user_code) = match p {
        P::Started => (acct::LoginStage::LOGIN_STAGE_STARTED, None, None),
        P::OpenedBrowser { url, user_code } => (
            acct::LoginStage::LOGIN_STAGE_OPENED_BROWSER,
            Some(url.clone()),
            user_code.clone(),
        ),
        P::GotCode => (acct::LoginStage::LOGIN_STAGE_GOT_CODE, None, None),
        P::Exchanging => (acct::LoginStage::LOGIN_STAGE_EXCHANGING, None, None),
    };
    acct::LoginEvent {
        stage: stage.into(),
        message,
        error: None,
        user_code,
        ..Default::default()
    }
}

// ═══════════════════════ AmpService ══════════════════════════════

struct AmpServiceImpl(Arc<AppState>);

fn to_pb_summary(s: &internal_threads::AmpThreadSummary) -> amp_pb::ThreadSummary {
    amp_pb::ThreadSummary {
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

fn to_pb_content_block(b: internal_threads::AmpContentBlock) -> amp_pb::ContentBlock {
    use amp_pb::content_block::Block;
    let block = Some(match b {
        internal_threads::AmpContentBlock::Text { text } => Block::Text(text),
        internal_threads::AmpContentBlock::Thinking { thinking } => Block::Thinking(thinking),
        internal_threads::AmpContentBlock::ToolUse { id, name, input } => {
            Block::ToolUse(Box::new(amp_pb::ToolUse {
                id,
                name,
                input: MessageField::some(json_to_pb_struct(input)),
                ..Default::default()
            }))
        }
        internal_threads::AmpContentBlock::ToolResult { tool_use_id, run } => {
            Block::ToolResult(Box::new(amp_pb::ToolResult {
                tool_use_id,
                run: MessageField::some(amp_pb::ToolRun {
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
    amp_pb::ContentBlock {
        block,
        ..Default::default()
    }
}

fn to_pb_message(m: internal_threads::AmpMessage) -> amp_pb::Message {
    amp_pb::Message {
        role: m.role,
        message_id: m.message_id,
        content: m.content.into_iter().map(to_pb_content_block).collect(),
        usage: m
            .usage
            .map(|u| amp_pb::Usage {
                model: u.model,
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_creation_input_tokens: u.cache_creation_input_tokens,
                cache_read_input_tokens: u.cache_read_input_tokens,
                total_input_tokens: u.total_input_tokens,
                ..Default::default()
            })
            .into(),
        state: m
            .state
            .map(|s| amp_pb::MessageState {
                state_type: s.state_type,
                stop_reason: s.stop_reason,
                ..Default::default()
            })
            .into(),
        ..Default::default()
    }
}

fn to_pb_detail(d: internal_threads::AmpThreadDetail) -> amp_pb::ThreadDetail {
    amp_pb::ThreadDetail {
        id: d.id,
        v: d.v,
        created: d.created,
        title: d.title,
        agent_mode: d.agent_mode,
        messages: d.messages.into_iter().map(to_pb_message).collect(),
        relationships: d
            .relationships
            .into_iter()
            .map(|r| amp_pb::Relationship {
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

impl amp_pb::AmpService for AmpServiceImpl {
    async fn list_threads(
        &self,
        ctx: Context,
        request: OwnedView<amp_pb::ListThreadsRequestView<'static>>,
    ) -> Result<(amp_pb::ListThreadsResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let all = self.0.amp_threads.list();
        let want_messages = req.has_messages.unwrap_or(true);
        let filtered: Vec<_> = all
            .iter()
            .filter(|s| !want_messages || s.message_count > 0)
            .collect();
        let total = filtered.len();
        let limit = usize::try_from(req.limit.unwrap_or(50))
            .unwrap_or(50)
            .min(200);
        let offset = usize::try_from(req.offset.unwrap_or(0))
            .unwrap_or(0)
            .min(total);
        let threads = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(to_pb_summary)
            .collect();
        Ok((
            amp_pb::ListThreadsResponse {
                threads,
                total: clamp_to_u32(total),
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn get_thread(
        &self,
        ctx: Context,
        request: OwnedView<amp_pb::GetThreadRequestView<'static>>,
    ) -> Result<(amp_pb::GetThreadResponse, Context), ConnectError> {
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
                ConnectError::internal("failed to parse thread")
            })
        })
        .await
        .map_err(|e| ConnectError::internal(format!("spawn_blocking failed: {e}")))??;
        Ok((
            amp_pb::GetThreadResponse {
                thread: MessageField::some(to_pb_detail(detail)),
                ..Default::default()
            },
            ctx,
        ))
    }

    async fn inject_url(
        &self,
        ctx: Context,
        request: OwnedView<amp_pb::InjectUrlRequestView<'static>>,
    ) -> Result<(amp_pb::InjectUrlResponse, Context), ConnectError> {
        let req = request.to_owned_message();
        let snapshot = self.0.config.load();
        let resolved_url =
            snapshot
                .amp
                .resolve_url(req.url.as_deref(), &snapshot.host, snapshot.port);
        let settings_path = byokey_config::AmpConfig::default_settings_path()
            .ok_or_else(|| ConnectError::internal("cannot determine HOME directory"))?;

        let amp_cfg = snapshot.amp.clone();
        let settings_path_for_spawn = settings_path.clone();
        let resolved_url_for_spawn = resolved_url.clone();
        #[allow(clippy::result_large_err)]
        let extras = tokio::task::spawn_blocking(move || {
            amp_cfg
                .inject(&resolved_url_for_spawn, &settings_path_for_spawn)
                .map_err(|e| ConnectError::internal(format!("inject failed: {e}")))
        })
        .await
        .map_err(|e| ConnectError::internal(format!("spawn_blocking failed: {e}")))??;

        Ok((
            amp_pb::InjectUrlResponse {
                resolved_url,
                settings_path: settings_path.display().to_string(),
                extras_merged: clamp_to_u32(extras),
                ..Default::default()
            },
            ctx,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_auth::flow::LoginProgress;

    #[test]
    fn progress_to_pb_started() {
        let ev = progress_to_pb(&LoginProgress::Started);
        assert_eq!(ev.stage, acct::LoginStage::LOGIN_STAGE_STARTED);
        assert!(ev.message.is_none());
        assert!(ev.user_code.is_none());
    }

    #[test]
    fn progress_to_pb_opened_browser_auth_code() {
        let ev = progress_to_pb(&LoginProgress::OpenedBrowser {
            url: "https://example.com/auth".into(),
            user_code: None,
        });
        assert_eq!(ev.stage, acct::LoginStage::LOGIN_STAGE_OPENED_BROWSER);
        assert_eq!(ev.message.as_deref(), Some("https://example.com/auth"));
        assert!(ev.user_code.is_none());
    }

    #[test]
    fn progress_to_pb_opened_browser_device_code() {
        let ev = progress_to_pb(&LoginProgress::OpenedBrowser {
            url: "https://github.com/login/device".into(),
            user_code: Some("ABCD-1234".into()),
        });
        assert_eq!(ev.stage, acct::LoginStage::LOGIN_STAGE_OPENED_BROWSER);
        assert_eq!(
            ev.message.as_deref(),
            Some("https://github.com/login/device")
        );
        assert_eq!(ev.user_code.as_deref(), Some("ABCD-1234"));
    }

    #[test]
    fn progress_to_pb_got_code() {
        let ev = progress_to_pb(&LoginProgress::GotCode);
        assert_eq!(ev.stage, acct::LoginStage::LOGIN_STAGE_GOT_CODE);
    }

    #[test]
    fn progress_to_pb_exchanging() {
        let ev = progress_to_pb(&LoginProgress::Exchanging);
        assert_eq!(ev.stage, acct::LoginStage::LOGIN_STAGE_EXCHANGING);
    }
}
