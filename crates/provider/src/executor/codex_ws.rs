//! Codex WebSocket executor.
//!
//! Connects to `ChatGPT`'s WebSocket endpoint and translates the Codex
//! Responses protocol to `OpenAI` chat completion SSE format.  Uses the same
//! SSE translator as the HTTP executor
//! ([`super::codex::translate_codex_responses_sse`]).

use crate::registry;
use aigw_openai::{ResponsesRequestConfig, build_responses_create_request};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ChatRequest, ProviderId,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _, stream::try_unfold};
use serde_json::Value;
use std::sync::Arc;
use tokio_tungstenite::tungstenite;

use super::codex::translate_codex_responses_sse;

/// `ChatGPT` WebSocket endpoint for the Codex Responses API.
const WS_URL: &str = "wss://chatgpt.com/backend-api/codex/ws";

/// Feature flag header sent to enable the WebSocket protocol.
const WS_BETA: &str = "responses_websockets=2026-02-06";

/// User-Agent matching the Codex CLI binary.
const CODEX_USER_AGENT: &str = "codex-tui/0.120.0 (Mac OS 26.0.1; arm64) Apple_Terminal/464";

/// WebSocket-based executor for the Codex API.
///
/// Each call to [`chat_completion`] opens a fresh WebSocket connection
/// (no connection pooling in this initial implementation).
pub struct CodexWsExecutor {
    auth: Arc<AuthManager>,
}

impl CodexWsExecutor {
    /// Creates a new WebSocket executor backed by the given [`AuthManager`].
    pub fn new(auth: Arc<AuthManager>) -> Self {
        Self { auth }
    }

    /// Retrieves an OAuth access token from the auth manager.
    async fn token(&self) -> Result<String> {
        let tok = self.auth.get_token(&ProviderId::Codex).await?;
        Ok(tok.access_token)
    }
}

#[async_trait]
impl ProviderExecutor for CodexWsExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let token = self.token().await?;

        // Translate the OpenAI chat request to Codex format via aigateway's
        // ResponsesRequestTranslator (codex preset).
        let aigw_request: aigw_core::model::ChatRequest =
            serde_json::from_value(request.into_body())
                .map_err(|e: serde_json::Error| ByokError::Translation(e.to_string()))?;
        let responses_req =
            build_responses_create_request(&aigw_request, &ResponsesRequestConfig::codex())
                .map_err(|e| ByokError::Translation(e.to_string()))?;
        let mut codex_body = serde_json::to_value(&responses_req)
            .map_err(|e: serde_json::Error| ByokError::Translation(e.to_string()))?;
        codex_body["stream"] = Value::Bool(true);
        codex_body["type"] = Value::String("response.create".into());

        // Build the WebSocket handshake request with required headers.
        let ws_request = http::Request::builder()
            .uri(WS_URL)
            .header("Authorization", format!("Bearer {token}"))
            .header("OpenAI-Beta", WS_BETA)
            .header("User-Agent", CODEX_USER_AGENT)
            .header("Originator", "codex_cli_rs")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .header("Sec-WebSocket-Version", "13")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Host", "chatgpt.com")
            .body(())
            .map_err(|e| ByokError::Http(format!("failed to build WS request: {e}")))?;

        // Connect to the WebSocket endpoint.
        let (ws_stream, _response) = tokio_tungstenite::connect_async(ws_request)
            .await
            .map_err(|e| ByokError::Http(format!("WebSocket connect failed: {e}")))?;

        let (mut sink, stream) = ws_stream.split();

        // Send the translated request body as a text message.
        let payload = serde_json::to_string(&codex_body)
            .map_err(|e| ByokError::Http(format!("failed to serialize body: {e}")))?;
        sink.send(tungstenite::Message::Text(payload.into()))
            .await
            .map_err(|e| ByokError::Http(format!("WebSocket send failed: {e}")))?;

        // Convert the incoming WebSocket messages into SSE-formatted bytes,
        // then pipe through the existing Codex SSE translator.
        let raw_stream: ByteStream = Box::pin(try_unfold(stream, |mut ws_rx| async move {
            loop {
                match ws_rx.next().await {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        // Wrap each JSON event in SSE "data:" framing so
                        // the shared translator can parse it.
                        let sse_line = format!("data: {text}\n\n");
                        return Ok(Some((Bytes::from(sse_line), ws_rx)));
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        // Stream finished.
                        return Ok(None);
                    }
                    Some(Ok(_)) => {
                        // Ignore ping/pong/binary frames.
                    }
                    Some(Err(e)) => {
                        return Err(ByokError::Http(format!("WebSocket recv error: {e}")));
                    }
                }
            }
        }));

        Ok(ProviderResponse::Stream(translate_codex_responses_sse(
            raw_stream,
        )))
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Codex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> CodexWsExecutor {
        let (_client, auth) = crate::http_util::test_auth();
        CodexWsExecutor::new(auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_supported_models_contains_o4_mini() {
        let ex = make_executor();
        assert!(ex.supported_models().iter().any(|m| m == "o4-mini"));
    }
}
