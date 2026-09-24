//! One Cursor `agent.v1.AgentService/Run` stream, owned by a background task.
//!
//! Cursor's agent protocol is a bidirectional Connect stream: after the run
//! request the server drives a conversation of its own, asking the client to
//! supply request context, persist blobs, approve web searches, and run its
//! builtin file/shell tools. Every such request must be answered or the stream
//! stalls for good, so a task answers them for the whole life of the stream —
//! including while a turn is parked on a caller-side tool call.
//!
//! The task surfaces only what the API caller needs as [`Event`]s: text,
//! thinking, caller tool calls, and the end of the turn.

use super::pb::{Fields, Msg, decode_json_value};
use byokey_types::{ByokError, Result};
use bytes::{Buf as _, Bytes, BytesMut};
use futures_util::StreamExt as _;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read as _;
use tokio::sync::mpsc;

/// Cursor's builtin tool names. A caller tool whose normalised name matches
/// one makes Cursor reject the whole run, so such tools travel renamed.
const BUILTIN_TOOLS: &[&str] = &[
    "read",
    "write",
    "ls",
    "delete",
    "grep",
    "glob",
    "shell",
    "web_search",
    "web_fetch",
];

/// Workspace path announced to the agent. Nothing lives there: the caller's
/// machine is reachable only through the caller's own tools.
const WORKSPACE: &str = "/workspace";

const TOOLS_PROMPT: &str = "When a task needs one of the provided tools, call the tool \
directly instead of describing it. Your builtin file, search and shell tools are not \
connected to this machine and will fail; use only the provided tools, and use the paths \
given in the conversation rather than the workspace path you were told about.";

const CHAT_PROMPT: &str = "You are answering a single API request in a plain conversation. \
There is no workspace, repository, project or user machine attached, and no tools are \
available: never call file, search, terminal, todo or task tools, never look for context, \
and never mention a workspace, codebase or your environment. Answer the user's message \
directly, as a general-purpose assistant, in the language the user used.";

/// A caller tool offered to the agent.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// Everything needed to start a run.
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub model: String,
    pub params: Vec<(String, String)>,
    pub system: Option<String>,
    pub prompt: String,
    pub tools: Vec<ToolSpec>,
}

/// Token counts reported when a turn ends.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
}

/// A caller tool call the agent is waiting on.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// What the stream reports to the API layer.
#[derive(Debug)]
pub enum Event {
    Text(String),
    Thinking(String),
    ToolCall(ToolCall),
    /// The agent finished the turn.
    TurnEnded(TurnUsage),
    /// The stream failed or closed; no more events follow.
    Failed(ByokError),
}

/// What the API layer can send back into a live run.
#[derive(Debug)]
enum Command {
    ToolResult {
        id: String,
        text: String,
        is_error: bool,
    },
}

/// Handle to a live run. Dropping it (and every clone of its command sender)
/// ends the stream.
pub struct Run {
    pub events: mpsc::Receiver<Event>,
    commands: mpsc::Sender<Command>,
}

impl Run {
    /// Answer a caller tool call the agent is waiting on.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream has already ended.
    pub async fn tool_result(&self, id: String, text: String, is_error: bool) -> Result<()> {
        self.commands
            .send(Command::ToolResult { id, text, is_error })
            .await
            .map_err(|_| ByokError::Http("Cursor stream closed before the tool result".into()))
    }
}

/// Connection settings shared by every run.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub url: String,
    pub client_version: String,
}

/// Start a run and return its handle once Cursor accepted the stream.
///
/// # Errors
///
/// Returns an error if the request cannot be sent or Cursor answers with a
/// non-200 status.
pub async fn start(
    http: &wreq::Client,
    endpoint: &Endpoint,
    access_token: &str,
    spec: RunSpec,
) -> Result<Run> {
    let tools = Tools::new(&spec.tools);
    let run_id = uuid::Uuid::new_v4().to_string();
    let (out_tx, out_rx) = mpsc::channel::<std::result::Result<Bytes, std::io::Error>>(16);
    out_tx
        .send(Ok(frame(&run_request(&spec, &tools, &run_id))))
        .await
        .map_err(|_| ByokError::Http("Cursor request body closed".into()))?;

    let resp = http
        .post(&endpoint.url)
        .version(http::Version::HTTP_2)
        .bearer_auth(access_token)
        .header("content-type", "application/connect+proto")
        .header("connect-protocol-version", "1")
        .header("connect-accept-encoding", "gzip")
        .header("user-agent", "connect-es/1.6.1")
        .header("x-cursor-client-type", "cli")
        .header("x-cursor-client-version", &endpoint.client_version)
        .header("x-ghost-mode", "false")
        .header("x-request-id", &run_id)
        .header("x-original-request-id", &run_id)
        .body(wreq::Body::wrap_stream(
            tokio_stream::wrappers::ReceiverStream::new(out_rx),
        ))
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ByokError::Upstream {
            status: status.as_u16(),
            body,
            retry_after: None,
        });
    }

    let (ev_tx, ev_rx) = mpsc::channel(64);
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let task = Task {
        out: out_tx,
        events: ev_tx,
        tools,
        chat: spec.tools.is_empty(),
        blobs: HashMap::new(),
        pending: HashMap::new(),
    };
    tokio::spawn(task.drive(resp, cmd_rx));
    Ok(Run {
        events: ev_rx,
        commands: cmd_tx,
    })
}

// ── Tool naming ─────────────────────────────────────────────────────────────

fn normalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(if c == '-' {
            '_'
        } else {
            c.to_ascii_lowercase()
        });
    }
    out
}

/// Caller tool names and the names they travel under.
#[derive(Debug, Default)]
struct Tools {
    /// `(caller name, wire name)` in declaration order.
    names: Vec<(String, String)>,
}

impl Tools {
    fn new(specs: &[ToolSpec]) -> Self {
        let mut taken: Vec<String> = specs.iter().map(|t| t.name.clone()).collect();
        let names = specs
            .iter()
            .map(|t| {
                let mut wire = t.name.clone();
                while BUILTIN_TOOLS.contains(&normalise(&wire).as_str())
                    || (wire != t.name && taken.contains(&wire))
                {
                    wire.push('_');
                }
                taken.push(wire.clone());
                (t.name.clone(), wire)
            })
            .collect();
        Self { names }
    }

    fn wire<'a>(&'a self, name: &'a str) -> &'a str {
        self.names
            .iter()
            .find(|(c, _)| c == name)
            .map_or(name, |(_, w)| w)
    }

    fn caller(&self, wire: &str) -> String {
        self.names
            .iter()
            .find(|(_, w)| w == wire)
            .map_or_else(|| wire.to_owned(), |(c, _)| c.clone())
    }

    /// The caller's tool that does what builtin exec `field` does, if any.
    fn equivalent(&self, field: u32) -> Option<&str> {
        let candidates: &[&str] = match field {
            2 | 14 | 52 => &["shell", "bash", "terminal", "run_command"],
            3 => &["write", "write_file", "create_file", "edit"],
            4 => &["delete", "remove", "delete_file"],
            5 => &["grep", "search", "ripgrep"],
            7 | 29 => &["read", "read_file", "view", "cat"],
            8 => &["ls", "list_dir", "glob", "list_files"],
            _ => return None,
        };
        candidates.iter().find_map(|cand| {
            self.names
                .iter()
                .find(|(c, _)| normalise(c) == *cand)
                .map(|(_, w)| w.as_str())
        })
    }
}

// ── Outbound messages ───────────────────────────────────────────────────────

/// Connect envelope: flags byte, big-endian length, payload.
fn frame(payload: &[u8]) -> Bytes {
    let mut out = BytesMut::with_capacity(payload.len() + 5);
    out.extend_from_slice(&[0]);
    out.extend_from_slice(
        &u32::try_from(payload.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(payload);
    out.freeze()
}

fn run_request(spec: &RunSpec, tools: &Tools, run_id: &str) -> Bytes {
    // `custom_system_prompt` is gated server-side, so instructions travel in
    // the turn text.
    let mut system = spec.system.clone().unwrap_or_default();
    let extra = if spec.tools.is_empty() {
        CHAT_PROMPT
    } else {
        TOOLS_PROMPT
    };
    if !system.is_empty() {
        system.push_str("\n\n");
    }
    system.push_str(extra);
    let text = format!("<system>\n{system}\n</system>\n\n{}", spec.prompt);

    let conversation = uuid::Uuid::new_v4().to_string();
    let user_message = Msg::new()
        .str(1, &text)
        .str(2, &uuid::Uuid::new_v4().to_string())
        .varint(4, 1);
    let action = Msg::new().msg(1, &Msg::new().msg(1, &user_message));
    let mut model = Msg::new().str(1, &spec.model);
    for (k, v) in &spec.params {
        model = model.msg(3, &Msg::new().str(1, k).str(2, v));
    }
    let mut run = Msg::new()
        .bytes(1, b"")
        .msg(2, &action)
        .str(5, &conversation)
        .msg(9, &model)
        .str(16, &conversation)
        .str(25, run_id)
        .bool(19, true)
        .varint(12, 0);
    if !spec.tools.is_empty() {
        let mut defs = Msg::new();
        for t in &spec.tools {
            let wire = tools.wire(&t.name);
            let description: String = t.description.chars().take(4000).collect();
            defs = defs.msg(
                1,
                &Msg::new()
                    .str(1, wire)
                    .str(2, &description)
                    .str(4, "anthropic-passthrough")
                    .str(5, wire)
                    .str(6, &t.schema.to_string()),
            );
        }
        run = run.msg(4, &defs);
    }
    Msg::new().msg(1, &run).finish()
}

/// Identity of a server exec request, echoed on its reply.
#[derive(Debug, Clone, Default)]
struct ExecId {
    num: u64,
    text: Option<String>,
}

impl ExecId {
    fn of(f: &Fields<'_>) -> Self {
        Self {
            num: f.varint(1).unwrap_or(0),
            text: f.str(15).map(str::to_owned),
        }
    }

    /// `AgentClientMessage{exec_client_message = {<field>: body, id…}}`.
    fn reply(&self, field: u32, body: &Msg) -> Bytes {
        let mut inner = Msg::new().msg(field, body);
        if self.num != 0 {
            inner = inner.varint(1, self.num);
        }
        if let Some(t) = &self.text {
            inner = inner.str(15, t);
        }
        Msg::new().msg(2, &inner).finish()
    }
}

/// Reply refusing builtin exec `field` with `text`, in that tool's error shape.
fn refusal(field: u32, text: &str) -> (u32, Msg) {
    let at = |outer: u32, inner: u32| Msg::new().msg(outer, &Msg::new().str(inner, text));
    match field {
        2 => (2, at(7, 3)),
        3 => (3, at(5, 2)),
        4 => (4, at(7, 2)),
        5 => (5, at(2, 1)),
        14 => (14, at(6, 3)),
        52 => (55, at(7, 3)),
        n => (n, at(2, 2)),
    }
}

fn context_reply(web: bool) -> Msg {
    let env = Msg::new()
        .str(1, "Linux 6.8")
        .str(3, "/bin/bash")
        .bool(5, false)
        .str(7, &format!("{WORKSPACE}/.terminals"))
        .str(8, &format!("{WORKSPACE}/.notes"))
        .str(9, &format!("{WORKSPACE}/.cnotes"))
        .str(10, "UTC")
        .str(11, WORKSPACE)
        .str(12, &format!("{WORKSPACE}/.transcripts"));
    let mut ctx = Msg::new().msg(4, &env);
    if web {
        ctx = ctx.bool(17, true).bool(24, true);
    }
    Msg::new().msg(1, &Msg::new().msg(1, &ctx))
}

// ── Inbound decoding ────────────────────────────────────────────────────────

/// Split complete Connect envelopes off `buf`, gunzipping compressed ones.
fn deframe(buf: &mut BytesMut) -> Result<Vec<(u8, Bytes)>> {
    let mut out = Vec::new();
    while buf.len() >= 5 {
        let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        if buf.len() < 5 + len {
            break;
        }
        let flags = buf[0];
        buf.advance(5);
        let payload = buf.split_to(len).freeze();
        let payload = if flags & 1 == 0 {
            payload
        } else {
            let mut raw = Vec::new();
            flate2::read::GzDecoder::new(&payload[..])
                .read_to_end(&mut raw)
                .map_err(|e| ByokError::Http(format!("Cursor sent a bad gzip frame: {e}")))?;
            Bytes::from(raw)
        };
        out.push((flags, payload));
    }
    Ok(out)
}

/// Map a Connect end-of-stream error to a [`ByokError`].
fn trailer_error(json: &str) -> Option<ByokError> {
    let v: Value = serde_json::from_str(json).ok()?;
    let err = v.get("error")?;
    let code = err.get("code").and_then(Value::as_str).unwrap_or("unknown");
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text = format!("{code}: {message}");
    let upper = json.to_ascii_uppercase();
    let status = if upper.contains("RATE_LIMIT") || code == "resource_exhausted" {
        429
    } else if upper.contains("NOT_LOGGED_IN") || code == "unauthenticated" {
        401
    } else if upper.contains("MODEL_BLOCKED") || code == "permission_denied" {
        403
    } else if upper.contains("MODEL_NOT_AVAILABLE") || upper.contains("MODEL_NOT_SUPPORTED") {
        400
    } else {
        502
    };
    Some(ByokError::Upstream {
        status,
        body: text,
        retry_after: None,
    })
}

// ── The driving task ────────────────────────────────────────────────────────

struct Task {
    out: mpsc::Sender<std::result::Result<Bytes, std::io::Error>>,
    events: mpsc::Sender<Event>,
    tools: Tools,
    chat: bool,
    blobs: HashMap<Vec<u8>, Bytes>,
    /// Caller tool calls awaiting a result: tool call id → exec identity.
    pending: HashMap<String, ExecId>,
}

impl Task {
    async fn drive(mut self, resp: wreq::Response, mut commands: mpsc::Receiver<Command>) {
        let mut body = resp.bytes_stream();
        let mut buf = BytesMut::new();
        let failure = loop {
            tokio::select! {
                chunk = body.next() => match chunk {
                    Some(Ok(bytes)) => {
                        buf.extend_from_slice(&bytes);
                        match self.on_bytes(&mut buf).await {
                            Ok(true) => {}
                            Ok(false) => break None,
                            Err(e) => break Some(e),
                        }
                    }
                    Some(Err(e)) => break Some(e.into()),
                    None => break Some(ByokError::Http("Cursor closed the stream".into())),
                },
                cmd = commands.recv() => match cmd {
                    Some(Command::ToolResult { id, text, is_error }) => {
                        if let Err(e) = self.tool_result(&id, &text, is_error).await {
                            break Some(e);
                        }
                    }
                    // Every handle is gone: nobody will read further turns.
                    None => break None,
                },
            }
        };
        if let Some(e) = failure {
            let _ = self.events.send(Event::Failed(e)).await;
        }
    }

    async fn send(&self, payload: Bytes) -> Result<()> {
        self.out
            .send(Ok(frame(&payload)))
            .await
            .map_err(|_| ByokError::Http("Cursor request body closed".into()))
    }

    async fn emit(&self, event: Event) -> Result<()> {
        self.events
            .send(event)
            .await
            .map_err(|_| ByokError::Http("Cursor run abandoned".into()))
    }

    async fn tool_result(&mut self, id: &str, text: &str, is_error: bool) -> Result<()> {
        let exec = self
            .pending
            .remove(id)
            .ok_or_else(|| ByokError::Http(format!("no pending Cursor tool call {id}")))?;
        let content = Msg::new().msg(1, &Msg::new().str(1, text));
        let success = Msg::new().msg(1, &content).bool(2, is_error);
        self.send(exec.reply(11, &Msg::new().msg(1, &success)))
            .await
    }

    /// Handle every complete frame in `buf`. Returns `Ok(false)` once the
    /// server ended the stream cleanly.
    async fn on_bytes(&mut self, buf: &mut BytesMut) -> Result<bool> {
        for (flags, payload) in deframe(buf)? {
            if flags & 2 != 0 {
                let json = String::from_utf8_lossy(&payload);
                return match trailer_error(&json) {
                    Some(e) => Err(e),
                    None => Ok(false),
                };
            }
            self.on_message(&payload).await?;
        }
        Ok(true)
    }

    async fn on_message(&mut self, payload: &[u8]) -> Result<()> {
        let msg = Fields::parse(payload);
        if let Some(update) = msg.message(1) {
            return self.on_interaction(&update).await;
        }
        if let Some(exec) = msg.message(2) {
            return self.on_exec(&exec).await;
        }
        if let Some(kv) = msg.message(4) {
            return self.on_kv(&kv).await;
        }
        if let Some(query) = msg.message(7) {
            return self.on_query(&query).await;
        }
        Ok(())
    }

    async fn on_interaction(&self, update: &Fields<'_>) -> Result<()> {
        if let Some(text) = update.message(1).and_then(|t| t.str(1).map(str::to_owned))
            && !text.is_empty()
        {
            self.emit(Event::Text(text)).await?;
        }
        if let Some(text) = update.message(4).and_then(|t| t.str(1).map(str::to_owned))
            && !text.is_empty()
        {
            self.emit(Event::Thinking(text)).await?;
        }
        if let Some(end) = update.message(14) {
            let n = |f| end.varint(f).unwrap_or(0);
            let usage = TurnUsage {
                input: n(1),
                output: n(2),
                cache_read: n(3),
            };
            self.emit(Event::TurnEnded(usage)).await?;
        }
        Ok(())
    }

    async fn on_exec(&mut self, exec: &Fields<'_>) -> Result<()> {
        let id = ExecId::of(exec);
        if exec.has(10) {
            return self.send(id.reply(10, &context_reply(!self.chat))).await;
        }
        if let Some(args) = exec.message(11) {
            let wire = args.str(5).or_else(|| args.str(1)).unwrap_or_default();
            let input: serde_json::Map<String, Value> = args
                .all_bytes(2)
                .filter_map(|kv| {
                    let kv = Fields::parse(kv);
                    let value = kv.bytes(2).map_or(Value::Null, decode_json_value);
                    Some((kv.str(1)?.to_owned(), value))
                })
                .collect();
            let call_id = args.str(3).filter(|s| !s.is_empty()).map_or_else(
                || format!("toolu_{}", uuid::Uuid::new_v4().simple()),
                str::to_owned,
            );
            self.pending.insert(call_id.clone(), id);
            return self
                .emit(Event::ToolCall(ToolCall {
                    id: call_id,
                    name: self.tools.caller(wire),
                    input: Value::Object(input),
                }))
                .await;
        }
        // Allowlist prechecks: always allowed.
        if let Some(field) = [41, 42, 43].into_iter().find(|f| exec.has(*f)) {
            return self.send(id.reply(field, &Msg::new().bool(1, true))).await;
        }
        // Anything else is a builtin tool; refuse it, pointing at the caller's
        // equivalent tool when there is one.
        let Some(field) = exec.numbers().find(|f| !matches!(f, 1 | 15 | 19 | 55)) else {
            return Ok(());
        };
        let text = if self.chat {
            "This tool is not available: there is no workspace, repository or user machine in \
             this conversation. Answer the user directly instead of calling tools."
                .to_owned()
        } else if let Some(tool) = self.tools.equivalent(field) {
            format!(
                "This tool is not connected to the user's machine. Call the tool '{tool}' instead."
            )
        } else {
            let names: Vec<&str> = self
                .tools
                .names
                .iter()
                .map(|(_, w)| w.as_str())
                .take(20)
                .collect();
            format!(
                "This tool is not connected to the user's machine. Use the provided tools ({}) instead.",
                names.join(", ")
            )
        };
        let (reply_field, body) = refusal(field, &text);
        self.send(id.reply(reply_field, &body)).await?;
        if field == 14 && id.num != 0 {
            // Streamed execs must be closed explicitly.
            let close = Msg::new().msg(1, &Msg::new().varint(1, id.num));
            self.send(Msg::new().msg(5, &close).finish()).await?;
        }
        Ok(())
    }

    async fn on_kv(&mut self, kv: &Fields<'_>) -> Result<()> {
        let kid = kv.varint(1).unwrap_or(0);
        if let Some(set) = kv.message(3) {
            let key = set.bytes(1).unwrap_or_default().to_vec();
            let data = Bytes::copy_from_slice(set.bytes(2).unwrap_or_default());
            self.blobs.insert(key, data);
            let ack = Msg::new().varint(1, kid).msg(3, &Msg::new());
            return self.send(Msg::new().msg(3, &ack).finish()).await;
        }
        if let Some(get) = kv.message(2) {
            let key = get.bytes(1).unwrap_or_default();
            let result = match self.blobs.get(key) {
                Some(data) => Msg::new().bytes(1, data),
                None => Msg::new().msg(2, &Msg::new().str(1, "not found")),
            };
            let reply = Msg::new().varint(1, kid).msg(2, &result);
            return self.send(Msg::new().msg(3, &reply).finish()).await;
        }
        Ok(())
    }

    /// Web search (2) and web fetch (9) approvals are granted.
    async fn on_query(&self, query: &Fields<'_>) -> Result<()> {
        let qid = query.varint(1).unwrap_or(0);
        for field in [2, 9] {
            if query.has(field) {
                let resp = Msg::new()
                    .varint(1, qid)
                    .msg(field, &Msg::new().bytes(1, b""));
                self.send(Msg::new().msg(6, &resp).finish()).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: String::new(),
            schema: Value::Null,
        }
    }

    #[test]
    fn tools_colliding_with_builtins_are_renamed_and_mapped_back() {
        let tools = Tools::new(&[
            spec("Read"),
            spec("Read_"),
            spec("WebSearch"),
            spec("get_weather"),
        ]);
        assert_eq!(tools.wire("Read"), "Read__");
        assert_eq!(tools.wire("Read_"), "Read_");
        assert_eq!(tools.wire("WebSearch"), "WebSearch_");
        assert_eq!(tools.wire("get_weather"), "get_weather");
        assert_eq!(tools.caller("Read__"), "Read");
        assert_eq!(tools.caller("WebSearch_"), "WebSearch");
    }

    #[test]
    fn builtin_execs_point_at_the_callers_equivalent_tool() {
        let tools = Tools::new(&[spec("Bash"), spec("Read")]);
        assert_eq!(tools.equivalent(2), Some("Bash"));
        assert_eq!(tools.equivalent(7), Some("Read_"));
        assert_eq!(tools.equivalent(5), None);
    }

    #[test]
    fn deframe_keeps_partial_frames_buffered() {
        let a = frame(b"abc");
        let mut buf = BytesMut::from(&a[..]);
        buf.extend_from_slice(&frame(b"defg")[..6]);
        let frames = deframe(&mut buf).unwrap();
        assert_eq!(frames, vec![(0, Bytes::from_static(b"abc"))]);
        assert_eq!(buf.len(), 6);
    }

    #[test]
    fn trailer_errors_map_to_http_statuses() {
        let status = |json: &str| match trailer_error(json) {
            Some(ByokError::Upstream { status, .. }) => Some(status),
            _ => None,
        };
        assert_eq!(
            status(r#"{"error":{"code":"resource_exhausted","message":"x"}}"#),
            Some(429)
        );
        assert_eq!(status(r#"{"error":{"code":"unauthenticated"}}"#), Some(401));
        assert_eq!(
            status(r#"{"error":{"code":"internal","message":"ERROR_MODEL_BLOCKED"}}"#),
            Some(403)
        );
        assert_eq!(status(r#"{"metadata":{}}"#), None);
    }
}
