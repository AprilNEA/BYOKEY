//! Request headers that present BYOKEY to GitHub as VS Code Copilot Chat.
//!
//! Every Copilot request — OpenAI-format chat completions and the native
//! Anthropic Messages passthrough alike — takes its headers from here, so the
//! client identity cannot drift between the two paths.

use crate::versions::ProviderVersions;
use serde_json::Value;

// Compile-time fallbacks for when `assets.byokey.io/versions/copilot.json`
// is unreachable. Keep them in step with that file.
const DEFAULT_USER_AGENT: &str = "GitHubCopilotChat/0.58.0";
const DEFAULT_EDITOR_VERSION: &str = "vscode/1.130.0";
const DEFAULT_PLUGIN_VERSION: &str = "copilot-chat/0.58.0";
const DEFAULT_API_VERSION: &str = "2026-06-01";

const INTEGRATION_ID: &str = "vscode-chat";
const OPENAI_INTENT: &str = "conversation-panel";
/// The fetch implementation VS Code's Electron host reports.
const LIBRARY_VERSION: &str = "electron-fetch";

/// The editor and extension versions a Copilot request claims to come from.
#[derive(Clone, Debug)]
pub struct CopilotIdentity {
    user_agent: String,
    editor_version: String,
    plugin_version: String,
    api_version: String,
}

impl Default for CopilotIdentity {
    fn default() -> Self {
        Self::from_versions(None)
    }
}

impl CopilotIdentity {
    /// Builds the identity from runtime-fetched versions, falling back per
    /// field to the compile-time defaults.
    #[must_use]
    pub fn from_versions(versions: Option<&ProviderVersions>) -> Self {
        let pick = |field: fn(&ProviderVersions) -> &Option<String>, default: &str| {
            versions
                .and_then(|v| field(v).clone())
                .unwrap_or_else(|| default.to_owned())
        };
        Self {
            user_agent: pick(|v| &v.user_agent, DEFAULT_USER_AGENT),
            editor_version: pick(|v| &v.editor_version, DEFAULT_EDITOR_VERSION),
            plugin_version: pick(|v| &v.plugin_version, DEFAULT_PLUGIN_VERSION),
            api_version: pick(|v| &v.github_api_version, DEFAULT_API_VERSION),
        }
    }

    /// Headers for `api.github.com` calls: token exchange and quota lookup.
    pub(super) fn github_headers(&self) -> [(&'static str, &str); 4] {
        [
            ("user-agent", &self.user_agent),
            ("editor-version", &self.editor_version),
            ("editor-plugin-version", &self.plugin_version),
            ("x-vscode-user-agent-library-version", LIBRARY_VERSION),
        ]
    }

    /// Headers sent on every Copilot API request.
    #[must_use]
    pub fn api_headers(&self) -> [(&'static str, &str); 7] {
        [
            ("user-agent", &self.user_agent),
            ("editor-version", &self.editor_version),
            ("editor-plugin-version", &self.plugin_version),
            ("copilot-integration-id", INTEGRATION_ID),
            ("openai-intent", OPENAI_INTENT),
            ("x-github-api-version", &self.api_version),
            ("x-vscode-user-agent-library-version", LIBRARY_VERSION),
        ]
    }
}

/// Headers that depend on the individual request.
///
/// Accepts `messages` in either `OpenAI` chat or Anthropic Messages shape.
#[derive(Debug)]
pub struct ConversationHeaders {
    initiator: &'static str,
    vision: bool,
    request_id: String,
}

impl ConversationHeaders {
    #[must_use]
    pub fn from_messages(messages: &[Value]) -> Self {
        Self {
            initiator: initiator(messages),
            vision: messages
                .iter()
                .any(|m| m.get("content").is_some_and(has_image)),
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            Some(("x-initiator", self.initiator)),
            Some(("x-request-id", self.request_id.as_str())),
            // Copilot rejects image input unless the request opts in.
            self.vision.then_some(("copilot-vision-request", "true")),
        ]
        .into_iter()
        .flatten()
    }
}

/// `agent` once the conversation contains a model or tool turn, else `user`.
fn initiator(messages: &[Value]) -> &'static str {
    let is_agent = messages.iter().any(|m| {
        matches!(
            m.get("role").and_then(Value::as_str),
            Some("assistant" | "tool")
        )
    });
    if is_agent { "agent" } else { "user" }
}

/// Whether message content carries an image: an `OpenAI` `image_url` part, an
/// Anthropic `image` block, or an image nested in an Anthropic `tool_result`.
fn has_image(content: &Value) -> bool {
    content.as_array().is_some_and(|parts| {
        parts
            .iter()
            .any(|part| match part.get("type").and_then(Value::as_str) {
                Some("image" | "image_url") => true,
                Some("tool_result") => part.get("content").is_some_and(has_image),
                _ => false,
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(messages: &Value) -> Vec<(&'static str, String)> {
        ConversationHeaders::from_messages(messages.as_array().unwrap())
            .iter()
            .map(|(k, v)| (k, v.to_owned()))
            .collect()
    }

    fn get<'a>(headers: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn initiator_is_user_for_a_fresh_prompt() {
        let h = headers(&json!([{"role": "user", "content": "hi"}]));
        assert_eq!(get(&h, "x-initiator"), Some("user"));
    }

    #[test]
    fn initiator_is_agent_once_the_model_has_replied() {
        let h = headers(&json!([
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
            {"role": "user", "content": "more"}
        ]));
        assert_eq!(get(&h, "x-initiator"), Some("agent"));
    }

    #[test]
    fn text_only_request_does_not_opt_into_vision() {
        let h = headers(&json!([
            {"role": "user", "content": [{"type": "text", "text": "hi"}]}
        ]));
        assert_eq!(get(&h, "copilot-vision-request"), None);
    }

    #[test]
    fn openai_image_part_opts_into_vision() {
        let h = headers(&json!([{"role": "user", "content": [
            {"type": "text", "text": "what is this"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,AA=="}}
        ]}]));
        assert_eq!(get(&h, "copilot-vision-request"), Some("true"));
    }

    #[test]
    fn anthropic_image_block_opts_into_vision() {
        let h = headers(&json!([{"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AA=="}}
        ]}]));
        assert_eq!(get(&h, "copilot-vision-request"), Some("true"));
    }

    #[test]
    fn image_inside_tool_result_opts_into_vision() {
        let h = headers(&json!([{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AA=="}}
            ]}
        ]}]));
        assert_eq!(get(&h, "copilot-vision-request"), Some("true"));
    }

    #[test]
    fn each_request_gets_its_own_request_id() {
        let msgs = json!([{"role": "user", "content": "hi"}]);
        let a = headers(&msgs);
        let b = headers(&msgs);
        assert_ne!(get(&a, "x-request-id"), get(&b, "x-request-id"));
    }

    #[test]
    fn runtime_versions_override_defaults_per_field() {
        let versions: ProviderVersions = serde_json::from_value(json!({
            "user_agent": "GitHubCopilotChat/9.9.9",
            "github_api_version": "2099-01-01"
        }))
        .unwrap();
        let id = CopilotIdentity::from_versions(Some(&versions));
        let h = id.api_headers();
        let get = |name| h.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
        assert_eq!(get("user-agent"), Some("GitHubCopilotChat/9.9.9"));
        assert_eq!(get("x-github-api-version"), Some("2099-01-01"));
        assert_eq!(get("editor-version"), Some(DEFAULT_EDITOR_VERSION));
    }
}
