//! The chat wire's events and framing: one JSON object per `\n`-terminated line over
//! `companion.sock`, protocol 3 (the vendored `contracts/companion/PROTOCOL.md`). Pure: it turns
//! lines into typed events and back, and does no I/O.
//!
//! Decoding is strict where a frame cannot mean anything (not JSON, not an object, no `type`, or a
//! known event whose own fields do not fit), and lenient inside a timeline row: a row's role and
//! kind are open words, a media ref is checked only when it is fetched, and one malformed
//! message, media ref, preview, block or place is skipped rather than costing the whole page.
//! A page counts what it skipped, for the caller to log.

use crate::realtime::protocol::LineBuffer;
pub use crate::realtime::protocol::{DecodeError, Direction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// The version this app speaks. Engine v0.17.0's daemon serves 3..4.
pub const PROTOCOL_VERSION: u64 = 3;
/// The one profile the wire serves.
pub const PROFILE: &str = "main";
/// The daemon's inbound line cap (`x-max-line-bytes`). Every line this client writes, newline
/// included, is shorter.
pub const MAX_LINE_BYTES: usize = 65_536;
/// The longest daemon line this client assembles (macOS `CompanionProtocol.swift`). A page is
/// kept to about 60 KiB, so this only stops a runaway.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// The most unscanned inbound bytes held at once.
pub const MAX_UNSCANNED_BYTES: usize = 32 * 1024 * 1024;

/// The server event types this build decodes; any other `type` is `ServerEvent::Unknown`.
const KNOWN_TYPES: [&str; 18] = [
    "server_hello",
    "accepted",
    "turn_started",
    "text_delta",
    "tool_event",
    "text_done",
    "turn_error",
    "turn_done",
    "row",
    "row_updated",
    "approval",
    "approval_resolved",
    "read_state",
    "history_page",
    "search_results",
    "media_path",
    "media_missing",
    "error",
];

/// A line buffer with this wire's caps.
pub fn line_buffer() -> LineBuffer {
    LineBuffer::with_caps(MAX_FRAME_BYTES, MAX_UNSCANNED_BYTES)
}

/// Where a history read starts: `after_seq` pages forward, `before_seq` backward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cursor {
    AfterSeq(u64),
    BeforeSeq(u64),
}

/// A GPT-Live task that outlived its call, by the three ids its row carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRef {
    pub call_uuid: String,
    pub task_id: String,
    pub revision: u64,
}

/// Everything this app may send. Build them with the constructors, which fill the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    ClientHello {
        protocol_version: u64,
    },
    Msg {
        client_msg_id: String,
        profile_id: String,
        text: String,
        /// Always empty: this wire refuses attachments.
        attach_ids: Vec<String>,
    },
    Command {
        client_msg_id: String,
        profile_id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<String>,
    },
    Cancel {
        profile_id: String,
        client_msg_id: String,
        /// Decoded for the contract; the app never sends one (a later step's feature).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_ref: Option<TaskRef>,
    },
    HistoryPull {
        profile_id: String,
        #[serde(flatten)]
        cursor: Cursor,
        limit: u16,
    },
    /// Decoded for the contract; the app does not search yet.
    HistorySearch {
        profile_id: String,
        query: String,
        limit: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_seq: Option<u64>,
    },
    ReadState {
        profile_id: String,
        read_up_to_seq: u64,
    },
    MediaFetch {
        profile_id: String,
        r#ref: String,
    },
    MediaRelease {
        profile_id: String,
        r#ref: String,
    },
}

/// A line that would reach the daemon's cap, which answers `line_too_large` and closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineTooLong(pub usize);

impl ClientEvent {
    pub fn hello() -> ClientEvent {
        ClientEvent::ClientHello {
            protocol_version: PROTOCOL_VERSION,
        }
    }

    pub fn msg(client_msg_id: &str, text: &str) -> ClientEvent {
        ClientEvent::Msg {
            client_msg_id: client_msg_id.to_owned(),
            profile_id: PROFILE.to_owned(),
            text: text.to_owned(),
            attach_ids: Vec::new(),
        }
    }

    pub fn command(client_msg_id: &str, name: &str, args: Option<&str>) -> ClientEvent {
        ClientEvent::Command {
            client_msg_id: client_msg_id.to_owned(),
            profile_id: PROFILE.to_owned(),
            name: name.to_owned(),
            args: args.map(str::to_owned),
        }
    }

    /// Stops the turn of the request `client_msg_id`.
    pub fn cancel(client_msg_id: &str) -> ClientEvent {
        ClientEvent::Cancel {
            profile_id: PROFILE.to_owned(),
            client_msg_id: client_msg_id.to_owned(),
            task_ref: None,
        }
    }

    pub fn pull(cursor: Cursor, limit: u16) -> ClientEvent {
        ClientEvent::HistoryPull {
            profile_id: PROFILE.to_owned(),
            cursor,
            limit,
        }
    }

    pub fn read_state(read_up_to_seq: u64) -> ClientEvent {
        ClientEvent::ReadState {
            profile_id: PROFILE.to_owned(),
            read_up_to_seq,
        }
    }

    pub fn media_fetch(media_ref: &str) -> ClientEvent {
        ClientEvent::MediaFetch {
            profile_id: PROFILE.to_owned(),
            r#ref: media_ref.to_owned(),
        }
    }

    pub fn media_release(media_ref: &str) -> ClientEvent {
        ClientEvent::MediaRelease {
            profile_id: PROFILE.to_owned(),
            r#ref: media_ref.to_owned(),
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
        min_version: u64,
        max_version: u64,
    },
    Accepted {
        client_msg_id: String,
        duplicate: bool,
        server_seq: Option<u64>,
    },
    TurnStarted {
        turn_id: String,
        in_reply_to: String,
    },
    TextDelta {
        turn_id: String,
        text: String,
    },
    ToolEvent {
        turn_id: String,
        tool: String,
        phase: ToolPhase,
        detail: Option<String>,
    },
    TextDone(Reply),
    TurnError {
        turn_id: String,
        code: String,
        message: String,
    },
    TurnDone {
        turn_id: String,
    },
    Row(Row),
    RowUpdated {
        server_seq: u64,
    },
    Approval(Approval),
    ApprovalResolved {
        approval_id: String,
        outcome: String,
    },
    ReadState {
        read_up_to_seq: u64,
    },
    HistoryPage(Page),
    /// Decoded so the contract holds; the app does not search yet.
    SearchResults {
        query: String,
    },
    MediaPath(Grant),
    MediaMissing {
        r#ref: String,
    },
    Error(ServerError),
    /// A type published after this build, by name. Log it; it is never fatal.
    #[serde(skip)]
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum ToolPhase {
    Start,
    Stop,
    Other(String),
}

impl From<String> for ToolPhase {
    fn from(word: String) -> ToolPhase {
        match word.as_str() {
            "start" => ToolPhase::Start,
            "stop" => ToolPhase::Stop,
            _ => ToolPhase::Other(word),
        }
    }
}

/// A reply's canonical text at its row, ending the draft of its turn.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawReply")]
pub struct Reply {
    pub turn_id: String,
    pub server_seq: u64,
    pub text: String,
    pub blocks: Vec<Block>,
    /// Blocks and places that did not decode or are of a type this build does not draw.
    pub skipped: usize,
}

#[derive(Deserialize)]
struct RawReply {
    turn_id: String,
    server_seq: u64,
    text: String,
    #[serde(default)]
    blocks: Vec<Value>,
}

impl From<RawReply> for Reply {
    fn from(raw: RawReply) -> Reply {
        let (blocks, skipped) = blocks(raw.blocks);
        Reply {
            turn_id: raw.turn_id,
            server_seq: raw.server_seq,
            text: raw.text,
            blocks,
            skipped,
        }
    }
}

/// One timeline row, from a live `row` (`text`) or a history message (`content`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawRow")]
pub struct Row {
    pub server_seq: u64,
    /// The daemon's own word: `user` and `assistant` today, any other kept as it came.
    pub role: String,
    #[serde(alias = "content")]
    pub text: String,
    pub ts: Option<String>,
    pub client_msg_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub kind: Option<String>,
    pub metadata: Option<Value>,
    pub media_refs: Vec<MediaRef>,
    pub link_previews: Vec<LinkPreview>,
    pub blocks: Vec<Block>,
    /// Media refs, previews, blocks and places that did not decode, for the caller to log.
    pub skipped: usize,
}

#[derive(Deserialize)]
struct RawRow {
    server_seq: u64,
    role: String,
    #[serde(alias = "content")]
    text: String,
    ts: Option<String>,
    client_msg_id: Option<String>,
    in_reply_to: Option<String>,
    kind: Option<String>,
    metadata: Option<Value>,
    #[serde(default)]
    media_refs: Vec<Value>,
    #[serde(default)]
    link_previews: Vec<Value>,
    #[serde(default)]
    blocks: Vec<Value>,
}

impl From<RawRow> for Row {
    fn from(raw: RawRow) -> Row {
        let (media_refs, media_skipped) = lenient(raw.media_refs);
        let (link_previews, previews_skipped) = lenient(raw.link_previews);
        let (blocks, blocks_skipped) = blocks(raw.blocks);
        Row {
            server_seq: raw.server_seq,
            role: raw.role,
            text: raw.text,
            ts: raw.ts,
            client_msg_id: raw.client_msg_id,
            in_reply_to: raw.in_reply_to,
            kind: raw.kind,
            metadata: raw.metadata,
            media_refs,
            link_previews,
            blocks,
            skipped: media_skipped + previews_skipped + blocks_skipped,
        }
    }
}

/// A file a row names. `ref` is kept as it came and checked only when fetched.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MediaRef {
    pub r#ref: String,
    pub kind: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub filename: Option<String>,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LinkPreview {
    /// Where the page answered from, after redirects.
    pub url: String,
    pub site: String,
    pub title: String,
    /// The address the row's text wrote, which a card pairs with.
    pub link: Option<String>,
    pub description: Option<String>,
    pub image_ref: Option<String>,
}

/// A record drawn under a reply. Unknown block types are skipped.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Places(Vec<Place>),
}

/// A place a search found and the reply links, cut to the fields the wire carries.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Place {
    pub name: String,
    pub location: Location,
    pub canonical_url: Option<String>,
    pub source_url: Option<String>,
    pub address: Option<String>,
    pub rating: Option<Rating>,
    pub price_range: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub opening_hours: Option<Hours>,
    pub distance: Option<Distance>,
}

/// A point on the globe. Coordinates off it would open a map of nowhere, so they do not decode,
/// and the place is skipped like any other that does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(try_from = "RawLocation")]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Deserialize)]
struct RawLocation {
    latitude: f64,
    longitude: f64,
}

impl TryFrom<RawLocation> for Location {
    type Error = String;

    /// Each coordinate inside its range, bounds included; NaN and infinity are in no range.
    fn try_from(raw: RawLocation) -> Result<Self, Self::Error> {
        let on_globe =
            (-90.0..=90.0).contains(&raw.latitude) && (-180.0..=180.0).contains(&raw.longitude);
        if !on_globe {
            return Err(format!(
                "{},{} is off the globe",
                raw.latitude, raw.longitude
            ));
        }
        Ok(Location {
            latitude: raw.latitude,
            longitude: raw.longitude,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Rating {
    pub value: Option<f64>,
    pub best: Option<f64>,
    /// A number in the schema, so a count written `321.0` reads too.
    pub count: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Hours {
    pub current_day: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Distance {
    pub value: f64,
    pub units: String,
}

/// An owner-approval card. Its token, and the two routes that contain it, are submitted and
/// never shown or logged, so `Debug` leaves them out.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Approval {
    pub approval_id: String,
    pub kind: String,
    pub text: String,
    pub detail: Option<String>,
    pub token: String,
    pub ttl_s: u64,
    pub approve_command: String,
    pub deny_command: String,
}

impl fmt::Debug for Approval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Approval")
            .field("approval_id", &self.approval_id)
            .field("kind", &self.kind)
            .field("text", &self.text)
            .field("ttl_s", &self.ttl_s)
            .finish_non_exhaustive()
    }
}

/// A history read's answer, oldest first. `skipped` counts messages that did not decode.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawPage")]
pub struct Page {
    pub messages: Vec<Row>,
    pub history_head_seq: u64,
    pub next_after_seq: Option<u64>,
    pub next_before_seq: Option<u64>,
    pub skipped: usize,
}

#[derive(Deserialize)]
struct RawPage {
    messages: Vec<Value>,
    history_head_seq: u64,
    next_after_seq: Option<u64>,
    next_before_seq: Option<u64>,
}

impl From<RawPage> for Page {
    fn from(raw: RawPage) -> Page {
        let (messages, skipped) = lenient(raw.messages);
        Page {
            messages,
            history_head_seq: raw.history_head_seq,
            next_after_seq: raw.next_after_seq,
            next_before_seq: raw.next_before_seq,
            skipped,
        }
    }
}

/// The daemon's grant of a managed file holding a ref's bytes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Grant {
    pub r#ref: String,
    pub path: String,
    pub kind: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// A refusal. `reason` is the daemon's own word.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerError {
    pub reason: String,
    pub message: Option<String>,
    pub field: Option<String>,
    pub event: Option<String>,
    pub client_msg_id: Option<String>,
    pub r#ref: Option<String>,
    pub direction: Option<Direction>,
    pub client_version: Option<u64>,
    pub min_version: Option<u64>,
    pub max_version: Option<u64>,
}

/// The elements of a list that decode, and how many did not.
fn lenient<T: DeserializeOwned>(values: Vec<Value>) -> (Vec<T>, usize) {
    let total = values.len();
    let decoded: Vec<T> = values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();
    let skipped = total - decoded.len();
    (decoded, skipped)
}

/// A reply's blocks of a known type with the places that decode, and how many blocks and places
/// were skipped.
fn blocks(values: Vec<Value>) -> (Vec<Block>, usize) {
    let total = values.len();
    let mut skipped = 0;
    let mut decoded = Vec::new();
    for value in values {
        let Some(places) = places(&value) else {
            continue;
        };
        let (places, dropped) = lenient(places.clone());
        skipped += dropped;
        decoded.push(Block::Places(places));
    }
    let unknown = total - decoded.len();
    (decoded, skipped + unknown)
}

/// The places of a `places` block; `None` for any other block.
fn places(value: &Value) -> Option<&Vec<Value>> {
    if value.get("type").and_then(Value::as_str) != Some("places") {
        return None;
    }
    value.get("places")?.as_array()
}

/// One line, without its `\n`, as an event. Every NUL in its strings is made U+FFFD here, the
/// one place the daemon's text enters.
pub fn decode_line(line: &[u8]) -> Result<ServerEvent, DecodeError> {
    let mut value: Value =
        serde_json::from_slice(line).map_err(|e| DecodeError::NotJson(e.to_string()))?;
    replace_nul(&mut value);
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

/// JSON may carry a NUL (`\u0000`); a GTK label cannot (GLib panics on one in a debug build and
/// cuts the text at it in a release one), so each becomes U+FFFD. Each value is looked at once.
fn replace_nul(value: &mut Value) {
    let mut waiting = vec![value];
    while let Some(value) = waiting.pop() {
        match value {
            Value::String(text) if text.contains('\0') => *text = text.replace('\0', "\u{FFFD}"),
            Value::Array(items) => waiting.extend(items.iter_mut()),
            Value::Object(fields) => waiting.extend(fields.values_mut()),
            _ => {}
        }
    }
}

/// Whether `text` is a media ref the daemon would accept in `media_fetch`: 64 lowercase hex.
pub fn is_media_ref(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
