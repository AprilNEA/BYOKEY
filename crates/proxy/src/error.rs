//! API error type that maps [`ByokError`] variants to HTTP responses.
//!
//! Upstream failures keep their status code and body: clients such as Claude
//! Code and Claude Desktop read the status to tell a rejected credential from
//! a rejected model, and their retry logic honours `retry-after`. Errors
//! raised by the gateway itself are rendered in the wire format of the route
//! that failed: the Anthropic Messages envelope on `/v1/messages`, the
//! `OpenAI` one elsewhere.
//!
//! Rendering an error logs it, unless an upstream exchange already logged
//! it where it happened.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use byokey_types::ByokError;
use serde_json::{Value, json};

/// The error envelope a route speaks.
#[derive(Debug, Clone, Copy, Default)]
pub enum Wire {
    /// `{"error": {"message", "type", "code"}}`, as `OpenAI` clients expect.
    #[default]
    OpenAi,
    /// `{"type": "error", "error": {"type", "message"}}`, as Anthropic
    /// clients expect.
    Anthropic,
}

/// Wrapper around [`ByokError`] that implements [`IntoResponse`].
#[derive(Debug)]
pub struct ApiError {
    /// What went wrong.
    pub error: ByokError,
    /// How to render it.
    pub wire: Wire,
    /// Whether the failure was logged where it happened.
    logged: bool,
}

impl ApiError {
    /// An error on an `OpenAI`-format route.
    #[must_use]
    pub fn new(error: ByokError) -> Self {
        Self {
            error,
            wire: Wire::OpenAi,
            logged: false,
        }
    }

    /// The same error, rendered for an Anthropic-format route.
    #[must_use]
    pub fn anthropic(self) -> Self {
        Self {
            wire: Wire::Anthropic,
            ..self
        }
    }

    /// The same error, already logged where it happened (an upstream
    /// exchange logs how it ended), so rendering it does not log it again.
    #[must_use]
    pub(crate) fn logged(self) -> Self {
        Self {
            logged: true,
            ..self
        }
    }

    /// Log the error: an upstream's refusal with the upstream's own message,
    /// a failure inside the gateway at `error`, anything else (a missing
    /// login, an unknown model, an unreachable upstream) at `warn`.
    fn log(&self) {
        if let ByokError::Upstream { status, body, .. } = &self.error {
            let upstream = UpstreamMessage::of(body);
            tracing::warn!(
                status,
                error_type = upstream.error_type.as_deref(),
                upstream_message = %upstream.message,
                "the upstream refused the request"
            );
            return;
        }
        let (status, ..) = self.classify();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(status = status.as_u16(), error = %self.error, "request failed");
        } else {
            tracing::warn!(status = status.as_u16(), error = %self.error, "request failed");
        }
    }

    /// Status, Anthropic error type and `OpenAI` error code for an error the
    /// gateway raised itself. Upstream errors are handled separately.
    fn classify(&self) -> (StatusCode, &'static str, &'static str) {
        match &self.error {
            ByokError::Auth(_) => (
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "invalid_api_key",
            ),
            ByokError::TokenNotFound(_) | ByokError::TokenExpired(_) => (
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "token_not_found",
            ),
            ByokError::UnsupportedModel(_) => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "model_not_found",
            ),
            ByokError::UnsupportedProvider(_) => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "provider_not_found",
            ),
            ByokError::Translation(_) => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "translation_error",
            ),
            ByokError::Http(_) => (StatusCode::BAD_GATEWAY, "api_error", "upstream_error"),
            ByokError::Upstream { .. } => unreachable!("upstream errors keep their own status"),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "internal_error",
            ),
        }
    }

    /// The envelope for a message the gateway produced.
    fn envelope(&self, error_type: &str, code: &str, message: &str) -> Value {
        match self.wire {
            Wire::OpenAi => json!({
                "error": {"message": message, "type": error_type, "code": code}
            }),
            Wire::Anthropic => anthropic_envelope(error_type, message),
        }
    }
}

/// How much of an upstream's error message goes into the log.
const LOGGED_MESSAGE_CHARS: usize = 300;

/// What an upstream's error body says, for the log: the error type
/// (Anthropic's `error.type`, or `error.code` as Copilot sends it) and the
/// message, cut to [`LOGGED_MESSAGE_CHARS`]. A body that is not an error
/// envelope is its own message.
///
/// The message is logged as `upstream_message`, which stays out of Sentry:
/// an upstream can quote the request it rejects.
pub(crate) struct UpstreamMessage {
    pub(crate) error_type: Option<String>,
    pub(crate) message: String,
}

impl UpstreamMessage {
    pub(crate) fn of(body: &str) -> Self {
        serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|envelope| Self::of_envelope(&envelope))
            .unwrap_or_else(|| Self {
                error_type: None,
                message: cut(body),
            })
    }

    /// The error in an envelope (`{"error": {...}}`), if it is one. An
    /// error without a message is described by its JSON.
    pub(crate) fn of_envelope(envelope: &Value) -> Option<Self> {
        let error = envelope.get("error").filter(|e| e.is_object())?;
        let text = |key: &str| error.get(key).and_then(Value::as_str);
        Some(Self {
            error_type: text("type").or_else(|| text("code")).map(str::to_owned),
            message: text("message").map_or_else(|| cut(&error.to_string()), cut),
        })
    }
}

/// `text` cut to [`LOGGED_MESSAGE_CHARS`] characters.
fn cut(text: &str) -> String {
    match text.char_indices().nth(LOGGED_MESSAGE_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// The Anthropic error envelope.
pub(crate) fn anthropic_envelope(error_type: &str, message: &str) -> Value {
    json!({
        "type": "error",
        "error": {"type": error_type, "message": message}
    })
}

/// The Anthropic error type and `OpenAI` error code that describe an
/// upstream status, for bodies that need wrapping.
pub(crate) fn describe_status(status: StatusCode) -> (&'static str, &'static str) {
    match status.as_u16() {
        400 | 404 | 422 => ("invalid_request_error", "invalid_request"),
        401 => ("authentication_error", "invalid_api_key"),
        403 => ("permission_error", "insufficient_quota"),
        429 => ("rate_limit_error", "rate_limit_exceeded"),
        529 => ("overloaded_error", "overloaded"),
        _ => ("api_error", "upstream_error"),
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if !self.logged {
            self.log();
        }
        if let ByokError::Upstream {
            status,
            body,
            retry_after,
        } = &self.error
        {
            let status = StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY);
            // A JSON body is the upstream's own error envelope: forward it.
            // Anything else is wrapped so the client can still parse it.
            let payload = match serde_json::from_str::<Value>(body) {
                Ok(v) if v.is_object() => v,
                _ => {
                    let (error_type, code) = describe_status(status);
                    self.envelope(error_type, code, body)
                }
            };
            let mut response = (status, Json(payload)).into_response();
            if let Some(delay) = retry_after
                && let Ok(value) = HeaderValue::from_str(&delay.as_secs().to_string())
            {
                response.headers_mut().insert(RETRY_AFTER, value);
            }
            return response;
        }
        let (status, error_type, code) = self.classify();
        let payload = self.envelope(error_type, code, &self.error.to_string());
        (status, Json(payload)).into_response()
    }
}

impl From<ByokError> for ApiError {
    fn from(error: ByokError) -> Self {
        Self::new(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_types::ProviderId;
    use http_body_util::BodyExt as _;
    use std::time::Duration;

    async fn render(err: ApiError) -> (StatusCode, axum::http::HeaderMap, Value) {
        let resp = err.into_response();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, headers, serde_json::from_slice(&bytes).unwrap())
    }

    fn upstream(status: u16, body: &str) -> ByokError {
        ByokError::Upstream {
            status,
            body: body.into(),
            retry_after: None,
        }
    }

    #[tokio::test]
    async fn local_errors_take_the_openai_shape_by_default() {
        let (status, _, body) = render(ApiError::new(ByokError::Auth("bad creds".into()))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["type"], "authentication_error");
        assert_eq!(body["error"]["code"], "invalid_api_key");
        assert!(body.get("type").is_none());

        let (status, _, body) =
            render(ApiError::new(ByokError::TokenNotFound(ProviderId::Claude))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "token_not_found");

        let (status, _, body) =
            render(ApiError::new(ByokError::UnsupportedModel("xyz".into()))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert!(body["error"]["message"].as_str().unwrap().contains("xyz"));

        let (status, _, body) =
            render(ApiError::new(ByokError::Http("connection refused".into()))).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"]["code"], "upstream_error");

        let (status, _, body) = render(ApiError::new(ByokError::Config("bad config".into()))).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["code"], "internal_error");
    }

    #[tokio::test]
    async fn local_errors_take_the_anthropic_shape_when_asked() {
        let (status, _, body) =
            render(ApiError::new(ByokError::TokenNotFound(ProviderId::Claude)).anthropic()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["type"], "authentication_error");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("claude")
        );
        assert!(body["error"].get("code").is_none());
    }

    #[tokio::test]
    async fn upstream_status_and_json_body_pass_through() {
        let anthropic = r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages: at least one message is required"},"request_id":"req_1"}"#;
        for wire in [Wire::OpenAi, Wire::Anthropic] {
            let err = ApiError {
                wire,
                ..ApiError::new(upstream(400, anthropic))
            };
            let (status, _, body) = render(err).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["type"], "error");
            assert_eq!(body["error"]["type"], "invalid_request_error");
            assert_eq!(body["request_id"], "req_1");
        }

        let (status, _, body) = render(ApiError::new(upstream(
            401,
            r#"{"error":{"message":"bad token","type":"error"}}"#,
        )))
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["message"], "bad token");

        let (status, _, _) = render(ApiError::new(upstream(529, r#"{"error":{}}"#))).await;
        assert_eq!(status.as_u16(), 529);
    }

    #[tokio::test]
    async fn upstream_text_body_is_wrapped_for_the_route() {
        let (status, _, body) = render(ApiError::new(upstream(403, "forbidden"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["type"], "permission_error");
        assert_eq!(body["error"]["code"], "insufficient_quota");
        assert_eq!(body["error"]["message"], "forbidden");

        let (status, _, body) =
            render(ApiError::new(upstream(429, "rate limited")).anthropic()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["type"], "rate_limit_error");
        assert_eq!(body["error"]["message"], "rate limited");

        let (status, _, body) =
            render(ApiError::new(upstream(500, "<html>oops</html>")).anthropic()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["type"], "api_error");

        // A JSON scalar is not an envelope either.
        let (_, _, body) = render(ApiError::new(upstream(400, "\"nope\""))).await;
        assert_eq!(body["error"]["message"], "\"nope\"");
    }

    #[tokio::test]
    async fn retry_after_becomes_a_header() {
        let err = ApiError::new(ByokError::Upstream {
            status: 429,
            body: "slow down".into(),
            retry_after: Some(Duration::from_secs(17)),
        });
        let (status, headers, _) = render(err).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(headers[RETRY_AFTER], "17");

        let (_, headers, _) = render(ApiError::new(upstream(429, "slow down"))).await;
        assert!(!headers.contains_key(RETRY_AFTER));
    }

    #[tokio::test]
    async fn each_error_is_logged_at_a_level_that_says_who_failed() {
        use crate::test_logs::Logs;
        use tracing::Level;

        let logs = Logs::capture();
        render(ApiError::new(upstream(
            400,
            r#"{"error":{"message":"The use of the web search tool is not supported.","code":"unsupported_value"}}"#,
        )))
        .await;
        render(ApiError::new(ByokError::TokenNotFound(ProviderId::Copilot)).anthropic()).await;
        render(ApiError::new(ByokError::Http("connection refused".into()))).await;
        render(ApiError::new(ByokError::Storage("disk full".into()))).await;
        render(ApiError::new(upstream(429, "slow down")).logged()).await;

        let logged = logs.at_least(Level::WARN);
        let summary: Vec<_> = logged
            .iter()
            .map(|e| (e.level, e.field("status").unwrap_or("-")))
            .collect();
        assert_eq!(
            summary,
            [
                (Level::WARN, "400"),
                (Level::WARN, "401"),
                (Level::WARN, "502"),
                (Level::ERROR, "500"),
            ],
            "an error logged where it happened is not logged again"
        );
        assert_eq!(logged[0].field("error_type"), Some("unsupported_value"));
        assert_eq!(
            logged[0].field("upstream_message"),
            Some("The use of the web search tool is not supported.")
        );
    }

    #[test]
    fn upstream_messages_are_read_from_either_envelope_and_cut() {
        let anthropic = UpstreamMessage::of(
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_eq!(anthropic.error_type.as_deref(), Some("overloaded_error"));
        assert_eq!(anthropic.message, "Overloaded");

        let bare = UpstreamMessage::of(r#"{"error":{"code":"quota_exceeded"}}"#);
        assert_eq!(bare.error_type.as_deref(), Some("quota_exceeded"));
        assert_eq!(bare.message, r#"{"code":"quota_exceeded"}"#);

        let text = UpstreamMessage::of("<html>bad gateway</html>");
        assert_eq!(text.error_type, None);
        assert_eq!(text.message, "<html>bad gateway</html>");

        let long = UpstreamMessage::of(&"é".repeat(LOGGED_MESSAGE_CHARS + 5));
        assert_eq!(long.message.chars().count(), LOGGED_MESSAGE_CHARS + 1);
        assert!(long.message.ends_with('…'));
    }

    #[tokio::test]
    async fn invalid_upstream_status_falls_back_to_bad_gateway() {
        let (status, _, _) = render(ApiError::new(upstream(0, "?"))).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }
}
