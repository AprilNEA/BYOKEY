//! GitHub Copilot executor — OpenAI-compatible API.
//!
//! Auth: device code flow → GitHub token → exchange for short-lived Copilot API token.
//! Format: `OpenAI` passthrough (no translation needed).
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ProviderId,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use futures_util::StreamExt as _;
use rquest::Client;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// GitHub Copilot Chat Completions API base URL.
const API_BASE_URL: &str = "https://api.githubcopilot.com";

/// Endpoint to exchange a GitHub OAuth token for a short-lived Copilot API token.
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

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
}

/// Executor for the GitHub Copilot API.
pub struct CopilotExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
    /// Cache: GitHub token → short-lived Copilot API token.
    cache: Mutex<HashMap<String, CachedToken>>,
}

impl CopilotExecutor {
    /// Creates a new Copilot executor with an optional API key and auth manager.
    pub fn new(api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http: Client::new(),
            api_key,
            auth,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Obtains the Copilot API token and endpoint to use for a request.
    ///
    /// When `api_key` is set it is used directly (skip token exchange).
    /// Otherwise the stored GitHub token is exchanged for a short-lived
    /// Copilot API token via `api.github.com/copilot_internal/v2/token`.
    async fn copilot_creds(&self) -> Result<(String, String)> {
        if let Some(key) = &self.api_key {
            return Ok((key.clone(), format!("{API_BASE_URL}/chat/completions")));
        }

        let github_token = self
            .auth
            .get_token(&ProviderId::Copilot)
            .await?
            .access_token;

        // Check cache
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(&github_token)
                && cached.expires_at > Instant::now()
            {
                return Ok((
                    cached.token.clone(),
                    format!("{}/chat/completions", cached.api_endpoint),
                ));
            }
        }

        // Exchange GitHub token for Copilot API token
        let resp = self
            .http
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

        let chat_url = format!("{api_endpoint}/chat/completions");

        // Cache the new token
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(
                github_token,
                CachedToken {
                    token: api_token.clone(),
                    api_endpoint,
                    expires_at: Instant::now() + ttl,
                },
            );
        }

        Ok((api_token, chat_url))
    }

    /// Returns the `X-Initiator` header value based on whether the request
    /// contains any assistant/tool messages (agent) or only user messages.
    fn initiator(body: &Value) -> &'static str {
        let is_agent = body
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|msgs| {
                msgs.iter().any(|m| {
                    matches!(
                        m.get("role").and_then(Value::as_str),
                        Some("assistant" | "tool")
                    )
                })
            });
        if is_agent { "agent" } else { "user" }
    }
}

#[async_trait]
impl ProviderExecutor for CopilotExecutor {
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = request;
        body["stream"] = Value::Bool(stream);

        let initiator = Self::initiator(&body);
        let (token, url) = self.copilot_creds().await?;

        let resp = self
            .http
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
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Http(format!("Copilot API {status}: {text}")));
        }

        if stream {
            let byte_stream: ByteStream = Box::pin(
                resp.bytes_stream()
                    .map(|r| r.map_err(ByokError::from)),
            );
            Ok(ProviderResponse::Stream(byte_stream))
        } else {
            let json: Value = resp.json().await?;
            Ok(ProviderResponse::Complete(json))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::copilot_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> CopilotExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        CopilotExecutor::new(None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_initiator_user() {
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": "hi"}]
        });
        assert_eq!(CopilotExecutor::initiator(&body), "user");
    }

    #[test]
    fn test_initiator_agent() {
        let body = serde_json::json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ]
        });
        assert_eq!(CopilotExecutor::initiator(&body), "agent");
    }
}
