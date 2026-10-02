//! GitHub Copilot accounts: credentials, quota-aware account selection, the
//! model catalog, and what each account's organisation policy rejects.
//!
//! Auth: device code flow → GitHub token. `OpenCode` tokens authenticate API
//! requests directly; VS Code tokens are first exchanged for a short-lived
//! Copilot API token. The proxy sends the Anthropic Messages requests
//! themselves, with the headers [`CopilotIdentity`] provides.
//!
//! Every account is served from the API host GitHub names for it in
//! `/copilot_internal/user` (`endpoints.api`: `api.githubcopilot.com` for
//! individual seats, `api.enterprise.githubcopilot.com` or a GHE host for
//! organisation seats), as VS Code does.
mod device;
mod headers;

pub use device::CopilotDevice;
pub use headers::{Conversation, CopilotIdentity, CopilotVersions};

use byokey_auth::AuthManager;
use byokey_types::{
    AccountInfo, AccountToken, ByokError, CopilotClient, DEFAULT_ACCOUNT, OAuthToken, ProviderId,
    Result, millis,
};
use serde_json::Value;
use std::{
    cmp::Ordering as CmpOrdering,
    collections::{HashMap, HashSet},
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

/// The Copilot API host when GitHub names none for the account.
const DEFAULT_BASE_URL: &str = "https://api.githubcopilot.com";

/// How long an account's API host from `/copilot_internal/user` is reused.
#[allow(
    clippy::duration_suboptimal_units,
    reason = "`Duration::from_hours` is not a const fn on stable"
)]
const ENDPOINT_TTL: Duration = Duration::from_secs(6 * 3600);

/// How long the default host stands in after `/copilot_internal/user`
/// failed, before GitHub is asked again.
const ENDPOINT_RETRY: Duration = Duration::from_mins(1);

/// API host per `OpenCode` credential, from `/copilot_internal/user`, and
/// when to ask again. Process-wide because upstreams are built per request.
static ENDPOINTS: LazyLock<Mutex<HashMap<String, (Instant, String)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Anthropic server tool types an account's organisation policy rejected,
/// per credential, so later requests leave them out instead of failing.
/// Learned from the 400 (see [`CopilotCredentials::reject_tool`]); Copilot
/// exposes no flag for it up front.
static REJECTED_TOOLS: LazyLock<Mutex<HashMap<String, HashSet<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
    /// The stored account the requests go out as, which usage is recorded
    /// against; [`DEFAULT_ACCOUNT`] for a configured key.
    pub account_id: String,
    /// The credential the account was resolved from (a GitHub token or a
    /// configured key), keying what BYOKEY remembers about the account.
    credential: String,
}

impl CopilotCredentials {
    /// Server tool types this account's policy rejected earlier, by
    /// `type` prefix (`web_search`, `web_fetch`).
    ///
    /// # Panics
    ///
    /// Panics if the rejected-tools mutex is poisoned.
    #[must_use]
    pub fn rejected_tools(&self) -> HashSet<String> {
        REJECTED_TOOLS
            .lock()
            .unwrap()
            .get(&self.credential)
            .cloned()
            .unwrap_or_default()
    }

    /// Remember that this account's policy rejects the server tool `kind`
    /// (`web_search`, `web_fetch`). Returns whether it is news.
    ///
    /// # Panics
    ///
    /// Panics if the rejected-tools mutex is poisoned.
    pub fn reject_tool(&self, kind: &str) -> bool {
        REJECTED_TOOLS
            .lock()
            .unwrap()
            .entry(self.credential.clone())
            .or_default()
            .insert(kind.to_owned())
    }
}

/// VS Code GitHub token → short-lived Copilot API token.
///
/// Process-wide because upstreams are built per request; an instance-owned
/// cache would never be hit and every request would re-run the exchange.
static TOKEN_CACHE: LazyLock<Mutex<HashMap<String, CachedToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A model the account's Copilot catalog offers to users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopilotModel {
    pub id: String,
    pub name: String,
    /// Served on Copilot's Anthropic-format `/v1/messages`.
    pub messages: bool,
    /// Served on Copilot's native `/responses` endpoint.
    pub responses: bool,
    /// Context window in tokens, when the catalog states it.
    pub context_window: Option<u64>,
}

/// A `/models` entry as Copilot sends it.
#[derive(serde::Deserialize)]
struct CatalogEntry {
    id: String,
    name: Option<String>,
    /// Offered to users; internal models (embeddings, retired snapshots)
    /// are not.
    #[serde(default)]
    model_picker_enabled: bool,
    /// `null` for internal models.
    supported_endpoints: Option<Vec<String>>,
    capabilities: Option<Capabilities>,
}

#[derive(serde::Deserialize)]
struct Capabilities {
    limits: Option<Limits>,
}

#[derive(serde::Deserialize)]
struct Limits {
    max_context_window_tokens: Option<u64>,
}

impl CatalogEntry {
    /// The model, if Copilot offers it to users.
    fn offered(self) -> Option<CopilotModel> {
        self.model_picker_enabled.then(|| CopilotModel {
            name: self.name.unwrap_or_else(|| self.id.clone()),
            id: self.id,
            messages: self
                .supported_endpoints
                .as_ref()
                .is_some_and(|eps| eps.iter().any(|ep| ep == "/v1/messages")),
            responses: self
                .supported_endpoints
                .as_ref()
                .is_some_and(|eps| eps.iter().any(|ep| ep == "/responses")),
            context_window: self
                .capabilities
                .and_then(|c| c.limits)
                .and_then(|l| l.max_context_window_tokens),
        })
    }
}

/// How long a Copilot model catalog is reused.
#[allow(
    clippy::duration_suboptimal_units,
    reason = "`Duration::from_mins` is not a const fn on stable"
)]
const MODELS_TTL: Duration = Duration::from_secs(5 * 60);

/// Credential → its catalog and when it was fetched. Process-wide because
/// upstreams are built per request.
type ModelsCache = HashMap<String, (Instant, Vec<CopilotModel>)>;

static MODELS_CACHE: LazyLock<Mutex<ModelsCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

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

/// Send `builder`, turning a non-success status into [`ByokError::Upstream`].
async fn send(builder: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let resp = builder.send().await?;
    if resp.status().is_success() {
        Ok(resp)
    } else {
        Err(ByokError::from_response(resp).await)
    }
}

/// The GitHub Copilot accounts BYOKEY can send requests as.
pub struct CopilotUpstream {
    http: reqwest::Client,
    api_key: Option<String>,
    base_url: Option<String>,
    auth: Arc<AuthManager>,
    identity: CopilotIdentity,
}

#[bon::bon]
impl CopilotUpstream {
    /// An `api_key` is used as a Copilot API bearer token as is; otherwise
    /// the stored GitHub logins are used.
    #[builder]
    pub fn new(
        http: reqwest::Client,
        auth: Arc<AuthManager>,
        api_key: Option<String>,
        base_url: Option<String>,
        identity: Option<CopilotIdentity>,
    ) -> Self {
        Self {
            http,
            api_key,
            base_url,
            auth,
            identity: identity.unwrap_or_default(),
        }
    }

    /// The client identity requests present.
    #[must_use]
    pub fn identity(&self) -> &CopilotIdentity {
        &self.identity
    }

    /// A GET against `api.github.com` authenticated with `client`'s GitHub token.
    fn github_request(
        &self,
        url: &str,
        client: CopilotClient,
        github_token: &str,
    ) -> reqwest::RequestBuilder {
        let mut builder = self
            .http
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

    /// The credentials for requests as `account_id`, which holds `token`.
    async fn credentials_for(
        &self,
        token: &OAuthToken,
        account_id: &str,
    ) -> Result<CopilotCredentials> {
        match CopilotClient::of(token)? {
            CopilotClient::OpenCode => Ok(CopilotCredentials {
                token: token.access_token.clone(),
                endpoint: self.account_endpoint(token).await,
                client: CopilotClient::OpenCode,
                device: CopilotDevice::for_credential(&token.access_token),
                account_id: account_id.to_owned(),
                credential: token.access_token.clone(),
            }),
            CopilotClient::VsCode => {
                self.exchange_and_cache(&token.access_token, account_id)
                    .await
            }
        }
    }

    /// The API host GitHub names for `token`'s account, cached for
    /// [`ENDPOINT_TTL`]. A configured `base_url` wins. When GitHub cannot be
    /// asked, the default host stands in for [`ENDPOINT_RETRY`] only, so a
    /// network hiccup does not keep the account off the host GitHub names.
    #[tracing::instrument(level = "debug", skip_all)]
    async fn account_endpoint(&self, token: &OAuthToken) -> String {
        if let Some(url) = &self.base_url {
            return url.trim_end_matches('/').to_owned();
        }
        if let Some((expires, endpoint)) = ENDPOINTS.lock().unwrap().get(&token.access_token)
            && Instant::now() < *expires
        {
            return endpoint.clone();
        }
        let started = Instant::now();
        let (endpoint, ttl) = match self.user_info(token).await {
            Ok(user) => {
                let endpoint = user
                    .pointer("/endpoints/api")
                    .and_then(Value::as_str)
                    .map_or(DEFAULT_BASE_URL, |url| url.trim_end_matches('/'))
                    .to_owned();
                tracing::info!(
                    %endpoint,
                    duration_ms = millis(started.elapsed()),
                    "looked up the Copilot account's API host"
                );
                (endpoint, ENDPOINT_TTL)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    fallback = DEFAULT_BASE_URL,
                    retry_secs = ENDPOINT_RETRY.as_secs(),
                    "could not look up the Copilot account's API host"
                );
                (DEFAULT_BASE_URL.to_owned(), ENDPOINT_RETRY)
            }
        };
        ENDPOINTS.lock().unwrap().insert(
            token.access_token.clone(),
            (Instant::now() + ttl, endpoint.clone()),
        );
        endpoint
    }

    /// `/copilot_internal/user` for `token`'s account.
    async fn user_info(&self, token: &OAuthToken) -> Result<Value> {
        let request = self.github_request(
            COPILOT_USER_URL,
            CopilotClient::of(token)?,
            &token.access_token,
        );
        Ok(send(request).await?.json().await?)
    }

    /// Exchange `account_id`'s VS Code GitHub token for a Copilot API token
    /// and cache the result.
    #[tracing::instrument(level = "debug", skip_all, fields(account = account_id))]
    async fn exchange_and_cache(
        &self,
        github_token: &str,
        account_id: &str,
    ) -> Result<CopilotCredentials> {
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
                    account_id: account_id.to_owned(),
                    credential: github_token.to_owned(),
                });
            }
        }

        // Exchange GitHub token for Copilot API token
        let started = Instant::now();
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

        tracing::info!(
            account = account_id,
            endpoint = %api_endpoint,
            valid_for_secs = ttl.as_secs(),
            duration_ms = millis(started.elapsed()),
            "exchanged a Copilot API token"
        );
        Ok(CopilotCredentials {
            token: api_token,
            endpoint: api_endpoint,
            client: CopilotClient::VsCode,
            device: CopilotDevice::for_credential(github_token),
            account_id: account_id.to_owned(),
            credential: github_token.to_owned(),
        })
    }

    /// Fetch quota snapshot for a single GitHub account.
    ///
    /// Returns `(percent_remaining, unlimited)`, or `None` when the account
    /// reports no premium-request quota.
    async fn fetch_quota(&self, token: &OAuthToken) -> Result<Option<(f64, bool)>> {
        let json = self.user_info(token).await?;
        let Some(pi) = json.pointer("/quota_snapshots/premium_interactions") else {
            return Ok(None);
        };
        let unlimited = pi
            .get("unlimited")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let percent = pi
            .get("percent_remaining")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        Ok(Some((percent, unlimited)))
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
            .get_token_for(ProviderId::Copilot, account_id)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(account = account_id, error = %e, "failed to get token for quota fetch");
                return;
            }
        };

        let started = Instant::now();
        match self.fetch_quota(&github_token).await {
            Ok(Some((percent, unlimited))) => {
                tracing::info!(
                    account = account_id,
                    percent_remaining = percent,
                    unlimited,
                    duration_ms = millis(started.elapsed()),
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
            }
            Ok(None) => {
                tracing::warn!(
                    account = account_id,
                    "copilot account reports no premium-request quota, skipping"
                );
            }
            Err(e) => {
                tracing::warn!(account = account_id, error = %e, "failed to fetch copilot quota, skipping");
            }
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
            account = %best.account_id,
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

    /// Drop the cached Copilot API token behind `creds`, so the next
    /// `credentials()` call exchanges a fresh one. Returns whether there was
    /// one to drop: an API key or `OpenCode` token is not exchanged and cannot
    /// be refreshed here.
    ///
    /// # Panics
    ///
    /// Panics if the token cache mutex is poisoned.
    pub fn forget_token(creds: &CopilotCredentials) -> bool {
        let mut cache = TOKEN_CACHE.lock().unwrap();
        let before = cache.len();
        cache.retain(|_, cached| cached.token != creds.token);
        cache.len() < before
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
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn credentials(&self) -> Result<CopilotCredentials> {
        if let Some(key) = &self.api_key {
            return Ok(CopilotCredentials {
                token: key.clone(),
                endpoint: self.default_endpoint(),
                client: CopilotClient::default(),
                device: CopilotDevice::for_credential(key),
                account_id: DEFAULT_ACCOUNT.to_owned(),
                credential: key.clone(),
            });
        }

        let accounts = self.auth.list_accounts(ProviderId::Copilot).await?;
        let AccountToken { account_id, token } = if accounts.len() > 1 {
            let account_id = self.select_account(&accounts).await?;
            let token = self
                .auth
                .get_token_for(ProviderId::Copilot, &account_id)
                .await?;
            AccountToken { account_id, token }
        } else {
            self.auth
                .get_token_with_account(ProviderId::Copilot)
                .await?
        };
        self.credentials_for(&token, &account_id).await
    }

    /// The account's live model catalog (`/models`), cached for
    /// `MODELS_TTL` per credential.
    ///
    /// # Errors
    ///
    /// Returns an error if there is no usable account or the listing fails.
    ///
    /// # Panics
    ///
    /// Panics if the catalog cache mutex is poisoned.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn models(&self) -> Result<Vec<CopilotModel>> {
        #[derive(serde::Deserialize)]
        struct Listing {
            data: Vec<CatalogEntry>,
        }
        let creds = self.credentials().await?;
        if let Some((at, models)) = MODELS_CACHE.lock().unwrap().get(&creds.token)
            && at.elapsed() < MODELS_TTL
        {
            return Ok(models.clone());
        }
        let mut builder = self
            .http
            .get(format!("{}/models", creds.endpoint))
            .header("authorization", format!("Bearer {}", creds.token));
        for (name, value) in self
            .identity
            .request_headers(&creds, &Conversation::from_messages(&[]))
        {
            builder = builder.header(name, value);
        }
        let started = Instant::now();
        let listing: Listing = send(builder).await?.json().await?;
        let models: Vec<CopilotModel> = listing
            .data
            .into_iter()
            .filter_map(CatalogEntry::offered)
            .collect();
        tracing::info!(
            account = %creds.account_id,
            models = models.len(),
            duration_ms = millis(started.elapsed()),
            "fetched the Copilot model catalog"
        );
        MODELS_CACHE
            .lock()
            .unwrap()
            .insert(creds.token, (Instant::now(), models.clone()));
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_entries_become_offered_models() {
        // Trimmed from a live `/models` response.
        let entries = serde_json::json!([
            {
                "id": "claude-opus-5.5",
                "name": "Claude Opus 5.5",
                "model_picker_enabled": true,
                "supported_endpoints": ["/v1/messages", "/chat/completions"],
                "capabilities": {"limits": {"max_context_window_tokens": 1_000_000}}
            },
            {
                "id": "gpt-5.4",
                "model_picker_enabled": true,
                "supported_endpoints": ["/responses", "/chat/completions"]
            },
            {"id": "gpt-4o", "model_picker_enabled": false, "supported_endpoints": null}
        ]);
        let entries: Vec<CatalogEntry> = serde_json::from_value(entries).unwrap();
        let models: Vec<CopilotModel> = entries
            .into_iter()
            .filter_map(CatalogEntry::offered)
            .collect();
        assert_eq!(
            models,
            [
                CopilotModel {
                    id: "claude-opus-5.5".into(),
                    name: "Claude Opus 5.5".into(),
                    messages: true,
                    responses: false,
                    context_window: Some(1_000_000),
                },
                CopilotModel {
                    id: "gpt-5.4".into(),
                    name: "gpt-5.4".into(),
                    messages: false,
                    responses: true,
                    context_window: None,
                },
            ],
            "internal models are dropped; unnamed ones go by their id"
        );
    }

    fn make_upstream() -> CopilotUpstream {
        let auth = Arc::new(AuthManager::new(
            Arc::new(byokey_store::InMemoryTokenStore::new()),
            reqwest::Client::new(),
        ));
        CopilotUpstream::builder()
            .http(reqwest::Client::new())
            .auth(auth)
            .build()
    }

    #[test]
    fn forgetting_a_token_drops_only_exchanged_ones() {
        let github_token = "ghu_forgetting_a_token_drops_only_exchanged_ones";
        TOKEN_CACHE.lock().unwrap().insert(
            github_token.to_owned(),
            CachedToken {
                token: "copilot-token-to-forget".to_owned(),
                api_endpoint: DEFAULT_BASE_URL.to_owned(),
                expires_at: Instant::now() + Duration::from_mins(10),
            },
        );
        let creds = CopilotCredentials {
            token: "copilot-token-to-forget".to_owned(),
            endpoint: DEFAULT_BASE_URL.to_owned(),
            client: CopilotClient::VsCode,
            device: CopilotDevice::for_credential(github_token),
            account_id: DEFAULT_ACCOUNT.to_owned(),
            credential: github_token.to_owned(),
        };
        assert!(CopilotUpstream::forget_token(&creds));
        assert!(!TOKEN_CACHE.lock().unwrap().contains_key(github_token));
        assert!(
            !CopilotUpstream::forget_token(&creds),
            "nothing left to forget"
        );
    }

    #[tokio::test]
    async fn token_cache_is_shared_across_executor_instances() {
        // Upstreams are built per request, so a token exchanged by one must be
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

        let creds = make_upstream()
            .exchange_and_cache(github_token, "work")
            .await
            .expect("served from cache, no network");
        assert_eq!(creds.token, "copilot-api-token");
        assert_eq!(creds.endpoint, "https://api.individual.githubcopilot.com");
        assert_eq!(creds.account_id, "work");
    }

    #[tokio::test]
    async fn a_cached_account_endpoint_is_used_without_a_round_trip() {
        let token = OAuthToken::new("gho_cached_endpoint").with_client("opencode");
        ENDPOINTS.lock().unwrap().insert(
            token.access_token.clone(),
            (
                Instant::now() + ENDPOINT_TTL,
                "https://api.enterprise.githubcopilot.com".to_owned(),
            ),
        );
        let creds = make_upstream()
            .credentials_for(&token, DEFAULT_ACCOUNT)
            .await
            .expect("served from cache, no network");
        assert_eq!(creds.endpoint, "https://api.enterprise.githubcopilot.com");
        assert_eq!(creds.client, CopilotClient::OpenCode);

        // A configured base_url overrides whatever GitHub names.
        let auth = Arc::new(AuthManager::new(
            Arc::new(byokey_store::InMemoryTokenStore::new()),
            reqwest::Client::new(),
        ));
        let pinned = CopilotUpstream::builder()
            .http(reqwest::Client::new())
            .auth(auth)
            .base_url("https://copilot-api.ghe.example/".to_owned())
            .build();
        let creds = pinned
            .credentials_for(&token, DEFAULT_ACCOUNT)
            .await
            .unwrap();
        assert_eq!(creds.endpoint, "https://copilot-api.ghe.example");
    }

    #[tokio::test]
    async fn an_unanswered_endpoint_lookup_is_retried_soon() {
        // Through a closed port every request fails at once, as offline.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let offline = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://{closed}")).unwrap())
            .build()
            .unwrap();
        let upstream = CopilotUpstream::builder()
            .http(offline)
            .auth(Arc::new(AuthManager::new(
                Arc::new(byokey_store::InMemoryTokenStore::new()),
                reqwest::Client::new(),
            )))
            .build();
        let token = OAuthToken::new("gho_unanswered_endpoint_lookup").with_client("opencode");

        let creds = upstream
            .credentials_for(&token, DEFAULT_ACCOUNT)
            .await
            .unwrap();
        assert_eq!(creds.endpoint, DEFAULT_BASE_URL);
        let (expires, _) = ENDPOINTS.lock().unwrap()[&token.access_token].clone();
        assert!(
            expires <= Instant::now() + ENDPOINT_RETRY,
            "the default host stands in briefly, not for ENDPOINT_TTL"
        );
    }

    #[tokio::test]
    async fn credentials_name_the_account_they_were_resolved_from() {
        let auth = Arc::new(AuthManager::new(
            Arc::new(byokey_store::InMemoryTokenStore::new()),
            reqwest::Client::new(),
        ));
        let token = OAuthToken::new("gho_credentials_name_the_account").with_client("opencode");
        auth.save_token_for(ProviderId::Copilot, "work", None, token.clone())
            .await
            .unwrap();
        ENDPOINTS.lock().unwrap().insert(
            token.access_token.clone(),
            (Instant::now() + ENDPOINT_TTL, DEFAULT_BASE_URL.to_owned()),
        );
        let upstream = CopilotUpstream::builder()
            .http(reqwest::Client::new())
            .auth(auth)
            .build();
        let creds = upstream.credentials().await.expect("no network needed");
        assert_eq!(creds.account_id, "work");
    }

    #[test]
    fn rejected_tools_are_remembered_per_credential() {
        let a = CopilotCredentials {
            token: "t".into(),
            endpoint: DEFAULT_BASE_URL.into(),
            client: CopilotClient::OpenCode,
            device: CopilotDevice::for_credential("gho_a"),
            account_id: DEFAULT_ACCOUNT.into(),
            credential: "gho_rejected_tools_a".into(),
        };
        let b = CopilotCredentials {
            credential: "gho_rejected_tools_b".into(),
            ..a.clone()
        };
        assert!(a.rejected_tools().is_empty());
        assert!(a.reject_tool("web_search"), "first time is news");
        assert!(!a.reject_tool("web_search"), "second time is not");
        assert_eq!(a.rejected_tools(), HashSet::from(["web_search".to_owned()]));
        assert!(
            b.rejected_tools().is_empty(),
            "another account is unaffected"
        );
    }
}
