//! Executor for Cursor's agent API (`agent.v1.AgentService/Run`).
//!
//! A run is a stateful bidirectional stream (see [`session`]). A turn that
//! stops on a caller tool call keeps its stream open, parked under the tool
//! call ids; the request that carries those tool results resumes the same
//! stream instead of replaying the conversation. That is what keeps Claude
//! and GPT models on Cursor from mistaking a flattened transcript for prompt
//! injection.
//!
//! Output is canonical [`StreamEvent`]s, rendered as `OpenAI` SSE here and as
//! Anthropic SSE by the proxy's `/v1/messages` route.

mod models;
mod pb;
mod session;

use crate::stream_bridge::{FinishReason, SseContext, StreamEvent, Usage, stream_events_to_sse};
use aigw_core::ForwardCompatible;
use aigw_core::model::{
    ChatRequest as CanonicalRequest, ChatResponse, Message, MessageContent, Role, TypedContentPart,
};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ChatRequest, ProviderId,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use bytes::Bytes;
use futures_util::{Stream, StreamExt as _, stream};
use serde_json::{Value, json};
use session::{Event, Run, RunSpec, ToolSpec};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const API_BASE: &str = "https://api2.cursor.sh";
const RUN_URL: &str = "https://agentn.global.api5.cursor.sh/agent.v1.AgentService/Run";
/// The Cursor CLI release this client presents as. Cursor rejects stale ones.
const CLIENT_VERSION: &str = "cli-2026.08.11-e8db854";
/// How long a parked run waits for its tool results.
const PARK_TTL: Duration = Duration::from_secs(600);

/// Runs stopped on tool calls, keyed by their sorted tool call ids, with
/// when they were parked. Process-wide because executors are built per
/// request.
type ParkedRuns = HashMap<Vec<String>, (Run, Instant)>;

static PARKED: LazyLock<Mutex<ParkedRuns>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// `crsr_` API key → the access token it was exchanged for.
static EXCHANGED: LazyLock<Mutex<HashMap<String, byokey_types::OAuthToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A stream of canonical events for one turn.
pub type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

/// Executor for the Cursor agent API.
pub struct CursorExecutor {
    http: wreq::Client,
    auth: Arc<AuthManager>,
    api_key: Option<String>,
}

#[bon::bon]
impl CursorExecutor {
    /// Creates a new Cursor executor. An `api_key` (`crsr_…`) is exchanged
    /// for an access token per request; otherwise the stored login is used.
    #[builder]
    pub fn new(http: wreq::Client, auth: Arc<AuthManager>, api_key: Option<String>) -> Self {
        Self {
            http,
            auth,
            api_key,
        }
    }

    /// An access token for this request: a configured or stored `crsr_` key
    /// is exchanged (and the result cached); a browser login is used as is.
    async fn access_token(&self) -> Result<String> {
        let credential = match &self.api_key {
            Some(key) => key.clone(),
            None => self.auth.get_token(&ProviderId::Cursor).await?.access_token,
        };
        if !credential.starts_with("crsr_") {
            return Ok(credential);
        }
        if let Some(token) = EXCHANGED.lock().expect("token cache lock").get(&credential)
            && !token.is_expired()
        {
            return Ok(token.access_token.clone());
        }
        let token = byokey_auth::provider::cursor::exchange(&self.http, &credential).await?;
        let access = token.access_token.clone();
        EXCHANGED
            .lock()
            .expect("token cache lock")
            .insert(credential, token);
        Ok(access)
    }

    /// Model names the account can use, from Cursor's live catalog.
    ///
    /// # Errors
    ///
    /// Returns an error if there is no credential or the catalog cannot be fetched.
    pub async fn models(&self) -> Result<Vec<String>> {
        let token = self.access_token().await?;
        models::names(&self.http, API_BASE, &token, CLIENT_VERSION).await
    }

    /// Run one turn of `request` and stream its canonical events.
    ///
    /// # Errors
    ///
    /// Returns an error if the model is unknown or the stream cannot start.
    pub async fn events(&self, request: CanonicalRequest) -> Result<EventStream> {
        let model_name = request.model.clone();
        let results = tool_results(&request);
        let run = match take_parked(&results) {
            Some(run) => {
                for (id, text) in results {
                    run.tool_result(id, text, false).await?;
                }
                run
            }
            None => self.start(&request).await?,
        };
        let want_thinking = request.thinking.is_some();
        Ok(turn_events(run, model_name, want_thinking))
    }

    async fn start(&self, request: &CanonicalRequest) -> Result<Run> {
        let token = self.access_token().await?;
        let mut model =
            models::resolve(&self.http, API_BASE, &token, CLIENT_VERSION, &request.model).await?;
        // Thinking costs time to first token; skip it unless it was asked for.
        if request.thinking.is_none()
            && !request.model.contains("think")
            && let Some(slot) = model
                .params
                .iter_mut()
                .find(|(k, v)| k == "thinking" && v == "true")
        {
            slot.1 = "false".into();
        }
        let (system, prompt) = render(&request.messages);
        let tools = match request
            .tool_choice
            .as_ref()
            .map(|c| serde_json::to_value(c).unwrap_or_default())
        {
            Some(Value::String(s)) if s == "none" => Vec::new(),
            _ => tool_specs(request),
        };
        let system = with_tool_choice(system, request);
        let spec = RunSpec {
            model: model.id,
            params: model.params,
            system,
            prompt,
            tools,
        };
        let endpoint = session::Endpoint {
            url: RUN_URL.into(),
            client_version: CLIENT_VERSION.into(),
        };
        session::start(&self.http, &endpoint, &token, spec).await
    }
}

/// Tool results at the end of `request`, if it follows up on a tool call.
fn tool_results(request: &CanonicalRequest) -> Vec<(String, String)> {
    request
        .messages
        .iter()
        .rev()
        .take_while(|m| m.role == Role::Tool)
        .filter_map(|m| Some((m.tool_call_id.clone()?, text_of(m.content.as_ref()))))
        .collect()
}

fn park_key<'a>(ids: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut key: Vec<String> = ids.cloned().collect();
    key.sort();
    key
}

/// Take the parked run waiting on exactly these tool results, if any.
fn take_parked(results: &[(String, String)]) -> Option<Run> {
    let mut parked = PARKED.lock().expect("parked lock");
    // Dropping an expired run closes its stream.
    parked.retain(|_, (_, since)| since.elapsed() < PARK_TTL);
    parked
        .remove(&park_key(results.iter().map(|(id, _)| id)))
        .map(|(run, _)| run)
}

/// Accumulates one turn: maps session events to canonical stream events.
struct Turn {
    want_thinking: bool,
    thinking: bool,
    tools: Vec<String>,
}

impl Turn {
    /// Canonical events for `event`, and whether the turn is over.
    fn on(&mut self, event: Option<Event>) -> (Vec<Result<StreamEvent>>, bool) {
        let mut out = Vec::new();
        if self.thinking && !matches!(event, Some(Event::Thinking(_))) {
            self.thinking = false;
            out.push(Ok(StreamEvent::ReasoningEnd {
                index: 0,
                signature: String::new(),
            }));
        }
        match event {
            Some(Event::Thinking(t)) if self.want_thinking => {
                if !self.thinking {
                    self.thinking = true;
                    out.push(Ok(StreamEvent::ReasoningStart {
                        index: 0,
                        source: None,
                    }));
                }
                out.push(Ok(StreamEvent::ReasoningDelta(t)));
            }
            Some(Event::Thinking(_)) => {}
            Some(Event::Text(t)) => out.push(Ok(StreamEvent::ContentDelta(t))),
            Some(Event::ToolCall(call)) => {
                let index = u32::try_from(self.tools.len()).unwrap_or(u32::MAX);
                out.push(Ok(StreamEvent::ToolCallStart {
                    index,
                    id: call.id.clone(),
                    name: call.name,
                }));
                out.push(Ok(StreamEvent::ToolCallDelta {
                    index,
                    arguments: call.input.to_string(),
                }));
                self.tools.push(call.id);
            }
            Some(Event::TurnEnded(usage)) => {
                let finish = if self.tools.is_empty() {
                    FinishReason::Stop
                } else {
                    FinishReason::ToolCalls
                };
                out.push(Ok(StreamEvent::Finish(finish)));
                if usage.input + usage.output > 0 {
                    out.push(Ok(StreamEvent::Usage(usage_of(usage))));
                }
                out.push(Ok(StreamEvent::Done));
                return (out, true);
            }
            Some(Event::Failed(e)) => {
                out.push(Err(e));
                return (out, true);
            }
            None => {
                out.push(Err(ByokError::Http("Cursor stream ended mid-turn".into())));
                return (out, true);
            }
        }
        (out, false)
    }
}

/// Stream one turn of `run`. If the turn stops on tool calls, the run is
/// parked for the request that carries their results.
fn turn_events(run: Run, model: String, want_thinking: bool) -> EventStream {
    let meta = StreamEvent::ResponseMeta {
        id: format!("cursor-{}", uuid::Uuid::new_v4().simple()),
        model,
    };
    let turn = Turn {
        want_thinking,
        thinking: false,
        tools: Vec::new(),
    };
    let body = stream::unfold(Some((run, turn)), |state| async move {
        let (mut run, mut turn) = state?;
        let event = if turn.tools.is_empty() {
            run.events.recv().await
        } else {
            // ponytail: a tool call leaves the agent waiting, so the turn ends once
            // no further parallel call arrives within 200ms; an upstream "calls
            // done" marker would be exact if Cursor sends one.
            tokio::time::timeout(Duration::from_millis(200), run.events.recv())
                .await
                .unwrap_or(Some(Event::TurnEnded(session::TurnUsage::default())))
        };
        let (out, over) = turn.on(event);
        let next = if over {
            if !turn.tools.is_empty() {
                let key = park_key(turn.tools.iter());
                PARKED
                    .lock()
                    .expect("parked lock")
                    .insert(key, (run, Instant::now()));
            }
            None
        } else {
            Some((run, turn))
        };
        Some((stream::iter(out), next))
    });
    Box::pin(stream::once(async { Ok(meta) }).chain(body.flatten()))
}

fn usage_of(u: session::TurnUsage) -> Usage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "prompt_tokens_details".into(),
        json!({ "cached_tokens": u.cache_read }),
    );
    Usage {
        prompt_tokens: Some(u.input),
        completion_tokens: Some(u.output),
        total_tokens: Some(u.input + u.output),
        extra,
    }
}

fn text_of(content: Option<&MessageContent>) -> String {
    match content {
        Some(MessageContent::Text(t)) => t.clone(),
        Some(MessageContent::Parts(parts)) => parts
            .iter()
            .filter_map(|p| match p {
                ForwardCompatible::Known(TypedContentPart::Text { text, .. }) => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        None => String::new(),
    }
}

/// Split messages into a system prompt and the turn text. Every fresh run is
/// a new Cursor conversation, so earlier turns travel as a transcript.
fn render(messages: &[Message]) -> (Option<String>, String) {
    let mut system = Vec::new();
    let mut lines = Vec::new();
    for m in messages {
        match m.role {
            Role::System | Role::Developer => system.push(text_of(m.content.as_ref())),
            Role::Tool => lines.push(format!(
                "Human: [tool result: {}]",
                text_of(m.content.as_ref())
            )),
            Role::Assistant => {
                let mut body = text_of(m.content.as_ref());
                for call in m.tool_calls.iter().flatten() {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    let _ = write!(
                        body,
                        "[called tool {} with {}]",
                        call.function.name, call.function.arguments
                    );
                }
                lines.push(format!("Assistant: {body}"));
            }
            _ => lines.push(format!("Human: {}", text_of(m.content.as_ref()))),
        }
    }
    let system = (!system.is_empty()).then(|| system.join("\n\n"));
    let prompt = match lines.as_slice() {
        [only] => only.strip_prefix("Human: ").unwrap_or(only).to_owned(),
        _ => format!("{}\n\nAssistant:", lines.join("\n\n")),
    };
    (system, prompt)
}

fn tool_specs(request: &CanonicalRequest) -> Vec<ToolSpec> {
    request
        .tools
        .iter()
        .flatten()
        .map(|t| ToolSpec {
            name: t.function.name.clone(),
            description: t.function.description.clone().unwrap_or_default(),
            schema: t
                .function
                .parameters
                .clone()
                .unwrap_or_else(|| json!({"type": "object"})),
        })
        .collect()
}

/// Cursor has no `tool_choice`; required tools are asked for in the prompt.
fn with_tool_choice(system: Option<String>, request: &CanonicalRequest) -> Option<String> {
    let choice = request
        .tool_choice
        .as_ref()
        .map(|c| serde_json::to_value(c).unwrap_or_default());
    let line = match &choice {
        Some(Value::String(s)) if s == "required" => {
            "You must answer by calling one of the provided tools.".to_owned()
        }
        Some(Value::Object(o)) => match o
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
        {
            Some(name) => format!("You must answer by calling the tool `{name}`."),
            None => return system,
        },
        _ => return system,
    };
    Some(match system {
        Some(s) => format!("{s}\n\n{line}"),
        None => line,
    })
}

/// Collect a turn into a canonical chat response.
///
/// # Errors
///
/// Returns the first error the turn's stream reports.
pub async fn collect(mut events: EventStream) -> Result<ChatResponse> {
    let (mut id, mut model, mut text, mut reasoning) =
        (String::new(), String::new(), String::new(), String::new());
    let mut calls: Vec<Value> = Vec::new();
    let mut finish = "stop";
    let mut usage = Value::Null;
    while let Some(event) = events.next().await {
        match event? {
            StreamEvent::ResponseMeta { id: i, model: m } => (id, model) = (i, m),
            StreamEvent::ContentDelta(t) => text.push_str(&t),
            StreamEvent::ReasoningDelta(t) => reasoning.push_str(&t),
            StreamEvent::ToolCallStart { id, name, .. } => {
                calls.push(json!({"id": id, "type": "function", "function": {"name": name, "arguments": ""}}));
            }
            StreamEvent::ToolCallDelta { arguments, .. } => {
                if let Some(Value::String(a)) = calls
                    .last_mut()
                    .and_then(|c| c.pointer_mut("/function/arguments"))
                {
                    a.push_str(&arguments);
                }
            }
            StreamEvent::Finish(FinishReason::ToolCalls) => finish = "tool_calls",
            StreamEvent::Usage(u) => usage = serde_json::to_value(u).unwrap_or_default(),
            _ => {}
        }
    }
    let mut message = json!({"role": "assistant", "content": text});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls);
    }
    serde_json::from_value(json!({
        "id": id,
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
        "usage": usage,
    }))
    .map_err(|e| ByokError::Translation(e.to_string()))
}

#[async_trait]
impl ProviderExecutor for CursorExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let canonical: CanonicalRequest = serde_json::from_value(request.into_body())
            .map_err(|e| ByokError::Translation(e.to_string()))?;
        let events = self.events(canonical).await?;
        if !stream {
            let response = serde_json::to_value(collect(events).await?)
                .map_err(|e| ByokError::Translation(e.to_string()))?;
            return Ok(ProviderResponse::Complete(response));
        }
        let mut ctx = SseContext::default();
        let sse: ByteStream = Box::pin(events.map(move |e| {
            e.map(|event| Bytes::from(stream_events_to_sse(std::slice::from_ref(&event), &mut ctx)))
        }));
        Ok(ProviderResponse::Stream(sse))
    }

    fn supported_models(&self) -> Vec<String> {
        crate::registry::models_for_provider(&ProviderId::Cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(v: Value) -> Message {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn single_user_turn_is_sent_bare() {
        let (system, prompt) = render(&[
            msg(json!({"role": "system", "content": "be brief"})),
            msg(json!({"role": "user", "content": "hi"})),
        ]);
        assert_eq!(system.as_deref(), Some("be brief"));
        assert_eq!(prompt, "hi");
    }

    #[test]
    fn history_becomes_a_transcript() {
        let (_, prompt) = render(&[
            msg(json!({"role": "user", "content": "weather?"})),
            msg(json!({"role": "assistant", "content": null, "tool_calls": [
                {"id": "t1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}}]})),
            msg(json!({"role": "tool", "tool_call_id": "t1", "content": "rain"})),
        ]);
        assert_eq!(
            prompt,
            "Human: weather?\n\nAssistant: [called tool get_weather with {}]\n\nHuman: [tool result: rain]\n\nAssistant:"
        );
    }

    #[test]
    fn tool_choice_becomes_an_instruction() {
        let req: CanonicalRequest = serde_json::from_value(json!({
            "model": "m", "messages": [],
            "tool_choice": {"type": "function", "function": {"name": "get_weather"}}
        }))
        .unwrap();
        assert_eq!(
            with_tool_choice(None, &req).as_deref(),
            Some("You must answer by calling the tool `get_weather`.")
        );
    }

    fn kinds(events: &[Result<StreamEvent>]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                Ok(StreamEvent::ReasoningStart { .. }) => "reasoning_start",
                Ok(StreamEvent::ReasoningDelta(_)) => "reasoning",
                Ok(StreamEvent::ReasoningEnd { .. }) => "reasoning_end",
                Ok(StreamEvent::ContentDelta(_)) => "text",
                Ok(StreamEvent::ToolCallStart { .. }) => "tool_start",
                Ok(StreamEvent::ToolCallDelta { .. }) => "tool_args",
                Ok(StreamEvent::Finish(FinishReason::ToolCalls)) => "finish_tools",
                Ok(StreamEvent::Finish(_)) => "finish",
                Ok(StreamEvent::Usage(_)) => "usage",
                Ok(StreamEvent::Done) => "done",
                Ok(_) => "other",
                Err(_) => "error",
            })
            .collect()
    }

    #[test]
    fn turn_closes_reasoning_before_text_and_reports_tool_calls() {
        let mut turn = Turn {
            want_thinking: true,
            thinking: false,
            tools: Vec::new(),
        };
        let mut all = Vec::new();
        for event in [
            Event::Thinking("hm".into()),
            Event::Thinking("…".into()),
            Event::Text("ok".into()),
            Event::ToolCall(session::ToolCall {
                id: "t1".into(),
                name: "f".into(),
                input: json!({}),
            }),
        ] {
            let (out, over) = turn.on(Some(event));
            assert!(!over);
            all.extend(out);
        }
        let (out, over) = turn.on(Some(Event::TurnEnded(session::TurnUsage {
            input: 1,
            output: 2,
            cache_read: 0,
        })));
        assert!(over);
        all.extend(out);
        assert_eq!(
            kinds(&all),
            [
                "reasoning_start",
                "reasoning",
                "reasoning",
                "reasoning_end",
                "text",
                "tool_start",
                "tool_args",
                "finish_tools",
                "usage",
                "done"
            ]
        );
        assert_eq!(turn.tools, ["t1"]);
    }

    #[test]
    fn unrequested_thinking_is_dropped_and_a_closed_stream_is_an_error() {
        let mut turn = Turn {
            want_thinking: false,
            thinking: false,
            tools: Vec::new(),
        };
        assert!(turn.on(Some(Event::Thinking("hm".into()))).0.is_empty());
        let (out, over) = turn.on(None);
        assert!(over);
        assert_eq!(kinds(&out), ["error"]);
    }

    #[test]
    fn parked_runs_are_found_by_their_tool_ids_in_any_order() {
        assert_eq!(
            park_key(["b".to_owned(), "a".to_owned()].iter()),
            park_key(["a".to_owned(), "b".to_owned()].iter())
        );
    }
}
