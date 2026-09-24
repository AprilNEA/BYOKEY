//! Request headers that present BYOKEY to GitHub as a Copilot client.
//!
//! `OpenCode` mirrors `packages/opencode/src/plugin/github-copilot/copilot.ts`
//! in `anomalyco/opencode`: a user agent, a pinned API version, and the
//! per-request initiator, interaction and vision headers.
//!
//! VS Code mirrors VS Code 1.139 / Copilot Chat 0.67 (`microsoft/vscode`,
//! `extensions/copilot`): `networkRequest` and `ChatMLFetcher` build the
//! per-request headers, and `@vscode/copilot-api`'s `CAPIClient` mixes in the
//! client identity and device ids last.
//!
//! Every Copilot request — OpenAI-format chat completions and the native
//! Anthropic Messages passthrough alike — takes its headers from here, so the
//! two paths cannot drift apart.

use super::CopilotCredentials;
use super::device::{CopilotDevice, uuid_from};
use crate::versions::ProviderVersions;
use byokey_types::CopilotClient;
use serde_json::Value;
use sha2::{Digest, Sha256};

// Compile-time fallbacks for when `assets.byokey.io/versions/copilot.json`
// is unreachable. Keep them in step with that file.
const DEFAULT_OPENCODE_VERSION: &str = "1.18.32";
const DEFAULT_OPENCODE_API_VERSION: &str = "2026-06-01";
const DEFAULT_USER_AGENT: &str = "GitHubCopilotChat/0.67.0";
const DEFAULT_EDITOR_VERSION: &str = "vscode/1.139.0";
const DEFAULT_PLUGIN_VERSION: &str = "copilot-chat/0.67.0";
/// What `CAPIClient` stamps on chat requests, overriding the caller's value.
const DEFAULT_API_VERSION: &str = "2026-08-01";

/// `OpenCode`'s intent for every chat request.
const OPENCODE_INTENT: &str = "conversation-edits";

/// Pinned by `CopilotTokenManager` for the token and user-info calls.
const GITHUB_REST_API_VERSION: &str = "2025-04-01";
const INTEGRATION_ID: &str = "vscode-chat";
/// The fetcher VS Code desktop prefers by default.
const LIBRARY_VERSION: &str = "electron-fetch";
/// `locationToIntent(ChatLocation.Agent)`. Also the `X-Interaction-Type`,
/// since a proxy has no signal for the subagent/compaction/background
/// overrides.
const VSCODE_INTENT: &str = "conversation-agent";

/// The client versions a Copilot request claims to come from.
#[derive(Clone, Debug)]
pub struct CopilotIdentity {
    opencode_user_agent: String,
    opencode_api_version: String,
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
            opencode_user_agent: format!(
                "opencode/{}",
                pick(|v| &v.opencode_version, DEFAULT_OPENCODE_VERSION)
            ),
            opencode_api_version: pick(|v| &v.opencode_api_version, DEFAULT_OPENCODE_API_VERSION),
            user_agent: pick(|v| &v.user_agent, DEFAULT_USER_AGENT),
            editor_version: pick(|v| &v.editor_version, DEFAULT_EDITOR_VERSION),
            plugin_version: pick(|v| &v.plugin_version, DEFAULT_PLUGIN_VERSION),
            api_version: pick(|v| &v.github_api_version, DEFAULT_API_VERSION),
        }
    }

    /// Headers for the `api.github.com` token and user-info calls. VS Code
    /// sends these through the fetcher alone, without `CAPIClient`'s editor
    /// headers.
    pub(super) fn github_headers(&self, client: CopilotClient) -> Vec<(&'static str, &str)> {
        match client {
            CopilotClient::OpenCode => vec![("user-agent", &self.opencode_user_agent)],
            CopilotClient::VsCode => vec![
                ("user-agent", &self.user_agent),
                ("x-github-api-version", GITHUB_REST_API_VERSION),
                ("x-vscode-user-agent-library-version", LIBRARY_VERSION),
            ],
        }
    }

    /// Every header one Copilot API request attempt carries, besides
    /// authorization and content negotiation.
    #[must_use]
    pub fn request_headers(
        &self,
        creds: &CopilotCredentials,
        conversation: &Conversation,
    ) -> Vec<(&'static str, String)> {
        let initiator = if conversation.user_initiated {
            "user"
        } else {
            "agent"
        };
        let mut headers = match creds.client {
            CopilotClient::OpenCode => vec![
                ("user-agent", self.opencode_user_agent.clone()),
                ("x-github-api-version", self.opencode_api_version.clone()),
                ("openai-intent", OPENCODE_INTENT.to_owned()),
                (
                    "x-interaction-id",
                    interaction_id(&creds.device, conversation.session),
                ),
                ("x-initiator", initiator.to_owned()),
            ],
            CopilotClient::VsCode => {
                // Each attempt is its own request, so it gets a fresh request
                // id, as `ChatMLFetcher` does per fetch.
                let request_id = uuid::Uuid::new_v4().to_string();
                let mut headers: Vec<_> = [
                    ("user-agent", self.user_agent.as_str()),
                    ("editor-version", &self.editor_version),
                    ("editor-plugin-version", &self.plugin_version),
                    ("copilot-integration-id", INTEGRATION_ID),
                    ("x-github-api-version", &self.api_version),
                    ("x-vscode-user-agent-library-version", LIBRARY_VERSION),
                ]
                .into_iter()
                .chain(creds.device.headers())
                .map(|(k, v)| (k, v.to_owned()))
                .collect();
                headers.extend([
                    ("x-request-id", request_id.clone()),
                    ("x-agent-task-id", request_id),
                    ("openai-intent", VSCODE_INTENT.to_owned()),
                    ("x-interaction-type", VSCODE_INTENT.to_owned()),
                    (
                        "x-interaction-id",
                        interaction_id(&creds.device, conversation.turn),
                    ),
                    ("x-initiator", initiator.to_owned()),
                ]);
                headers
            }
        };
        if conversation.vision {
            headers.push(("copilot-vision-request", "true".to_owned()));
        }
        headers
    }
}

/// What a request's messages say about the turn it belongs to.
///
/// Accepts `messages` in either `OpenAI` chat or Anthropic Messages shape.
#[derive(Debug)]
pub struct Conversation {
    user_initiated: bool,
    vision: bool,
    /// Identifies the conversation: VS Code mints an interaction per user
    /// turn, `OpenCode` keys it by session.
    turn: [u8; 32],
    session: [u8; 32],
}

impl Conversation {
    #[must_use]
    pub fn from_messages(messages: &[Value]) -> Self {
        Self {
            user_initiated: messages.last().is_none_or(is_user_prompt),
            vision: messages
                .iter()
                .any(|m| m.get("content").is_some_and(has_image)),
            turn: anchor(messages, messages.iter().rposition(is_user_prompt)),
            session: anchor(messages, (!messages.is_empty()).then_some(0)),
        }
    }
}

/// A UUID for `anchor`, scoped to the device's session so it does not
/// repeat across accounts or processes.
fn interaction_id(device: &CopilotDevice, anchor: [u8; 32]) -> String {
    uuid_from(
        &Sha256::new()
            .chain_update(device.session_id())
            .chain_update(anchor)
            .finalize()
            .into(),
    )
    .to_string()
}

/// Whether `message` is a prompt the user typed, as opposed to the agent loop
/// feeding a tool result back. VS Code sends `x-initiator: user` only for the
/// first request of a user turn; every tool-loop iteration after it is
/// `agent`. `OpenAI` tool results arrive as `role: "tool"`; Anthropic ones as
/// `tool_result` blocks in a user message, which may also carry text such as
/// system reminders — the tool result still makes it a loop iteration.
fn is_user_prompt(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
        && !message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            })
}

/// Hash of the message at `opener` and its position.
///
/// Later requests of the same turn (opened by the last user prompt) or
/// session (opened by the first message) resend that message unchanged, so
/// the hash keys the interaction they belong to. `cache_control` markers are
/// dropped first: agents move them between requests.
fn anchor(messages: &[Value], opener: Option<usize>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    if let Some(index) = opener {
        let mut prompt = messages[index].clone();
        strip_cache_control(&mut prompt);
        hasher.update(index.to_le_bytes());
        hasher.update(prompt.to_string());
    }
    hasher.finalize().into()
}

fn strip_cache_control(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("cache_control");
            map.values_mut().for_each(strip_cache_control);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_cache_control),
        _ => {}
    }
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

    fn creds(client: CopilotClient) -> CopilotCredentials {
        CopilotCredentials {
            token: "ghu_test".to_owned(),
            endpoint: "https://api.githubcopilot.com".to_owned(),
            client,
            device: CopilotDevice::for_credential("ghu_test"),
        }
    }

    fn headers_as(client: CopilotClient, messages: &Value) -> Vec<(&'static str, String)> {
        CopilotIdentity::default().request_headers(
            &creds(client),
            &Conversation::from_messages(messages.as_array().unwrap()),
        )
    }

    fn headers(messages: &Value) -> Vec<(&'static str, String)> {
        headers_as(CopilotClient::VsCode, messages)
    }

    fn get<'a>(headers: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_typed_prompt_is_user_initiated() {
        let h = headers(&json!([{"role": "user", "content": "hi"}]));
        assert_eq!(get(&h, "x-initiator"), Some("user"));
    }

    #[test]
    fn a_follow_up_prompt_in_a_long_conversation_is_user_initiated() {
        let h = headers(&json!([
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
            {"role": "user", "content": "now do this"}
        ]));
        assert_eq!(get(&h, "x-initiator"), Some("user"));
    }

    #[test]
    fn an_openai_tool_result_is_agent_initiated() {
        let h = headers(&json!([
            {"role": "user", "content": "list files"},
            {"role": "assistant", "tool_calls": [{"id": "c1", "type": "function",
                "function": {"name": "ls", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": "a.rs"}
        ]));
        assert_eq!(get(&h, "x-initiator"), Some("agent"));
    }

    #[test]
    fn an_anthropic_tool_result_is_agent_initiated_even_with_text_alongside() {
        let h = headers(&json!([
            {"role": "user", "content": "list files"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "ls", "input": {}}]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "a.rs"},
                {"type": "text", "text": "<system-reminder>todo list is empty</system-reminder>"}
            ]}
        ]));
        assert_eq!(get(&h, "x-initiator"), Some("agent"));
    }

    #[test]
    fn tool_loop_iterations_share_the_turns_interaction_id() {
        let first = json!([
            {"role": "user", "content": [{"type": "text", "text": "fix it",
                "cache_control": {"type": "ephemeral"}}]}
        ]);
        let later = json!([
            {"role": "user", "content": [{"type": "text", "text": "fix it"}]},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "ls", "input": {}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "a.rs",
                "cache_control": {"type": "ephemeral"}}]}
        ]);
        assert_eq!(
            get(&headers(&first), "x-interaction-id"),
            get(&headers(&later), "x-interaction-id")
        );
    }

    #[test]
    fn a_new_prompt_starts_a_new_interaction() {
        let first = json!([{"role": "user", "content": "fix it"}]);
        let next = json!([
            {"role": "user", "content": "fix it"},
            {"role": "assistant", "content": "done"},
            {"role": "user", "content": "now test it"}
        ]);
        assert_ne!(
            get(&headers(&first), "x-interaction-id"),
            get(&headers(&next), "x-interaction-id")
        );
    }

    #[test]
    fn opencode_keys_the_interaction_by_session_across_turns() {
        let first = json!([{"role": "user", "content": "fix it"}]);
        let next = json!([
            {"role": "user", "content": "fix it"},
            {"role": "assistant", "content": "done"},
            {"role": "user", "content": "now test it"}
        ]);
        let other_session = json!([{"role": "user", "content": "write docs"}]);
        let id =
            |m| get(&headers_as(CopilotClient::OpenCode, m), "x-interaction-id").map(str::to_owned);
        assert_eq!(id(&first), id(&next));
        assert_ne!(id(&first), id(&other_session));
    }

    #[test]
    fn opencode_does_not_claim_to_be_vscode() {
        let h = headers_as(
            CopilotClient::OpenCode,
            &json!([{"role": "user", "content": "hi"}]),
        );
        assert_eq!(get(&h, "openai-intent"), Some(OPENCODE_INTENT));
        assert!(get(&h, "user-agent").unwrap().starts_with("opencode/"));
        for vscode_only in ["editor-version", "vscode-machineid", "x-request-id"] {
            assert_eq!(get(&h, vscode_only), None, "{vscode_only}");
        }
    }

    #[test]
    fn opencode_marks_tool_loop_iterations_as_agent() {
        let h = headers_as(
            CopilotClient::OpenCode,
            &json!([
                {"role": "user", "content": "list files"},
                {"role": "assistant", "tool_calls": [{"id": "c1", "type": "function",
                    "function": {"name": "ls", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "c1", "content": "a.rs"}
            ]),
        );
        assert_eq!(get(&h, "x-initiator"), Some("agent"));
    }

    #[test]
    fn each_attempt_gets_its_own_request_id_mirrored_as_task_id() {
        let messages = json!([{"role": "user", "content": "hi"}]);
        let a = headers(&messages);
        let b = headers(&messages);
        assert_ne!(get(&a, "x-request-id"), get(&b, "x-request-id"));
        assert_eq!(get(&a, "x-request-id"), get(&a, "x-agent-task-id"));
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
    fn runtime_versions_override_defaults_per_field() {
        let versions: ProviderVersions = serde_json::from_value(json!({
            "user_agent": "GitHubCopilotChat/9.9.9",
            "github_api_version": "2099-01-01",
            "opencode_version": "9.0.0"
        }))
        .unwrap();
        let id = CopilotIdentity::from_versions(Some(&versions));
        let api = id.request_headers(
            &creds(CopilotClient::VsCode),
            &Conversation::from_messages(&[]),
        );
        assert_eq!(get(&api, "user-agent"), Some("GitHubCopilotChat/9.9.9"));
        assert_eq!(get(&api, "x-github-api-version"), Some("2099-01-01"));
        assert_eq!(get(&api, "editor-version"), Some(DEFAULT_EDITOR_VERSION));
        let opencode = id.request_headers(
            &creds(CopilotClient::OpenCode),
            &Conversation::from_messages(&[]),
        );
        assert_eq!(get(&opencode, "user-agent"), Some("opencode/9.0.0"));
        assert_eq!(
            get(&opencode, "x-github-api-version"),
            Some(DEFAULT_OPENCODE_API_VERSION)
        );
        // The REST calls keep their own pinned version.
        let rest = id.github_headers(CopilotClient::VsCode);
        assert!(rest.contains(&("x-github-api-version", GITHUB_REST_API_VERSION)));
    }
}
