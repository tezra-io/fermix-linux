//! The chat wire's events and framing: one JSON object per `\n`-terminated line over
//! `companion.sock`, protocol 7 (the vendored `contracts/companion/PROTOCOL.md`). Pure: it turns
//! lines into typed events and back, and does no I/O. Every event of the contract decodes, and
//! every client event encodes, whether or not the app shows or sends it yet.
//!
//! A frame that cannot mean anything (not JSON, not an object, no `type`) is refused. A known
//! event whose own fields do not fit is read as `ServerEvent::Unreadable`, by the ids it still
//! names: that event alone is dropped and the connection stays. Inside a timeline row decoding is
//! lenient: a row's role and kind are open words, a media ref is checked only when it is fetched,
//! and one malformed message, media ref, preview, block or place is skipped rather than costing
//! the whole page. A page counts what it skipped, for the caller to log.

use crate::realtime::protocol::LineBuffer;
pub use crate::realtime::protocol::{DecodeError, Direction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// The version this app speaks: 7, the files an owner attaches (PROTOCOL.md "Version 7"), with
/// 5's follow-ups and 6's turn status, lists and Always On notice. The engine's dev daemon serves
/// 6..7. There is no fallback: a daemon whose window ends below 7 is refused.
pub const PROTOCOL_VERSION: u64 = 7;
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
/// The longest id an unreadable event is kept by (`Unreadable`).
pub const MAX_ID_BYTES: usize = 256;
/// The longest id or code word a log line carries as it is (`loggable_id`): the schema's
/// `attach_id` bound.
pub const MAX_LOGGED_BYTES: usize = 64;
/// What a log line says in place of an id or a word it may not carry.
pub const NOT_LOGGED: &str = "(not logged)";

/// The server event types this build decodes; any other `type` is `ServerEvent::Unknown`.
const KNOWN_TYPES: [&str; 25] = [
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
    "proposal",
    "proposal_resolved",
    "error",
    "reasoning",
    "home_status_changed",
    "checklists",
    "checklist",
    "attach_status",
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
        /// Decoded for the contract; the app sends none (it has no home that moved).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        primary_epoch: Option<u64>,
    },
    Msg {
        client_msg_id: String,
        profile_id: String,
        text: String,
        /// The files the message carries, each answered `present` first. Always empty: the app
        /// attaches no file yet.
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
    /// The owner's answer to a proposal: its id and the word, never arguments (M60 cues §6
    /// rule 2).
    ProposalAnswer {
        client_msg_id: String,
        profile_id: String,
        proposal_id: String,
        decision: Decision,
    },
    /// Decoded for the contract, like the two below; the app reads no lists yet, and never asks
    /// for the index before it draws them: a reply then leaves a list's items to the card
    /// (PROTOCOL.md "Lists").
    ChecklistsPull {
        profile_id: String,
    },
    ChecklistPull {
        profile_id: String,
        list_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        removed: Option<bool>,
    },
    ChecklistEdit {
        client_msg_id: String,
        profile_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        list_id: Option<String>,
        op: ListOp,
    },
    /// Decoded for the contract; the app attaches no file yet.
    AttachFile {
        attach_id: String,
        kind: String,
        mime: String,
        size_bytes: u64,
        sha256: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

/// The owner's answer to a proposal. `AlwaysAllow` answers only a follow-up whose card offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Confirm,
    Deny,
    AlwaysAllow,
}

/// One change to a list (PROTOCOL.md "Lists", schema `checklistOp`), by row id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ListOp {
    Create {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
    },
    Add {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        section_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_id: Option<String>,
    },
    Check {
        row_id: String,
    },
    Uncheck {
        row_id: String,
    },
    /// `note: Some("")` clears the note.
    Edit {
        row_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Move {
        row_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        section_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_list_id: Option<String>,
    },
    Section {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_id: Option<String>,
    },
    Remove {
        row_id: String,
    },
    /// An empty `folder` takes the list out of its folder.
    Folder {
        folder: String,
    },
    Rename {
        text: String,
    },
    RemoveList,
    ClearDone,
    /// With no `row_id`, the removed list itself, under `title` when given.
    Restore {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
}

/// A line that would reach the daemon's cap, which answers `line_too_large` and closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineTooLong(pub usize);

impl ClientEvent {
    pub fn hello() -> ClientEvent {
        ClientEvent::ClientHello {
            protocol_version: PROTOCOL_VERSION,
            primary_epoch: None,
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

    pub fn proposal_answer(
        client_msg_id: &str,
        proposal_id: &str,
        decision: Decision,
    ) -> ClientEvent {
        ClientEvent::ProposalAnswer {
            client_msg_id: client_msg_id.to_owned(),
            profile_id: PROFILE.to_owned(),
            proposal_id: proposal_id.to_owned(),
            decision,
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
    /// `primary_epoch` and `role` are decoded and not acted on (PROTOCOL.md "Primary epoch"): the
    /// app has no home that moved. `commands` and `attachments` are offered later; what of them
    /// does not fit costs only itself, never the handshake.
    ServerHello {
        min_version: u64,
        max_version: u64,
        primary_epoch: Option<u64>,
        role: Option<String>,
        #[serde(default)]
        commands: Commands,
        #[serde(default)]
        attachments: Attachments,
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
        /// The engine's sentence for the step, on a start; never the tool's name.
        activity: Option<String>,
    },
    /// The model's summary of one reasoning step, live only.
    Reasoning {
        turn_id: String,
        text: String,
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
    /// A proposal waiting for the owner: sent when it is stored, and again after `server_hello`
    /// for each still waiting.
    Proposal {
        proposal: Box<Proposal>,
    },
    /// A proposal left `waiting`, whichever surface decided it.
    ProposalResolved(Resolution),
    Error(ServerError),
    /// Always On changed, for an app that shows it to read `home.status`. This one shows none
    /// while Always On stays behind its flag.
    HomeStatusChanged,
    /// The owner's lists, each by its counts.
    Checklists {
        lists: Vec<ListEntry>,
    },
    /// One list whole. `made` maps an edit's `client_msg_id` to the list or row it made.
    Checklist {
        list: Box<List>,
        #[serde(default)]
        made: BTreeMap<String, String>,
    },
    /// A file this connection handed over is in the store.
    AttachStatus {
        attach_id: String,
        status: String,
    },
    /// A type published after this build, by name. Log it; it is never fatal.
    #[serde(skip)]
    Unknown(String),
    /// A type this build knows whose fields do not fit. Only `read_line` makes one.
    #[serde(skip)]
    Unreadable(Unreadable),
}

impl ServerEvent {
    /// The event's type as the contract names it, for a log line: never the daemon's own text. A
    /// type this build does not know is "unknown".
    pub fn kind(&self) -> &'static str {
        match self {
            ServerEvent::ServerHello { .. } => "server_hello",
            ServerEvent::Accepted { .. } => "accepted",
            ServerEvent::TurnStarted { .. } => "turn_started",
            ServerEvent::TextDelta { .. } => "text_delta",
            ServerEvent::ToolEvent { .. } => "tool_event",
            ServerEvent::Reasoning { .. } => "reasoning",
            ServerEvent::TextDone(_) => "text_done",
            ServerEvent::TurnError { .. } => "turn_error",
            ServerEvent::TurnDone { .. } => "turn_done",
            ServerEvent::Row(_) => "row",
            ServerEvent::RowUpdated { .. } => "row_updated",
            ServerEvent::Approval(_) => "approval",
            ServerEvent::ApprovalResolved { .. } => "approval_resolved",
            ServerEvent::ReadState { .. } => "read_state",
            ServerEvent::HistoryPage(_) => "history_page",
            ServerEvent::SearchResults { .. } => "search_results",
            ServerEvent::MediaPath(_) => "media_path",
            ServerEvent::MediaMissing { .. } => "media_missing",
            ServerEvent::Proposal { .. } => "proposal",
            ServerEvent::ProposalResolved(_) => "proposal_resolved",
            ServerEvent::Error(_) => "error",
            ServerEvent::HomeStatusChanged => "home_status_changed",
            ServerEvent::Checklists { .. } => "checklists",
            ServerEvent::Checklist { .. } => "checklist",
            ServerEvent::AttachStatus { .. } => "attach_status",
            ServerEvent::Unreadable(unread) => known(&unread.kind),
            ServerEvent::Unknown(_) => "unknown",
        }
    }
}

/// `kind` as one of the contract's types, or "unknown".
fn known(kind: &str) -> &'static str {
    KNOWN_TYPES
        .into_iter()
        .find(|known| *known == kind)
        .unwrap_or("unknown")
}

/// An id or a code word the daemon sent, as a log line may carry it: as it is when it is 1 to
/// `MAX_LOGGED_BYTES` of the contract's id alphabet (ASCII letters, digits, `_` and `-`, the
/// schema's `attach_id`), and otherwise `NOT_LOGGED`, which holds none of its text.
pub fn loggable_id(id: &str) -> &str {
    let plain = !id.is_empty()
        && id.len() <= MAX_LOGGED_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if plain {
        id
    } else {
        NOT_LOGGED
    }
}

/// An event this build could not read: its type and the ids it still names, each where its one
/// field reads as a string no longer than `MAX_ID_BYTES`. Never its content: serde's reason can
/// quote it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unreadable {
    pub kind: String,
    /// The card a `proposal` or an `approval` is, or a resolution settles: its `proposal_id` or
    /// `approval_id`.
    pub card_id: Option<String>,
    /// The turn a `text_done`, a `turn_done` or a `turn_error` ends.
    pub turn_id: Option<String>,
    /// The request an `accepted` or an `error` answers.
    pub client_msg_id: Option<String>,
    /// The file a `media_path` or an `error` answers. A `media_missing` whose ref reads is read
    /// whole: it has no other field.
    pub r#ref: Option<String>,
}

/// The slash commands a hello offers, each that decodes, and how many did not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commands {
    pub offered: Vec<CommandDescriptor>,
    /// Commands that did not fit, or all of them when they are no list.
    pub skipped: usize,
}

impl<'de> Deserialize<'de> for Commands {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Value::Array(values) = Value::deserialize(deserializer)? else {
            return Ok(Commands {
                offered: Vec::new(),
                skipped: 1,
            });
        };
        let (offered, skipped) = lenient(values);
        Ok(Commands { offered, skipped })
    }
}

/// What a hello says of the inbox for the owner's files: nothing, the inbox, or an offer this
/// build cannot read, which offers nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Attachments {
    #[default]
    Absent,
    Offered(Inbox),
    Unreadable,
}

impl<'de> Deserialize<'de> for Attachments {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if value.is_null() {
            return Ok(Attachments::Absent);
        }
        Ok(serde_json::from_value(value).map_or(Attachments::Unreadable, Attachments::Offered))
    }
}

/// A slash command the engine takes, as `server_hello` names it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CommandDescriptor {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
}

/// Where the owner's connection hands the daemon a file, and the largest the store takes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Inbox {
    pub dir: String,
    pub max_bytes: u64,
}

/// A list in the owner's index.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListEntry {
    pub list_id: String,
    pub title: String,
    pub folder: Option<String>,
    pub open: u64,
    pub done: u64,
    pub rev: u64,
}

/// One list, its rows in order. A removed list carries when (`removed_at`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct List {
    pub list_id: String,
    pub title: String,
    pub rev: u64,
    pub rows: Vec<ListRow>,
    /// Rows removed in the last 30 days, which a restore brings back.
    #[serde(default)]
    pub removed: Vec<RemovedRow>,
    pub removed_at: Option<String>,
}

/// An item or a section heading (`kind`, the daemon's word).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListRow {
    pub row_id: String,
    pub kind: String,
    pub text: String,
    pub note: Option<String>,
    pub done: Option<bool>,
    pub done_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemovedRow {
    #[serde(flatten)]
    pub row: ListRow,
    pub removed_at: String,
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
    /// The time its row stores, as the row and its history message give it; an older daemon
    /// sends none.
    pub ts: Option<String>,
    pub blocks: Vec<Block>,
    /// Blocks and places that did not decode or are of a type this build does not draw.
    pub skipped: usize,
}

#[derive(Deserialize)]
struct RawReply {
    turn_id: String,
    server_seq: u64,
    text: String,
    ts: Option<String>,
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
            ts: raw.ts,
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

/// Where a proposal stands, in the daemon's word. A word this build does not know is kept and
/// draws no button.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum ProposalState {
    Waiting,
    Confirmed,
    Denied,
    Expired,
    Refused,
    Unknown(String),
}

impl From<String> for ProposalState {
    fn from(word: String) -> ProposalState {
        match word.as_str() {
            "waiting" => ProposalState::Waiting,
            "confirmed" => ProposalState::Confirmed,
            "denied" => ProposalState::Denied,
            "expired" => ProposalState::Expired,
            "refused" => ProposalState::Refused,
            _ => ProposalState::Unknown(word),
        }
    }
}

/// A proposal's card: Fermix's record and `summary`, which carry no outside text, and
/// `outside`, everything a model or a stranger wrote. A field that does not apply is absent on
/// this wire and null on management; both decode as `None`. Times are RFC 3339 UTC, kept as
/// they came. A card that is neither a mandate's call with its arguments nor a watcher's
/// follow-up, or is both, does not decode, and neither does a call whose arguments are blank or
/// a follow-up that asks anything but `FOLLOW_UP_RUNS`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawProposal")]
pub struct Proposal {
    pub proposal_id: String,
    pub state: ProposalState,
    pub ask: Ask,
    pub task: ProposalTask,
    pub watcher: Option<String>,
    pub source_ref: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub received_at: Option<String>,
    pub decided_at: Option<String>,
    /// The daemon's word for the surface that decided it.
    pub decided_from: Option<String>,
    /// The daemon's word for how a confirmed call ended.
    pub result_status: Option<String>,
    pub status_sentence: String,
    pub summary: String,
    pub outside: Outside,
}

/// What a proposal asks the owner to let run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// A mandate's recorded call, run exactly as recorded once confirmed. `args_text` is its
    /// arguments as pretty JSON, outside text, and never blank.
    Call {
        mandate: String,
        tool: String,
        args_text: String,
    },
    /// A watcher's follow-up, a task Fermix starts once confirmed (PROTOCOL.md "Version 5").
    FollowUp(FollowUp),
}

impl Ask {
    /// Whether a card asking this takes `decision`. Confirm and deny answer every card;
    /// `always_allow` only a follow-up whose card offers it, since a call's card refuses it and
    /// a daemon that does not offer it may not take it (PROTOCOL.md "Version 5").
    pub fn takes(&self, decision: Decision) -> bool {
        match (self, decision) {
            (_, Decision::Confirm | Decision::Deny) => true,
            (Ask::FollowUp(follow_up), Decision::AlwaysAllow) => follow_up.offers_always_allow,
            (Ask::Call { .. }, Decision::AlwaysAllow) => false,
        }
    }
}

/// The one `follow_up.asks` this build reads (PROTOCOL.md "Proposals"): run the follow-up once,
/// now: the one question its card's answers (Skip, Run, Always allow) answer.
pub const FOLLOW_UP_RUNS: &str = "run";

/// A follow-up as its card names it: the owner's own words for it, and the tools it may use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FollowUp {
    /// The daemon's word for what is asked. `FOLLOW_UP_RUNS` is the one this build reads.
    pub asks: String,
    /// Whether the card takes `always_allow`.
    #[serde(default)]
    pub offers_always_allow: bool,
    pub name: String,
    pub instructions: String,
    pub tools: Vec<String>,
    pub reports_to: String,
}

#[derive(Deserialize)]
struct RawProposal {
    proposal_id: String,
    state: ProposalState,
    mandate: Option<String>,
    tool: Option<String>,
    follow_up: Option<FollowUp>,
    task: ProposalTask,
    watcher: Option<String>,
    source_ref: Option<String>,
    created_at: String,
    expires_at: String,
    received_at: Option<String>,
    decided_at: Option<String>,
    decided_from: Option<String>,
    result_status: Option<String>,
    status_sentence: String,
    summary: String,
    outside: RawOutside,
}

#[derive(Deserialize)]
struct RawOutside {
    args_text: Option<String>,
    #[serde(default)]
    args: Vec<Arg>,
    reason: Option<String>,
    cue: Option<Cue>,
}

impl TryFrom<RawProposal> for Proposal {
    type Error = String;

    /// The schema's `oneOf`: a call with `mandate`, `tool` and `outside.args_text`, or a
    /// `follow_up`, never both. A call whose arguments are blank is refused too: its card would
    /// ask the owner to confirm a call it shows nothing of. So is a follow-up that asks anything
    /// but `FOLLOW_UP_RUNS`: its card's answers would mean what it does not ask.
    fn try_from(raw: RawProposal) -> Result<Self, Self::Error> {
        let RawOutside {
            args_text,
            args,
            reason,
            cue,
        } = raw.outside;
        let args_text = args_text.filter(|text| !text.trim().is_empty());
        let ask = match (raw.mandate, raw.tool, args_text, raw.follow_up) {
            (Some(mandate), Some(tool), Some(args_text), None) => Ask::Call {
                mandate,
                tool,
                args_text,
            },
            (None, None, _, Some(follow_up)) if follow_up.asks == FOLLOW_UP_RUNS => {
                Ask::FollowUp(follow_up)
            }
            (None, None, _, Some(_)) => return Err("a follow-up that asks no run".into()),
            _ => return Err("neither a mandate's call nor a follow-up".into()),
        };
        Ok(Proposal {
            proposal_id: raw.proposal_id,
            state: raw.state,
            ask,
            task: raw.task,
            watcher: raw.watcher,
            source_ref: raw.source_ref,
            created_at: raw.created_at,
            expires_at: raw.expires_at,
            received_at: raw.received_at,
            decided_at: raw.decided_at,
            decided_from: raw.decided_from,
            result_status: raw.result_status,
            status_sentence: raw.status_sentence,
            summary: raw.summary,
            outside: Outside { args, reason, cue },
        })
    }
}

/// The task whose cue run raised a proposal. One with no name is named by its job.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProposalTask {
    pub job_id: String,
    pub name: Option<String>,
}

/// Outside text, kept apart by structure: a call's arguments one by one (`args`; as pretty JSON
/// they are its `Ask::Call`'s), the filter's reason for passing the item, and the cue's snapshot,
/// absent when Fermix no longer keeps it. A follow-up's card carries no arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outside {
    pub args: Vec<Arg>,
    pub reason: Option<String>,
    pub cue: Option<Cue>,
}

/// One argument of a recorded call, outside text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Arg {
    pub name: String,
    pub value: String,
}

/// The item a proposal acted on. Every field is outside text, `occurred_at` too: it is the
/// producer's claim.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Cue {
    pub r#type: Option<String>,
    pub subject: Option<String>,
    pub text: Option<String>,
    pub occurred_at: Option<String>,
}

/// A proposal's end. `sentence` is Fermix's; `result_text` is outside text, what the tool or the
/// validator answered.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Resolution {
    pub proposal_id: String,
    pub state: ProposalState,
    pub result_status: Option<String>,
    pub sentence: String,
    pub result_text: Option<String>,
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
    /// The file an `attach_file` named, on its refusal.
    pub attach_id: Option<String>,
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

/// One line, without its `\n`, as an event. A known event whose fields do not fit is
/// `Unreadable`, so it costs only itself. Its reason is dropped here on purpose: serde's words can
/// quote the line, and the caller logs the event's type instead.
pub fn read_line(line: &[u8]) -> Result<ServerEvent, DecodeError> {
    let (value, kind) = framed(line)?;
    if !KNOWN_TYPES.contains(&kind.as_str()) {
        return Ok(ServerEvent::Unknown(kind));
    }
    let unread = unreadable(kind, &value);
    Ok(serde_json::from_value(value).unwrap_or(ServerEvent::Unreadable(unread)))
}

/// A line as a JSON object and its `type`. Every NUL in its strings is made U+FFFD here, the one
/// place the daemon's text enters.
fn framed(line: &[u8]) -> Result<(Value, String), DecodeError> {
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
    Ok((value, kind))
}

/// What an event of `kind` that could not be read still names.
fn unreadable(kind: String, value: &Value) -> Unreadable {
    let card_id = match kind.as_str() {
        "proposal" => id_at(value, "/proposal/proposal_id"),
        "proposal_resolved" => id_at(value, "/proposal_id"),
        "approval" | "approval_resolved" => id_at(value, "/approval_id"),
        _ => None,
    };
    let ends_turn = matches!(kind.as_str(), "text_done" | "turn_done" | "turn_error");
    let answers_request = matches!(kind.as_str(), "accepted" | "error");
    let answers_file = matches!(kind.as_str(), "media_path" | "error");
    Unreadable {
        card_id,
        turn_id: ends_turn.then(|| id_at(value, "/turn_id")).flatten(),
        client_msg_id: answers_request
            .then(|| id_at(value, "/client_msg_id"))
            .flatten(),
        r#ref: answers_file.then(|| id_at(value, "/ref")).flatten(),
        kind,
    }
}

/// The string at `pointer`, where it is one no longer than `MAX_ID_BYTES`.
fn id_at(value: &Value, pointer: &str) -> Option<String> {
    let id = value.pointer(pointer)?.as_str()?;
    (id.len() <= MAX_ID_BYTES).then(|| id.to_owned())
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
