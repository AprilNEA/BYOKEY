//! Unified error type for the byokey workspace.

use thiserror::Error;

/// Enumerates all error kinds that can occur across byokey crates.
#[derive(Debug, Error)]
pub enum ByokError {
    /// OAuth or credential authentication failure.
    #[error("authentication error: {0}")]
    Auth(String),

    /// No stored token exists for the given provider.
    #[error("token not found for provider: {0}")]
    TokenNotFound(crate::ProviderId),

    /// The stored token has expired and cannot be used.
    #[error("token expired for provider: {0}")]
    TokenExpired(crate::ProviderId),

    /// The requested provider is not configured or reachable.
    #[error("provider not available: {0}")]
    ProviderUnavailable(crate::ProviderId),

    /// Request or response format translation failure.
    #[error("translation error: {0}")]
    Translation(String),

    /// HTTP transport error.
    #[error("http error: {0}")]
    Http(String),

    /// JSON serialization or deserialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Persistent storage (`SQLite`) error.
    #[error("storage error: {0}")]
    Storage(String),

    /// Configuration loading or validation error.
    #[error("configuration error: {0}")]
    Config(String),

    /// The requested model is not supported by any provider.
    #[error("unsupported model: {0}")]
    UnsupportedModel(String),

    /// The upstream provider returned a non-success status.
    #[error("upstream error: status={status}, body={body}")]
    Upstream { status: u16, body: String },
}

// ── Feature-gated From impls ──────────────────────────────────────────────────

#[cfg(feature = "rquest")]
impl From<rquest::Error> for ByokError {
    fn from(e: rquest::Error) -> Self {
        Self::Http(e.to_string())
    }
}

#[cfg(feature = "sqlx")]
impl From<sqlx::Error> for ByokError {
    fn from(e: sqlx::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

impl ByokError {
    /// Returns `true` if the error is likely transient and worth retrying.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Upstream { status, .. } => matches!(status, 408 | 429 | 500 | 502 | 503 | 504),
            Self::Http(_) => true, // transport errors are retryable
            _ => false,
        }
    }
}

/// Convenience alias used throughout the workspace.
pub type Result<T> = std::result::Result<T, ByokError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_auth() {
        let err = ByokError::Auth("bad credentials".to_string());
        assert_eq!(err.to_string(), "authentication error: bad credentials");
    }

    #[test]
    fn test_error_display_token_not_found() {
        let err = ByokError::TokenNotFound(crate::ProviderId::Claude);
        assert!(err.to_string().contains("claude"));
    }

    #[test]
    fn test_error_display_upstream() {
        let err = ByokError::Upstream {
            status: 429,
            body: "rate limited".to_string(),
        };
        let s = err.to_string();
        assert!(s.contains("429"));
        assert!(s.contains("rate limited"));
    }

    #[test]
    fn test_serialization_error_conversion() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid {{{").unwrap_err();
        let err: ByokError = json_err.into();
        assert!(matches!(err, ByokError::Serialization(_)));
    }

    #[test]
    fn test_is_retryable_upstream() {
        assert!(
            ByokError::Upstream {
                status: 429,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            ByokError::Upstream {
                status: 500,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            ByokError::Upstream {
                status: 502,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            ByokError::Upstream {
                status: 503,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            ByokError::Upstream {
                status: 504,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            ByokError::Upstream {
                status: 408,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            !ByokError::Upstream {
                status: 401,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            !ByokError::Upstream {
                status: 403,
                body: String::new()
            }
            .is_retryable()
        );
        assert!(
            !ByokError::Upstream {
                status: 404,
                body: String::new()
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_is_retryable_http_transport() {
        assert!(ByokError::Http("connection refused".into()).is_retryable());
    }

    #[test]
    fn test_is_retryable_other_errors() {
        assert!(!ByokError::Auth("bad".into()).is_retryable());
        assert!(!ByokError::Config("bad".into()).is_retryable());
        assert!(!ByokError::UnsupportedModel("gpt-5".into()).is_retryable());
    }
}
