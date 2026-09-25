//! The chat wire: ACP v1, one JSON-RPC message per line over `~/.fermix/acp.sock`,
//! after Fermix's own bridge handshake (engine `Channels.Acp.Peer`). Pure: it
//! turns lines into typed messages and back, and does no I/O.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Map, Value};
use std::fmt;
use std::sync::Arc;

pub const BRIDGE_VERSION: u64 = 1;
pub const PROTOCOL_VERSION: u64 = 1;
/// The engine's line cap (`Acp.Wire.max_line_bytes/0`); a longer line is broken.
pub const MAX_LINE_BYTES: usize = 10 * 1024 * 1024;
/// ACP carries no files, so the engine names one it could not send in a reply
/// chunk of its own (engine `Acp.Peer.attachment_line/1`).
const ATTACHMENT_OPEN: &str = "[attachment: ";
const ATTACHMENT_CLOSE: &str = " — not transferable over this surface]";
const CLIENT_NAME: &str = "fermix-desktop";
const METHOD_NOT_FOUND: i64 = -32601;

fn line(message: Value) -> String {
    let mut text = message.to_string();
    text.push('\n');
    text
}

/// The first line on a new connection, before any JSON-RPC.
pub fn handshake_line(app_version: &str) -> String {
    line(json!({"fermix_bridge": BRIDGE_VERSION, "app_version": app_version, "env": {}}))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckError {
    /// The daemon answered and said no, in its own words.
    Refused(String),
    Malformed(String),
}

pub fn parse_ack(text: &str) -> Result<(), AckError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| AckError::Malformed(format!("not JSON: {e}")))?;
    let ack = value
        .get("fermix_bridge_ack")
        .ok_or_else(|| AckError::Malformed("no fermix_bridge_ack".into()))?;
    match ack.get("status").and_then(Value::as_str) {
        Some("ok") => Ok(()),
        Some("error") => Err(AckError::Refused(
            ack.get("message")
                .and_then(Value::as_str)
                .unwrap_or("the daemon refused the chat connection")
                .to_owned(),
        )),
        _ => Err(AckError::Malformed(format!("unknown ack: {ack}"))),
    }
}

fn request(id: u64, method: &str, params: Value) -> String {
    line(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
}

pub fn initialize(id: u64, app_version: &str) -> String {
    let params = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "clientCapabilities": {},
        "clientInfo": {"name": CLIENT_NAME, "version": app_version},
    });
    request(id, "initialize", params)
}

/// Opens a conversation. `cwd` is the host folder the assistant's tools work in.
pub fn new_session(id: u64, cwd: &str) -> String {
    assert!(
        cwd.starts_with('/'),
        "a session's workspace must be an absolute path: {cwd}"
    );
    request(id, "session/new", json!({"cwd": cwd, "mcpServers": []}))
}

pub fn prompt(id: u64, session_id: &str, text: &str) -> String {
    let params = json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]});
    request(id, "session/prompt", params)
}

/// Asks the running reply to stop; the prompt then answers `cancelled`.
pub fn cancel(session_id: &str) -> String {
    line(json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session_id}}))
}

/// The answer to any request from the agent: this client offers no file,
/// terminal or permission services, and saying so keeps the agent from waiting.
pub fn reply_unsupported(id: &Value) -> String {
    line(json!({"jsonrpc": "2.0", "id": id, "error": {
        "code": METHOD_NOT_FOUND, "message": "not supported by this client"}}))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Completed,
    Failed,
}

/// A picture in a reply, decoded from the wire but not yet into pixels.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub mime: String,
    pub bytes: Arc<[u8]>,
}

/// The type and size only: a picture's bytes would flood any log line.
impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("mime", &self.mime)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    MessageChunk(String),
    /// The assistant's reasoning, apart from the reply.
    ThoughtChunk(String),
    Image(Image),
    /// A file the assistant sent that ACP cannot carry, by name.
    Attachment(String),
    ToolCall {
        id: String,
        title: String,
        status: ToolStatus,
    },
    ToolUpdate {
        id: String,
        status: ToolStatus,
    },
    /// An update kind this app does not draw, by name.
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response {
        id: u64,
        result: Value,
    },
    Error {
        id: Option<u64>,
        code: i64,
        message: String,
    },
    Update {
        session_id: String,
        update: Update,
    },
    /// A request from the agent; answer it with `reply_unsupported`.
    Request {
        id: Value,
        method: String,
    },
    /// A notification this app has no use for.
    Ignored(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    Cancelled,
    Other(String),
}

impl StopReason {
    pub fn from_result(result: &Value) -> Option<StopReason> {
        let reason = result.get("stopReason")?.as_str()?;
        Some(match reason {
            "end_turn" => StopReason::EndTurn,
            "cancelled" => StopReason::Cancelled,
            other => StopReason::Other(other.to_owned()),
        })
    }
}

pub fn parse_line(text: &str) -> Result<Incoming, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let message = value.as_object().ok_or("not a JSON object")?;
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        return match message.get("id") {
            Some(id) if !id.is_null() => Ok(Incoming::Request {
                id: id.clone(),
                method: method.to_owned(),
            }),
            _ => notification(method, message.get("params")),
        };
    }
    if let Some(error) = message.get("error") {
        return Ok(Incoming::Error {
            id: message.get("id").and_then(Value::as_u64),
            code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }
    match (
        message.get("id").and_then(Value::as_u64),
        message.get("result"),
    ) {
        (Some(id), Some(result)) => Ok(Incoming::Response {
            id,
            result: result.clone(),
        }),
        _ => Err("neither a request, a notification nor a response".into()),
    }
}

fn notification(method: &str, params: Option<&Value>) -> Result<Incoming, String> {
    if method != "session/update" {
        return Ok(Incoming::Ignored(method.to_owned()));
    }
    let params = params
        .and_then(Value::as_object)
        .ok_or("session/update without params")?;
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or("session/update without a sessionId")?;
    let update = params
        .get("update")
        .and_then(Value::as_object)
        .ok_or("session/update without an update")?;
    Ok(Incoming::Update {
        session_id: session_id.to_owned(),
        update: decode_update(update)?,
    })
}

fn decode_update(update: &Map<String, Value>) -> Result<Update, String> {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .ok_or("an update without a sessionUpdate kind")?;
    let text = |key: &str| update.get(key).and_then(Value::as_str).map(str::to_owned);
    let decoded = match kind {
        "agent_message_chunk" => reply_content(update.get("content"))?,
        "agent_thought_chunk" => match content_text(update.get("content")) {
            Some(text) => Update::ThoughtChunk(text.to_owned()),
            None => Update::Other(kind.to_owned()),
        },
        "tool_call" => Update::ToolCall {
            id: text("toolCallId").ok_or("a tool_call without a toolCallId")?,
            title: text("title").unwrap_or_default(),
            status: tool_status(update.get("status")).unwrap_or(ToolStatus::Running),
        },
        "tool_call_update" => match tool_status(update.get("status")) {
            Some(status) => Update::ToolUpdate {
                id: text("toolCallId").ok_or("a tool_call_update without a toolCallId")?,
                status,
            },
            None => Update::Other(kind.to_owned()),
        },
        other => Update::Other(other.to_owned()),
    };
    Ok(decoded)
}

/// The text of a `text` content block, or `None` for any other kind.
fn content_text(content: Option<&Value>) -> Option<&str> {
    let content = content?;
    if content.get("type").and_then(Value::as_str) != Some("text") {
        return None;
    }
    Some(
        content
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
}

fn reply_content(content: Option<&Value>) -> Result<Update, String> {
    if let Some(text) = content_text(content) {
        return Ok(match attachment_name(text) {
            Some(name) => Update::Attachment(name.to_owned()),
            None => Update::MessageChunk(text.to_owned()),
        });
    }
    match content {
        Some(image) if image.get("type").and_then(Value::as_str) == Some("image") => {
            decode_image(image).map(Update::Image)
        }
        _ => Ok(Update::Other("agent_message_chunk".into())),
    }
}

fn attachment_name(text: &str) -> Option<&str> {
    let name = text
        .strip_prefix(ATTACHMENT_OPEN)?
        .strip_suffix(ATTACHMENT_CLOSE)?;
    (!name.is_empty()).then_some(name)
}

/// An image block that says it is a picture and carries its bytes; anything
/// less breaks the line, as a malformed frame does.
fn decode_image(image: &Value) -> Result<Image, String> {
    let mime = image
        .get("mimeType")
        .and_then(Value::as_str)
        .filter(|mime| mime.starts_with("image/"))
        .ok_or_else(|| {
            format!(
                "an image without a picture type: {:?}",
                image.get("mimeType")
            )
        })?;
    let data = image
        .get("data")
        .and_then(Value::as_str)
        .ok_or("an image without data")?;
    let bytes = STANDARD
        .decode(data)
        .map_err(|e| format!("an image whose data is not base64: {e}"))?;
    Ok(Image {
        mime: mime.to_owned(),
        bytes: bytes.into(),
    })
}

fn tool_status(status: Option<&Value>) -> Option<ToolStatus> {
    match status?.as_str()? {
        "pending" | "in_progress" => Some(ToolStatus::Running),
        "completed" => Some(ToolStatus::Completed),
        "failed" => Some(ToolStatus::Failed),
        _ => None,
    }
}
