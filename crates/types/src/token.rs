//! OAuth token representation and expiry logic.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// An OAuth token with optional refresh capability and expiry tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
}

impl OAuthToken {
    /// Create a new token with the given access token and `Bearer` type.
    pub fn new(access_token: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token: None,
            expires_at: None,
            token_type: Some("Bearer".to_string()),
        }
    }

    /// Set the expiry to `expires_in_secs` seconds from now.
    #[must_use]
    pub fn with_expiry(mut self, expires_in_secs: u64) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        self.expires_at = Some(now + expires_in_secs);
        self
    }

    /// Attach a refresh token.
    #[must_use]
    pub fn with_refresh(mut self, refresh_token: impl Into<String>) -> Self {
        self.refresh_token = Some(refresh_token.into());
        self
    }

    /// Return `true` if the token expires within 60 seconds.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let Some(expires_at) = self.expires_at else {
            return false;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        now + 60 >= expires_at
    }

    /// Determine the current token state based on expiry and refresh availability.
    #[must_use]
    pub fn state(&self) -> TokenState {
        if self.is_expired() {
            if self.refresh_token.is_some() {
                TokenState::Expired
            } else {
                TokenState::Invalid
            }
        } else {
            TokenState::Valid
        }
    }
}

/// Describes the usability state of an [`OAuthToken`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenState {
    Valid,
    /// Expired but a refresh token is available for renewal.
    Expired,
    /// Expired with no refresh token; the token cannot be renewed.
    Invalid,
}

/// Metadata about a stored account (without the token itself).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    /// Unique identifier within the provider (e.g. `"default"`, `"work"`).
    pub account_id: String,
    /// Optional human-readable label for display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether this account is the active one for its provider.
    pub is_active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn past_secs(secs: u64) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(secs)
    }

    #[test]
    fn test_valid_no_expiry() {
        let t = OAuthToken::new("tok");
        assert!(!t.is_expired());
        assert_eq!(t.state(), TokenState::Valid);
    }

    #[test]
    fn test_valid_future_expiry() {
        let t = OAuthToken::new("tok").with_expiry(3600);
        assert!(!t.is_expired());
        assert_eq!(t.state(), TokenState::Valid);
    }

    #[test]
    fn test_expired_with_refresh() {
        let t = OAuthToken {
            access_token: "old".into(),
            refresh_token: Some("ref".into()),
            expires_at: Some(past_secs(100)),
            token_type: None,
        };
        assert!(t.is_expired());
        assert_eq!(t.state(), TokenState::Expired);
    }

    #[test]
    fn test_invalid_no_refresh() {
        let t = OAuthToken {
            access_token: "old".into(),
            refresh_token: None,
            expires_at: Some(past_secs(100)),
            token_type: None,
        };
        assert_eq!(t.state(), TokenState::Invalid);
    }

    #[test]
    fn test_near_expiry_treated_as_expired() {
        let soon = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 30; // 30s < 60s threshold
        let t = OAuthToken {
            access_token: "tok".into(),
            refresh_token: Some("ref".into()),
            expires_at: Some(soon),
            token_type: None,
        };
        assert!(t.is_expired());
    }

    #[test]
    fn test_serde_roundtrip() {
        let t = OAuthToken::new("access")
            .with_expiry(3600)
            .with_refresh("ref");
        let json = serde_json::to_string(&t).unwrap();
        let back: OAuthToken = serde_json::from_str(&json).unwrap();
        assert_eq!(back.access_token, "access");
        assert_eq!(back.refresh_token, Some("ref".into()));
        assert!(back.expires_at.is_some());
    }

    #[test]
    fn test_serde_skips_none() {
        let t = OAuthToken::new("tok");
        let json = serde_json::to_string(&t).unwrap();
        assert!(!json.contains("refresh_token"));
        assert!(!json.contains("expires_at"));
    }
}
