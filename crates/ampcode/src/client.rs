//! `AmpcodeClient` — the main entry point for Ampcode API calls.

use crate::TokenProvider;
use crate::error::{AmpcodeError, Result};
use crate::types::balance::BalanceInfo;
use crate::types::rpc::{BalanceInfoRaw, RpcRequest, RpcResponseEnvelope};
use crate::types::thread::Thread;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Base URL for all Ampcode API calls.
const BASE_URL: &str = "https://ampcode.com";

/// Rust client for the Ampcode API.
///
/// Create with [`AmpcodeClient::new`] for a simple static token or
/// [`AmpcodeClient::with_client`] for a custom HTTP client and token provider.
///
/// ```rust,no_run
/// use ampcode::AmpcodeClient;
///
/// let client = AmpcodeClient::new("sgamp_user_...".to_string());
/// ```
pub struct AmpcodeClient {
    http: Client,
    base_url: String,
    token: Box<dyn TokenProvider>,
}

impl AmpcodeClient {
    /// Create a client with a static bearer token.
    pub fn new(token: impl TokenProvider + 'static) -> Self {
        Self {
            http: Client::new(),
            base_url: BASE_URL.to_string(),
            token: Box::new(token),
        }
    }

    /// Create a client with a custom `reqwest::Client` and token provider.
    ///
    /// Use this when you need to configure timeouts, proxies, or a custom TLS
    /// stack.
    pub fn with_client(http: Client, token: impl TokenProvider + 'static) -> Self {
        Self {
            http,
            base_url: BASE_URL.to_string(),
            token: Box::new(token),
        }
    }

    /// Override the base URL (useful for testing or self-hosted instances).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Return the current bearer token string.
    fn bearer(&self) -> String {
        self.token.token()
    }

    /// Check an HTTP response status and return an `Api` error for non-2xx.
    async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(AmpcodeError::Api {
            status: status.as_u16(),
            body,
        })
    }

    // ── JSON-RPC ──────────────────────────────────────────────────────────────

    /// Call an arbitrary JSON-RPC method on `POST /api/internal`.
    ///
    /// This is the generic escape hatch for undocumented or future methods.
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::Api`] for non-2xx responses,
    /// [`AmpcodeError::Json`] for parse failures, or [`AmpcodeError::Http`]
    /// for transport errors.
    pub async fn rpc<R: DeserializeOwned>(&self, method: &str, params: Option<Value>) -> Result<R> {
        let body = RpcRequest { method, params };
        let resp = self
            .http
            .post(format!("{}/api/internal", self.base_url))
            .bearer_auth(self.bearer())
            .json(&body)
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        Ok(resp.json::<R>().await?)
    }

    // ── Balance ───────────────────────────────────────────────────────────────

    /// Fetch the current account balance.
    ///
    /// Calls `userDisplayBalanceInfo` and parses the `displayText` string
    /// into a structured [`BalanceInfo`].
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::BalanceParse`] if the API returns a format not
    /// recognized by this library version.
    pub async fn balance(&self) -> Result<BalanceInfo> {
        let envelope: RpcResponseEnvelope<BalanceInfoRaw> =
            self.rpc("userDisplayBalanceInfo", None).await?;
        BalanceInfo::parse(envelope.result.display_text)
    }

    /// Fetch the raw `displayText` string without parsing.
    ///
    /// Useful as a fallback when [`balance`](Self::balance) fails with
    /// [`AmpcodeError::BalanceParse`].
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::Api`] or [`AmpcodeError::Http`] on failure.
    pub async fn balance_display_text(&self) -> Result<String> {
        let envelope: RpcResponseEnvelope<BalanceInfoRaw> =
            self.rpc("userDisplayBalanceInfo", None).await?;
        Ok(envelope.result.display_text)
    }

    // ── Thread API ────────────────────────────────────────────────────────────

    /// Get a thread by ID from the API.
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::Api`] with status 404 if the thread does not
    /// exist.
    pub async fn get_thread(&self, id: &str) -> Result<Thread> {
        let resp = self
            .http
            .get(format!("{}/api/threads/{id}", self.base_url))
            .bearer_auth(self.bearer())
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        Ok(resp.json::<Thread>().await?)
    }

    /// Get a thread as rendered Markdown text.
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::Api`] or [`AmpcodeError::Http`] on failure.
    pub async fn get_thread_markdown(&self, id: &str) -> Result<String> {
        let resp = self
            .http
            .get(format!("{}/api/threads/{id}.md", self.base_url))
            .bearer_auth(self.bearer())
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        Ok(resp.text().await?)
    }

    /// List threads by user ID.
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::Api`] or [`AmpcodeError::Http`] on failure.
    pub async fn list_threads_by_user(&self, user_id: &str) -> Result<Vec<Thread>> {
        let resp = self
            .http
            .get(format!("{}/api/threads", self.base_url))
            .query(&[("createdByUserID", user_id)])
            .bearer_auth(self.bearer())
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        Ok(resp.json::<Vec<Thread>>().await?)
    }

    /// Search threads by query string.
    ///
    /// `limit` and `offset` default to the API's own defaults when `None`.
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::Api`] or [`AmpcodeError::Http`] on failure.
    pub async fn find_threads(
        &self,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Thread>> {
        let mut req = self
            .http
            .get(format!("{}/api/threads/find", self.base_url))
            .query(&[("q", query)])
            .bearer_auth(self.bearer());
        if let Some(l) = limit {
            req = req.query(&[("limit", l)]);
        }
        if let Some(o) = offset {
            req = req.query(&[("offset", o)]);
        }
        let resp = req.send().await?;
        let resp = Self::check(resp).await?;
        Ok(resp.json::<Vec<Thread>>().await?)
    }

    // ── GitHub integration ────────────────────────────────────────────────────

    /// Check GitHub authentication status.
    ///
    /// # Errors
    ///
    /// Returns [`AmpcodeError::Api`] or [`AmpcodeError::Http`] on failure.
    pub async fn github_auth_status(&self) -> Result<Value> {
        let resp = self
            .http
            .get(format!("{}/api/internal/github-auth-status", self.base_url))
            .bearer_auth(self.bearer())
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        Ok(resp.json::<Value>().await?)
    }
}
