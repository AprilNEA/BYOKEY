//! GitHub Copilot executor — OpenAI-compatible API.
//!
//! Auth: device code flow → GitHub token → exchange for short-lived Copilot API token.
//! Format: `OpenAI` passthrough (no translation needed).
use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    AccountInfo, ByokError, ChatRequest, ProviderId,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use serde_json::Value;
use std::{
    cmp::Ordering as CmpOrdering,
    collections::HashMap,
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

/// How often to re-compare quotas across accounts.
const REBALANCE_INTERVAL: Duration = Duration::from_secs(300); // 5 min

/// Quota cache TTL — avoid re-fetching within this window.
const QUOTA_CACHE_TTL: Duration = Duration::from_secs(300);

/// GitHub Copilot Chat Completions API base URL.
const API_BASE_URL: &str = "https://api.githubcopilot.com";

/// Endpoint to exchange a GitHub OAuth token for a short-lived Copilot API token.
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// Copilot usage/quota endpoint (returns `quota_snapshots`).
const COPILOT_USER_URL: &str = "https://api.github.com/copilot_internal/user";

// Header values matching the VS Code Copilot Chat extension.
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const EDITOR_VERSION: &str = "vscode/1.107.0";
const PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const INTEGRATION_ID: &str = "vscode-chat";
const OPENAI_INTENT: &str = "conversation-panel";
const GITHUB_API_VERSION: &str = "2025-04-01";

/// A cached Copilot API token with its expiry time.
struct CachedToken {
    token: String,
    api_endpoint: String,
    expires_at: Instant,
    /// `true` = Pro/Business/Enterprise, `false` = Free tier.
    is_pro: bool,
}

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
    auth: Arc<AuthManager>,
    /// Cache: GitHub token → short-lived Copilot API token.
    cache: Mutex<HashMap<String, CachedToken>>,
}

impl CopilotExecutor {
    /// Creates a new Copilot executor with an optional API key and auth manager.
    pub fn new(http: rquest::Client, api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            ph: ProviderHttp::new(http),
            api_key,
            auth,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Exchange a GitHub token for a Copilot API token and cache the result.
    ///
    /// Returns `(copilot_api_token, api_endpoint)`.
    async fn exchange_and_cache(&self, github_token: &str) -> Result<(String, String)> {
        // Check cache first
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(github_token)
                && cached.expires_at > Instant::now()
            {
                return Ok((cached.token.clone(), cached.api_endpoint.clone()));
            }
        }

        // Exchange GitHub token for Copilot API token
        let resp = self
            .ph
            .client()
            .get(COPILOT_TOKEN_URL)
            .header("authorization", format!("token {github_token}"))
            .header("accept", "application/json")
            .header("user-agent", USER_AGENT)
            .header("editor-version", EDITOR_VERSION)
            .header("editor-plugin-version", PLUGIN_VERSION)
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
            Duration::from_secs(1500) // default ~25 min
        };

        let api_endpoint = json
            .pointer("/endpoints/api")
            .and_then(Value::as_str)
            .unwrap_or(API_BASE_URL)
            .trim_end_matches('/')
            .to_string();

        // If "copilot_plan" is absent or not "copilot_free", assume Pro+.
        let is_pro = json
            .get("copilot_plan")
            .and_then(Value::as_str)
            .is_none_or(|plan| plan != "copilot_free");

        // Cache the new token
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(
                github_token.to_string(),
                CachedToken {
                    token: api_token.clone(),
                    api_endpoint: api_endpoint.clone(),
                    expires_at: Instant::now() + ttl,
                    is_pro,
                },
            );
        }

        Ok((api_token, api_endpoint))
    }

    /// Obtain a Copilot API token for a specific account.
    async fn copilot_token_for_account(&self, account_id: &str) -> Result<(String, String)> {
        let github_token = self
            .auth
            .get_token_for(&ProviderId::Copilot, account_id)
            .await?
            .access_token;
        self.exchange_and_cache(&github_token).await
    }

    /// Fetch quota snapshot for a single GitHub account.
    ///
    /// Returns `(percent_remaining, unlimited)` on success, `None` on any failure.
    async fn fetch_quota(&self, github_token: &str) -> Option<(f64, bool)> {
        let resp = self
            .ph
            .client()
            .get(COPILOT_USER_URL)
            .header("authorization", format!("token {github_token}"))
            .header("accept", "application/json")
            .header("user-agent", USER_AGENT)
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
            Ok(t) => t.access_token,
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

    /// Force the next `copilot_token()` call to re-evaluate account selection.
    ///
    /// # Panics
    ///
    /// Panics if the account tracker mutex is poisoned.
    pub fn invalidate_current_account() {
        let mut tracker = ACCOUNT_TRACKER.lock().unwrap();
        tracker.last_rebalance = None;
    }

    /// Returns the Copilot API token and base endpoint URL (without path suffix).
    ///
    /// When `api_key` is set it is used directly (skip token exchange).
    /// With multiple accounts, selects the account with the most remaining quota.
    /// Otherwise falls back to the active account.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Auth`] if the token exchange fails.
    ///
    /// # Panics
    ///
    /// Panics if the internal token cache mutex is poisoned.
    pub async fn copilot_token(&self) -> Result<(String, String)> {
        if let Some(key) = &self.api_key {
            return Ok((key.clone(), API_BASE_URL.to_string()));
        }

        let accounts = self.auth.list_accounts(&ProviderId::Copilot).await?;

        if accounts.len() > 1 {
            let account_id = self.select_account(&accounts).await?;
            return self.copilot_token_for_account(&account_id).await;
        }

        // Single or no account: use active account (original behavior).
        let github_token = self
            .auth
            .get_token(&ProviderId::Copilot)
            .await?
            .access_token;
        self.exchange_and_cache(&github_token).await
    }

    /// Obtains the Copilot API token and chat completions URL.
    async fn copilot_creds(&self) -> Result<(String, String)> {
        let (token, endpoint) = self.copilot_token().await?;
        Ok((token, format!("{endpoint}/chat/completions")))
    }

    /// Returns `true` if any cached Copilot token belongs to a Pro/Business/Enterprise plan.
    ///
    /// With multiple accounts, returns `true` if **any** account is Pro+.
    /// Defaults to `true` (Pro) if the plan cannot be determined (e.g. no cached token yet
    /// or the `copilot_plan` field was absent in the token exchange response).
    ///
    /// # Panics
    ///
    /// Panics if the internal token cache mutex is poisoned.
    pub async fn is_pro(&self) -> bool {
        let accounts = self
            .auth
            .list_accounts(&ProviderId::Copilot)
            .await
            .unwrap_or_default();

        if accounts.len() > 1 {
            // Check all cached tokens: any Pro → true.
            let cache = self.cache.lock().unwrap();
            let now = Instant::now();
            let mut found_any = false;
            for cached in cache.values() {
                if cached.expires_at > now {
                    found_any = true;
                    if cached.is_pro {
                        return true;
                    }
                }
            }
            // If we found cached tokens but none are Pro, return false.
            if found_any {
                return false;
            }
            // No cached tokens yet: conservative default.
            return true;
        }

        // Single account: original behavior.
        if let Ok(github_token) = self
            .auth
            .get_token(&ProviderId::Copilot)
            .await
            .map(|t| t.access_token)
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(&github_token)
                && cached.expires_at > Instant::now()
            {
                return cached.is_pro;
            }
        }
        true // conservative default: assume Pro
    }

    /// Returns the `X-Initiator` header value based on whether the request
    /// contains any assistant/tool messages (agent) or only user messages.
    fn initiator(request: &ChatRequest) -> &'static str {
        let is_agent = request.messages.iter().any(|m| {
            matches!(
                m.get("role").and_then(Value::as_str),
                Some("assistant" | "tool")
            )
        });
        if is_agent { "agent" } else { "user" }
    }
}

#[async_trait]
impl ProviderExecutor for CopilotExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let initiator = Self::initiator(&request);
        let body = request.into_body();

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
            let creds = self.copilot_creds().await;
            let (token, url) = match creds {
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

            let builder = self
                .ph
                .client()
                .post(&url)
                .header("authorization", format!("Bearer {token}"))
                .header("user-agent", USER_AGENT)
                .header("editor-version", EDITOR_VERSION)
                .header("editor-plugin-version", PLUGIN_VERSION)
                .header("openai-intent", OPENAI_INTENT)
                .header("copilot-integration-id", INTEGRATION_ID)
                .header("x-github-api-version", GITHUB_API_VERSION)
                .header("x-initiator", initiator)
                .header("content-type", "application/json")
                .json(&body);

            match self.ph.send_passthrough(builder, stream).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if !e.is_retryable() || attempt + 1 >= max_attempts {
                        return Err(e);
                    }
                    tracing::warn!(attempt, error = %e, "copilot request failed, trying next account");
                    Self::invalidate_current_account();
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| ByokError::Auth("no copilot accounts available".into())))
    }

    fn supported_models(&self) -> Vec<String> {
        registry::copilot_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;
    use rquest::Client;

    fn make_executor() -> CopilotExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store, rquest::Client::new()));
        CopilotExecutor::new(Client::new(), None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_initiator_user() {
        let req: ChatRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        assert_eq!(CopilotExecutor::initiator(&req), "user");
    }

    #[test]
    fn test_initiator_agent() {
        let req: ChatRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ]
        }))
        .unwrap();
        assert_eq!(CopilotExecutor::initiator(&req), "agent");
    }
}
