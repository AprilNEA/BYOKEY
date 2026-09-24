//! Which client BYOKEY presents itself as to GitHub Copilot.

use crate::{ByokError, OAuthToken};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// A Copilot client BYOKEY can log in and send requests as.
///
/// The client is fixed per credential: a GitHub token only works the way the
/// OAuth app that issued it is allowed to use Copilot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CopilotClient {
    /// `OpenCode`, an officially supported Copilot client. Its GitHub token
    /// authenticates Copilot API requests directly.
    #[default]
    OpenCode,
    /// VS Code Copilot Chat. Its GitHub token must first be exchanged for a
    /// short-lived Copilot API token, and requests carry VS Code's editor
    /// and device headers.
    VsCode,
}

impl CopilotClient {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::VsCode => "vscode",
        }
    }

    /// The client a stored Copilot token was issued to.
    ///
    /// Tokens saved before BYOKEY supported more than one client carry no
    /// marker; they all came from VS Code's OAuth app.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Auth`] if the token names an unknown client.
    pub fn of(token: &OAuthToken) -> Result<Self, ByokError> {
        token.client.as_deref().map_or(Ok(Self::VsCode), str::parse)
    }
}

impl fmt::Display for CopilotClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CopilotClient {
    type Err = ByokError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opencode" => Ok(Self::OpenCode),
            "vscode" => Ok(Self::VsCode),
            other => Err(ByokError::Auth(format!(
                "unknown Copilot client '{other}' (expected 'opencode' or 'vscode')"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmarked_tokens_are_legacy_vscode_logins() {
        let token = OAuthToken::new("ghu_legacy");
        assert_eq!(CopilotClient::of(&token).unwrap(), CopilotClient::VsCode);
    }

    #[test]
    fn marked_tokens_name_their_client() {
        let token = OAuthToken::new("gho_new").with_client(CopilotClient::OpenCode.as_str());
        assert_eq!(CopilotClient::of(&token).unwrap(), CopilotClient::OpenCode);
    }

    #[test]
    fn unknown_client_marker_is_an_error() {
        let token = OAuthToken::new("x").with_client("jetbrains");
        assert!(CopilotClient::of(&token).is_err());
    }
}
