//! Error type for the `ampcode` crate.

use thiserror::Error;

/// All errors that can occur in the `ampcode` crate.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AmpcodeError {
    /// HTTP transport error (connection, TLS, timeout, etc.).
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// The API returned a non-2xx status code.
    #[error("api error: status={status}, body={body}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Response body text.
        body: String,
    },

    /// JSON serialization or deserialization failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// A local file (secrets.json, thread JSON) could not be read.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The secrets file exists but is missing the expected token for `ampcode.com`.
    #[error("no token found in secrets file for ampcode.com")]
    NoToken,

    /// `displayText` from `userDisplayBalanceInfo` could not be parsed.
    ///
    /// Contains the raw text that failed to parse.
    #[error("failed to parse balance display text: {0:?}")]
    BalanceParse(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, AmpcodeError>;
