//! The voice wire's events and framing: one JSON object per `\n`-terminated line over
//! `realtime.sock`, protocol 2 (the vendored `contracts/realtime/PROTOCOL.md`). Pure: it turns
//! lines into typed events and back, and does no I/O.
//!
//! Decoding is lenient where the contract is open: unknown fields are ignored, an unknown event
//! `type` becomes `ServerEvent::Unknown` (for the caller to log), and every open vocabulary keeps
//! a word it has never seen. It is strict where a frame cannot mean anything: not JSON, not an
//! object, no `type`, or a known type whose fields do not fit. Those end the connection.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// The version this app speaks. The daemon's window is 1..2 (fermix 0.11.0 and later).
pub const PROTOCOL_VERSION: u32 = 2;
/// The longest line this client assembles from the daemon (macOS `RealtimeProtocol.swift`).
pub const MAX_FRAME_BYTES: usize = 1_048_576;
/// The most unscanned inbound bytes held at once.
pub const MAX_UNSCANNED_BYTES: usize = 2_097_152;
/// The daemon's inbound line cap. Every line this client writes, newline included, is shorter.
pub const MAX_LINE_BYTES: usize = 65_536;
/// The daemon's default ceiling on one `audio_chunk`'s decoded bytes (`max_chunk_bytes`).
pub const MAX_CHUNK_BYTES: usize = 16_384;

/// The server event types this build decodes; any other `type` is `ServerEvent::Unknown`.
const KNOWN_TYPES: [&str; 12] = [
    "server_hello",
    "state",
    "audio_delta",
    "transcript_delta",
    "assistant_text_delta",
    "tool_event",
    "usage",
    "error",
    "playback_stop",
    "call_ready",
    "caption",
    "task",
];

/// Everything this app sends. Audio is base64 PCM16 LE mono 24 kHz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    ClientHello {
        protocol_version: u32,
    },
    CallStart,
    AudioChunk {
        audio: String,
    },
    /// Stop the reply; `audio_end_ms` is how much of it actually reached the speaker.
    Interrupt {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audio_end_ms: Option<u64>,
    },
    Mute {
        enabled: bool,
    },
    CallStop,
    /// Live only: the Realtime engine refuses it and closes the connection.
    TaskCancel {
        delegation_id: String,
    },
}

/// A line that would reach the daemon's cap, which answers `line_too_large` and closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineTooLong(pub usize);

impl ClientEvent {
    pub fn audio_chunk(pcm16: &[u8]) -> ClientEvent {
        ClientEvent::AudioChunk {
            audio: STANDARD.encode(pcm16),
        }
    }

    /// The event as it goes on the wire: one JSON object and a `\n`.
    pub fn line(&self) -> Result<String, LineTooLong> {
        let mut text = serde_json::to_string(self).expect("a client event always encodes");
        text.push('\n');
        if text.len() >= MAX_LINE_BYTES {
            return Err(LineTooLong(text.len()));
        }
        Ok(text)
    }
}

/// Everything the daemon sends. Field names are the wire's.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    ServerHello {
        min_version: u32,
        max_version: u32,
    },
    State {
        state: TurnState,
    },
    /// Decoded PCM16 LE mono 24 kHz.
    AudioDelta {
        #[serde(deserialize_with = "base64_bytes")]
        audio: Vec<u8>,
    },
    /// Realtime only: the whole user utterance once transcribed, with an unlisted `role`.
    TranscriptDelta {
        text: String,
        role: Option<String>,
    },
    /// Realtime only: deltas, then the full text again (spec R6). Not shown in v1.
    AssistantTextDelta {
        text: String,
    },
    ToolEvent {
        #[serde(default)]
        status: ToolStatus,
        name: Option<String>,
        reason: Option<String>,
    },
    Usage(Box<Usage>),
    /// Terminal for the call; the daemon closes after most of them.
    Error(ServerError),
    /// Flush the playback queue now.
    PlaybackStop,
    CallReady(CallReady),
    Caption(Caption),
    Task(Task),
    /// A type published after this build, by name. Log it; it is never fatal.
    #[serde(skip)]
    Unknown(String),
}

/// The daemon's turn state. The vocabulary is open: an unknown word is kept, and shown as idle.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum TurnState {
    Idle,
    Listening,
    Speaking,
    Muted,
    Thinking,
    Reconnecting,
    Other(String),
}

impl From<String> for TurnState {
    fn from(word: String) -> TurnState {
        match word.as_str() {
            "idle" => TurnState::Idle,
            "listening" => TurnState::Listening,
            "speaking" => TurnState::Speaking,
            "muted" => TurnState::Muted,
            "thinking" => TurnState::Thinking,
            "reconnecting" => TurnState::Reconnecting,
            _ => TurnState::Other(word),
        }
    }
}

/// A tool call's lifecycle. The daemon sends `running`; the schema's older `started` means the
/// same, and so does a missing status.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(from = "String")]
pub enum ToolStatus {
    #[default]
    Running,
    Completed,
    Error,
    Other(String),
}

impl From<String> for ToolStatus {
    fn from(word: String) -> ToolStatus {
        match word.as_str() {
            "running" | "started" => ToolStatus::Running,
            "completed" => ToolStatus::Completed,
            "error" => ToolStatus::Error,
            _ => ToolStatus::Other(word),
        }
    }
}

/// Cost facts, never a state change. Every field is optional: the Realtime and Live engines
/// send different shapes, and none of them is in the schema whole (spec §1.4).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Usage {
    /// `estimated`, `reported`, `limit_reached` (Realtime) or `live`.
    pub status: Option<String>,
    /// On `limit_reached`: which limit.
    pub reason: Option<String>,
    /// On `reported`: the call's reported total so far.
    pub cost_cents: Option<f64>,
    pub estimated: Option<UsageFigures>,
    pub reported: Option<UsageFigures>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub voice_seconds: Option<f64>,
    pub voice_cost_cents: Option<f64>,
    pub backend_turns: Option<u64>,
    /// The daemon's word; `unknown` is never zero.
    pub backend_cost: Option<String>,
    /// `complete`, `incomplete` or `running`.
    pub accounting: Option<String>,
}

/// The Realtime engine's `estimated` and `reported` maps.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct UsageFigures {
    pub input_audio_ms: Option<u64>,
    pub input_audio_tokens: Option<u64>,
    pub cost_cents: Option<f64>,
    pub transcription_ms: Option<u64>,
    pub transcription_cost_cents: Option<f64>,
}

/// A refusal. `reason` is the daemon's own word; Realtime errors carry no `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerError {
    pub reason: String,
    pub kind: Option<String>,
    /// The provider's own bounded sentence, where it gave one.
    pub detail: Option<String>,
    pub direction: Option<Direction>,
    pub client_version: Option<u32>,
    pub min_version: Option<u32>,
    pub max_version: Option<u32>,
    /// On `update_required`: the engine that needs the higher version.
    pub required_for: Option<String>,
}

/// Which side of the wire is out of date.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum Direction {
    /// Update this app.
    ClientTooOld,
    /// Update Fermix.
    ClientTooNew,
    Other(String),
}

impl From<String> for Direction {
    fn from(word: String) -> Direction {
        match word.as_str() {
            "client_too_old" => Direction::ClientTooOld,
            "client_too_new" => Direction::ClientTooNew,
            _ => Direction::Other(word),
        }
    }
}

/// Live only: the provider session is up. `captions` says whether caption frames will follow.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CallReady {
    pub engine: String,
    pub call_id: String,
    pub provider_session_id: Option<String>,
    /// Unix seconds.
    pub expires_at: Option<i64>,
    pub captions: bool,
}

/// Live only: one verbatim transcript fragment. Never trim or join it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Caption {
    pub speaker: Speaker,
    pub delta: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum Speaker {
    User,
    Assistant,
    Other(String),
}

impl From<String> for Speaker {
    fn from(word: String) -> Speaker {
        match word.as_str() {
            "user" => Speaker::User,
            "assistant" => Speaker::Assistant,
            _ => Speaker::Other(word),
        }
    }
}

/// Live only: one backend delegation. `revision` fences a re-asked task.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Task {
    pub delegation_id: String,
    pub revision: u64,
    pub status: TaskStatus,
    /// At most 240 characters.
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Other(String),
}

impl From<String> for TaskStatus {
    fn from(word: String) -> TaskStatus {
        match word.as_str() {
            "pending" => TaskStatus::Pending,
            "running" => TaskStatus::Running,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "cancelled" => TaskStatus::Cancelled,
            _ => TaskStatus::Other(word),
        }
    }
}

impl TaskStatus {
    /// Whether the work has stopped. A word this build cannot read is not: nothing may claim
    /// that unknown work finished.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

fn base64_bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    STANDARD
        .decode(text)
        .map_err(|e| serde::de::Error::custom(format!("audio is not base64: {e}")))
}

/// Why bytes from the daemon could not become an event. Each one ends the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    NotJson(String),
    NotAnObject,
    MissingType,
    /// A known type whose fields do not fit, in serde's words.
    BadEvent {
        kind: String,
        why: String,
    },
    FrameTooLarge(usize),
    BufferTooLarge(usize),
}

/// One line, without its `\n`, as an event.
pub fn decode_line(line: &[u8]) -> Result<ServerEvent, DecodeError> {
    let value: Value =
        serde_json::from_slice(line).map_err(|e| DecodeError::NotJson(e.to_string()))?;
    if !value.is_object() {
        return Err(DecodeError::NotAnObject);
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(DecodeError::MissingType)?
        .to_owned();
    if !KNOWN_TYPES.contains(&kind.as_str()) {
        return Ok(ServerEvent::Unknown(kind));
    }
    serde_json::from_value(value).map_err(|e| DecodeError::BadEvent {
        kind,
        why: e.to_string(),
    })
}

/// Splits the inbound byte stream into lines, holding at most one unfinished line.
#[derive(Debug, Default)]
pub struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    pub fn new() -> LineBuffer {
        LineBuffer::default()
    }

    /// Adds what was read and returns every line it completed, in order, without newlines and
    /// without empty lines. A burst past `MAX_UNSCANNED_BYTES` is refused before scanning; a
    /// line past `MAX_FRAME_BYTES` is refused, including one whose newline has not come yet.
    /// After an error the buffer is empty, and the connection is over.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, DecodeError> {
        let total = self.pending.len() + bytes.len();
        if total > MAX_UNSCANNED_BYTES {
            self.pending.clear();
            return Err(DecodeError::BufferTooLarge(total));
        }
        self.pending.extend_from_slice(bytes);
        let mut lines = Vec::new();
        let mut start = 0;
        while let Some(length) = self.pending[start..].iter().position(|&b| b == b'\n') {
            if length > MAX_FRAME_BYTES {
                self.pending.clear();
                return Err(DecodeError::FrameTooLarge(length));
            }
            if length > 0 {
                lines.push(self.pending[start..start + length].to_vec());
            }
            start += length + 1;
        }
        self.pending.drain(..start);
        let unfinished = self.pending.len();
        if unfinished > MAX_FRAME_BYTES {
            self.pending.clear();
            return Err(DecodeError::FrameTooLarge(unfinished));
        }
        Ok(lines)
    }
}
