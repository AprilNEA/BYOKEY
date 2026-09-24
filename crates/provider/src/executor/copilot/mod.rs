//! GitHub Copilot executor — OpenAI-compatible API.
//!
//! Auth: device code flow → GitHub token. `OpenCode` tokens authenticate API
//! requests directly; VS Code tokens are first exchanged for a short-lived
//! Copilot API token.
//! Format: `OpenAI` passthrough via `aigw::openai_compat` for URL/header/request building.
//!         Streaming: raw byte passthrough (Option P). Non-streaming: aigw response translator.
mod device;
mod headers;

pub use device::CopilotDevice;
pub use headers::{Conversation, CopilotIdentity};

use crate::http_util::ProviderHttp;
use crate::registry;
use aigw::openai::translate::OpenAIResponseTranslator;
use aigw::openai::{HttpTransportConfig, OpenAIAuthConfig};
use aigw::openai_compat::translate::OpenAICompatRequestTranslator;
use aigw::openai_compat::{OpenAICompatConfig, OpenAICompatProvider, Quirks};
use aigw_core::translate::{RequestTranslator as _, ResponseTranslator as _};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    AccountInfo, ByokError, ChatRequest, CopilotClient, OAuthToken, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use secrecy::SecretString;
use serde_json::Value;
use std::{
    cmp::Ordering as CmpOrdering,
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

/// Cached quota snapshot for a single Copilot account.
struct CachedQuota {
    percent_remaining: f64,
    unlimited: bool,
    fetched_at: Instant,
}

/// Tracks the currently selected account and per-account quota snapshots.
struct AccountTracker {
    /// Currently sticky account id.
    current: Option<String>,
    /// When the last rebalance comparison happened.
    last_rebalance: Option<Instant>,
    /// Per-account cached quota data.
    quotas: HashMap<String, CachedQuota>,
}

/// Global account tracker for quota-aware multi-account routing.
static ACCOUNT_TRACKER: LazyLock<Mutex<AccountTracker>> = LazyLock::new(|| {
    Mutex::new(AccountTracker {
        current: None,
        last_rebalance: None,
        quotas: HashMap::new(),
    })
});

// `Duration::from_mins` is not yet a const fn on stable.
/// How often to re-compare quotas across accounts.
#[allow(clippy::duration_suboptimal_units)]
const REBALANCE_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Quota cache TTL — avoid re-fetching within this window.
#[allow(clippy::duration_suboptimal_units)]
const QUOTA_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Default GitHub Copilot Chat Completions API base URL.
const DEFAULT_BASE_URL: &str = "https://api.githubcopilot.com";

/// Endpoint to exchange a VS Code GitHub token for a short-lived Copilot API token.
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// Copilot usage/quota endpoint (returns `quota_snapshots`).
const COPILOT_USER_URL: &str = "https://api.github.com/copilot_internal/user";

/// A cached Copilot API token with its expiry time.
struct CachedToken {
    token: String,
    api_endpoint: String,
    expires_at: Instant,
}

/// Everything needed to send one Copilot API request as a given account.
#[derive(Clone, Debug)]
pub struct CopilotCredentials {
    /// Bearer token for the Copilot API.
    pub token: String,
    /// API base URL, without a path suffix.
    pub endpoint: String,
    /// The client the requests present themselves as.
    pub client: CopilotClient,
    /// The machine the account appears to be using.
    pub device: CopilotDevice,
}

/// VS Code GitHub token → short-lived Copilot API token.
///
/// Process-wide because executors are built per request; an instance-owned
/// cache would never be hit and every request would re-run the exchange.
static TOKEN_CACHE: LazyLock<Mutex<HashMap<String, CachedToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Score a cached quota for account comparison.
///
/// `unlimited` → 100, known quota → `percent_remaining`, unknown → 50 (neutral).
fn quota_score(q: Option<&CachedQuota>) -> f64 {
    match q {
        Some(q) if q.unlimited => 100.0,
        Some(q) => q.percent_remaining,
        None => 50.0,
    }
}

/// Executor for the GitHub Copilot API.
pub struct CopilotExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    base_url: Option<String>,
    auth: Arc<AuthManager>,
    identity: CopilotIdentity,
}

#[bon::bon]
impl CopilotExecutor {
    /// Creates a new Copilot executor.
    #[builder]
    pub fn new(
        http: wreq::Client,
        auth: Arc<AuthManager>,
        api_key: Option<String>,
        base_url: Option<String>,
        ratelimit: Option<Arc<RateLimitStore>>,
        identity: Option<CopilotIdentity>,
    ) -> Self {
        let mut ph = ProviderHttp::new(http);
        if let Some(store) = ratelimit {
            ph = ph.with_ratelimit(store, ProviderId::Copilot);
        }
        Self {
            ph,
            api_key,
            base_url,
            auth,
            identity: identity.unwrap_or_default(),
        }
    }

    /// A GET against `api.github.com` authenticated with `client`'s GitHub token.
    fn github_request(
        &self,
        url: &str,
        client: CopilotClient,
        github_token: &str,
    ) -> wreq::RequestBuilder {
        let mut builder = self
            .ph
            .client()
            .get(url)
            .header("authorization", format!("token {github_token}"));
        for (name, value) in self.identity.github_headers(client) {
            builder = builder.header(name, value);
        }
        builder
    }

    fn default_endpoint(&self) -> String {
        self.base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/')
            .to_owned()
    }

    /// The credentials for requests as the account holding `token`.
    async fn credentials_for(&self, token: &OAuthToken) -> Result<CopilotCredentials> {
        match CopilotClient::of(token)? {
            CopilotClient::OpenCode => Ok(CopilotCredentials {
                token: token.access_token.clone(),
                endpoint: self.default_endpoint(),
                client: CopilotClient::OpenCode,
                device: CopilotDevice::for_credential(&token.access_token),
            }),
            CopilotClient::VsCode => self.exchange_and_cache(&token.access_token).await,
        }
    }

    /// Exchange a VS Code GitHub token for a Copilot API token and cache the result.
    async fn exchange_and_cache(&self, github_token: &str) -> Result<CopilotCredentials> {
        // Check cache first
        {
            let cache = TOKEN_CACHE.lock().unwrap();
            if let Some(cached) = cache.get(github_token)
                && cached.expires_at > Instant::now()
            {
                return Ok(CopilotCredentials {
                    token: cached.token.clone(),
                    endpoint: cached.api_endpoint.clone(),
                    client: CopilotClient::VsCode,
                    device: CopilotDevice::for_credential(github_token),
                });
            }
        }

        // Exchange GitHub token for Copilot API token
        let resp = self
            .github_request(COPILOT_TOKEN_URL, CopilotClient::VsCode, github_token)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Auth(format!(
                "Copilot token exchange {status}: {text}"
            )));
        }

        let json: Value = resp.json().await?;

        let api_token = json
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| ByokError::Auth("missing token in Copilot response".into()))?
            .to_string();

        let expires_at_unix = json.get("expires_at").and_then(Value::as_i64).unwrap_or(0);

        let ttl = if expires_at_unix > 0 {
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .cast_signed();
            let secs = (expires_at_unix - now_unix).max(0).cast_unsigned();
            Duration::from_secs(secs)
        } else {
            Duration::from_mins(25) // default TTL
        };

        let api_endpoint = json
            .pointer("/endpoints/api")
            .and_then(Value::as_str)
            .map_or_else(
                || self.default_endpoint(),
                |url| url.trim_end_matches('/').to_owned(),
            );

        // Cache the new token
        {
            let mut cache = TOKEN_CACHE.lock().unwrap();
            cache.insert(
                github_token.to_string(),
                CachedToken {
                    token: api_token.clone(),
                    api_endpoint: api_endpoint.clone(),
                    expires_at: Instant::now() + ttl,
                },
            );
        }

        Ok(CopilotCredentials {
            token: api_token,
            endpoint: api_endpoint,
            client: CopilotClient::VsCode,
            device: CopilotDevice::for_credential(github_token),
        })
    }

    /// Fetch quota snapshot for a single GitHub account.
    ///
    /// Returns `(percent_remaining, unlimited)` on success, `None` on any failure.
    async fn fetch_quota(&self, token: &OAuthToken) -> Option<(f64, bool)> {
        let resp = self
            .github_request(
                COPILOT_USER_URL,
                CopilotClient::of(token).ok()?,
                &token.access_token,
            )
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let json: Value = resp.json().await.ok()?;
        let pi = json.pointer("/quota_snapshots/premium_interactions")?;
        let unlimited = pi
            .get("unlimited")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let percent = pi
            .get("percent_remaining")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        Some((percent, unlimited))
    }

    /// Refresh quota for an account if the cached value is stale or missing.
    async fn refresh_quota_if_stale(&self, account_id: &str) {
        // Check if we already have a fresh cache entry.
        {
            let tracker = ACCOUNT_TRACKER.lock().unwrap();
            if let Some(q) = tracker.quotas.get(account_id)
                && q.fetched_at.elapsed() < QUOTA_CACHE_TTL
            {
                return;
            }
        }

        // Fetch the GitHub token for this account.
        let github_token = match self
            .auth
            .get_token_for(&ProviderId::Copilot, account_id)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(account_id, error = %e, "failed to get token for quota fetch");
                return;
            }
        };

        if let Some((percent, unlimited)) = self.fetch_quota(&github_token).await {
            tracing::info!(
                account_id,
                percent_remaining = percent,
                unlimited,
                "fetched copilot quota"
            );
            let mut tracker = ACCOUNT_TRACKER.lock().unwrap();
            tracker.quotas.insert(
                account_id.to_string(),
                CachedQuota {
                    percent_remaining: percent,
                    unlimited,
                    fetched_at: Instant::now(),
                },
            );
        } else {
            tracing::warn!(account_id, "failed to fetch copilot quota, skipping");
        }
    }

    /// Select the best account based on cached quota data.
    ///
    /// Uses sticky selection: keeps the current account until the rebalance
    /// interval elapses, then re-compares all accounts' quotas.
    async fn select_account(&self, accounts: &[AccountInfo]) -> Result<String> {
        {
            let tracker = ACCOUNT_TRACKER.lock().unwrap();

            // Sticky: current is still valid and rebalance interval hasn't elapsed.
            if let Some(ref current) = tracker.current
                && accounts.iter().any(|a| a.account_id == *current)
                && tracker
                    .last_rebalance
                    .is_some_and(|t| t.elapsed() < REBALANCE_INTERVAL)
            {
                return Ok(current.clone());
            }
        }

        // Fetch quotas (skips accounts with fresh cache).
        for account in accounts {
            self.refresh_quota_if_stale(&account.account_id).await;
        }

        // Pick the account with the highest remaining quota.
        let mut tracker = ACCOUNT_TRACKER.lock().unwrap();
        let best = accounts
            .iter()
            .max_by(|a, b| {
                let qa = tracker.quotas.get(&a.account_id);
                let qb = tracker.quotas.get(&b.account_id);
                quota_score(qa)
                    .partial_cmp(&quota_score(qb))
                    .unwrap_or(CmpOrdering::Equal)
            })
            .ok_or_else(|| ByokError::Auth("no copilot accounts available".into()))?;

        tracing::info!(
            account_id = %best.account_id,
            score = quota_score(tracker.quotas.get(&best.account_id)),
            "selected copilot account"
        );

        tracker.current = Some(best.account_id.clone());
        tracker.last_rebalance = Some(Instant::now());
        Ok(best.account_id.clone())
    }

    /// Force the next `credentials()` call to re-evaluate account selection.
    ///
    /// # Panics
    ///
    /// Panics if the account tracker mutex is poisoned.
    pub fn invalidate_current_account() {
        let mut tracker = ACCOUNT_TRACKER.lock().unwrap();
        tracker.last_rebalance = None;
    }

    /// Resolves the credentials for the next Copilot API request.
    ///
    /// A configured `api_key` is a bearer token sent as the default client.
    /// With multiple accounts, selects the account with the most remaining quota.
    /// Otherwise falls back to the active account.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Auth`] if no account is usable or the VS Code
    /// token exchange fails.
    pub async fn credentials(&self) -> Result<CopilotCredentials> {
        if let Some(key) = &self.api_key {
            return Ok(CopilotCredentials {
                token: key.clone(),
                endpoint: self.default_endpoint(),
                client: CopilotClient::default(),
                device: CopilotDevice::for_credential(key),
            });
        }

        let accounts = self.auth.list_accounts(&ProviderId::Copilot).await?;
        let token = if accounts.len() > 1 {
            let account_id = self.select_account(&accounts).await?;
            self.auth
                .get_token_for(&ProviderId::Copilot, &account_id)
                .await?
        } else {
            self.auth.get_token(&ProviderId::Copilot).await?
        };
        self.credentials_for(&token).await
    }

    /// Builds an [`OpenAICompatProvider`] for a single request as `creds`' account.
    ///
    /// Client headers depend on the request, so they are appended after
    /// translation rather than set as `default_headers`.
    fn build_provider(creds: &CopilotCredentials) -> Result<OpenAICompatProvider> {
        let default_headers =
            BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);

        OpenAICompatProvider::new(OpenAICompatConfig {
            name: "copilot".to_owned(),
            http: HttpTransportConfig {
                base_url: creds.endpoint.clone(),
                timeout_seconds: 600,
                default_headers,
            },
            auth: OpenAIAuthConfig {
                api_key: SecretString::from(creds.token.clone()),
                organization: None,
                project: None,
            },
            quirks: Quirks::default(),
        })
        .map_err(|e| ByokError::Config(e.to_string()))
    }
}

#[async_trait]
impl ProviderExecutor for CopilotExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        // Derived from the messages before the request is consumed.
        let conversation = Conversation::from_messages(&request.messages);

        // Translate: BYOKEY ChatRequest → aigw ChatRequest.
        let aigw_request: aigw_core::model::ChatRequest =
            serde_json::from_value(request.into_body())
                .map_err(|e| ByokError::Translation(e.to_string()))?;

        let accounts = self
            .auth
            .list_accounts(&ProviderId::Copilot)
            .await
            .unwrap_or_default();
        let max_attempts = if accounts.len() > 1 {
            accounts.len().min(3)
        } else {
            1
        };

        let mut last_err = None;
        for attempt in 0..max_attempts {
            let creds = match self.credentials().await {
                Ok(c) => c,
                Err(e) => {
                    if max_attempts > 1 {
                        tracing::warn!(attempt, error = %e, "copilot creds failed, trying next account");
                        Self::invalidate_current_account();
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            };

            // Build aigw provider + translator for this account.
            let provider = Self::build_provider(&creds)?;
            let translator = OpenAICompatRequestTranslator::new(&provider)
                .map_err(|e| ByokError::Config(e.to_string()))?;

            // Translate the canonical request to a Copilot HTTP request.
            // aigw handles: URL (`{endpoint}/chat/completions`), static headers,
            // `Authorization: Bearer <token>`, content-type, and body serialization.
            let translated = if stream {
                translator.translate_stream_request(&aigw_request)
            } else {
                translator.translate_request(&aigw_request)
            }
            .map_err(|e| ByokError::Translation(e.to_string()))?;

            // Build the wreq request from aigw's translated URL and headers.
            let mut builder = self.ph.client().post(&translated.url);
            for (name, value) in &translated.headers {
                if let Ok(v) = value.to_str() {
                    builder = builder.header(name.as_str(), v);
                }
            }
            for (name, value) in self.identity.request_headers(&creds, &conversation) {
                builder = builder.header(name, value);
            }
            // Attach the translated body (already serialized JSON bytes by aigw).
            let builder = builder.body(translated.body.to_vec());

            if stream {
                // Option P: raw byte passthrough — stream Copilot SSE bytes to caller
                // unchanged. aigw is used only for URL/header/body building.
                match self.ph.send_passthrough(builder, true).await {
                    Ok(resp) => return Ok(resp),
                    Err(e) => {
                        if !e.is_retryable() || attempt + 1 >= max_attempts {
                            return Err(e);
                        }
                        tracing::warn!(attempt, error = %e, "copilot stream request failed, trying next account");
                        Self::invalidate_current_account();
                        last_err = Some(e);
                    }
                }
            } else {
                // Non-streaming: use aigw's OpenAICompatResponseTranslator.
                let resp = match self.ph.send(builder).await {
                    Ok(r) => r,
                    Err(e) => {
                        if !e.is_retryable() || attempt + 1 >= max_attempts {
                            return Err(e);
                        }
                        tracing::warn!(attempt, error = %e, "copilot request failed, trying next account");
                        Self::invalidate_current_account();
                        last_err = Some(e);
                        continue;
                    }
                };
                let resp_bytes = resp.bytes().await.map_err(ByokError::from)?;
                let aigw_response = OpenAIResponseTranslator
                    .translate_response(http::StatusCode::OK, &resp_bytes)
                    .map_err(|e: aigw_core::error::TranslateError| {
                        ByokError::Translation(e.to_string())
                    })?;
                let value = serde_json::to_value(aigw_response)
                    .map_err(|e| ByokError::Translation(e.to_string()))?;
                return Ok(ProviderResponse::Complete(value));
            }
        }

        tracing::error!(
            attempts = max_attempts,
            "all copilot accounts exhausted for chat request"
        );
        Err(last_err.unwrap_or_else(|| ByokError::Auth("no copilot accounts available".into())))
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Copilot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> CopilotExecutor {
        let (client, auth) = crate::http_util::test_auth();
        CopilotExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[tokio::test]
    async fn token_cache_is_shared_across_executor_instances() {
        // Executors are built per request, so a token exchanged by one must be
        // served from cache to the next without another round trip.
        let github_token = "ghu_token_cache_is_shared_across_executor_instances";
        TOKEN_CACHE.lock().unwrap().insert(
            github_token.to_owned(),
            CachedToken {
                token: "copilot-api-token".to_owned(),
                api_endpoint: "https://api.individual.githubcopilot.com".to_owned(),
                expires_at: Instant::now() + Duration::from_mins(10),
            },
        );

        let creds = make_executor()
            .exchange_and_cache(github_token)
            .await
            .expect("served from cache, no network");
        assert_eq!(creds.token, "copilot-api-token");
        assert_eq!(creds.endpoint, "https://api.individual.githubcopilot.com");
    }
}
