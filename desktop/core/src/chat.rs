//! The Chat page's entries, read off the timeline the chat wire keeps (`chat_wire::timeline`):
//! the held rows, the questions still sending, the turn answering now with its tools, and why
//! the last request failed. Nothing here is decided: it only says how the held facts read.

use crate::activity::ToolKind;
use crate::chat_wire::protocol::{MediaRef, Row};
use crate::chat_wire::timeline::{self, Chat, ToolState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    User(String),
    /// A question the daemon has not accepted yet.
    Sending(String),
    /// Markdown, growing while it streams.
    Assistant(String),
    /// A file a row names, as one line ("Voice note", "Picture · lamp.jpg"), on the side of
    /// whoever wrote the row.
    Media {
        line: String,
        person: bool,
    },
    /// The running turn's tools, drawn as one group.
    Tools(Vec<ToolRun>),
    /// Why the last request did not get its reply.
    Failure(String),
}

/// One tool's run. Times are the caller's clock, in seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    pub id: String,
    /// The engine's name for the tool, as it came: "web_search".
    pub name: String,
    pub kind: ToolKind,
    pub state: RunState,
    pub started: i64,
    pub ended: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Running,
    Done,
    Failed,
    /// The person stopped the reply while it ran.
    Stopped,
    /// The reply ended, or failed, before the run said how it went.
    Unfinished,
}

/// The turn answering now, as the live line needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveTurn {
    pub started: i64,
    pub stopping: bool,
    /// Only a turn this connection started can be stopped.
    pub can_stop: bool,
}

/// The page's entries, each with the daemon's time for it (RFC 3339) where it has one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub entries: Vec<Entry>,
    pub times: Vec<Option<String>>,
    pub turn: Option<LiveTurn>,
    /// A turn runs and nothing on screen shows its work yet: the page shows its own live line.
    pub thinking: bool,
    /// The oldest seq held, which a page read backward moves.
    pub first_seq: Option<u64>,
}

impl View {
    fn push(&mut self, entry: Entry, time: Option<String>) {
        self.entries.push(entry);
        self.times.push(time);
    }
}

pub fn view(chat: &Chat) -> View {
    let mut view = View {
        first_seq: chat.rows.first().map(|held| held.row.server_seq),
        ..View::default()
    };
    for held in &chat.rows {
        push_row(&mut view, &held.row);
    }
    let held_ids: Vec<&str> = chat
        .rows
        .iter()
        .filter_map(|held| held.row.client_msg_id.as_deref())
        .collect();
    for out in &chat.pending {
        // An approval's route is a command, not a question: it draws no bubble.
        let question = out.approval_id.is_none();
        if question && !held_ids.contains(&out.client_msg_id.as_str()) {
            view.push(Entry::Sending(out.text.clone()), None);
        }
    }
    push_turn(&mut view, chat);
    if let Some(error) = &chat.last_error {
        view.push(Entry::Failure(error.clone()), None);
    }
    view
}

fn push_row(view: &mut View, row: &Row) {
    let text = row.text.trim();
    let time = row.ts.clone();
    if !text.is_empty() {
        let entry = match row.role.as_str() {
            "user" => Entry::User(text.to_owned()),
            _ => Entry::Assistant(row.text.clone()),
        };
        view.push(entry, time.clone());
    }
    let person = row.role == "user";
    for media in &row.media_refs {
        let line = media_line(media);
        view.push(Entry::Media { line, person }, time.clone());
    }
}

fn push_turn(view: &mut View, chat: &Chat) {
    let Some(turn) = &chat.turn else {
        return;
    };
    view.turn = Some(LiveTurn {
        started: turn.started,
        stopping: turn.stopping,
        can_stop: timeline::can_stop(chat),
    });
    if !turn.tools.is_empty() {
        let runs = turn.tools.iter().enumerate().map(|(index, run)| ToolRun {
            id: format!("{}-{index}", turn.turn_id),
            name: run.name.clone(),
            kind: ToolKind::Other,
            state: match run.state {
                ToolState::Running => RunState::Running,
                ToolState::Done => RunState::Done,
            },
            started: run.started,
            ended: run.ended,
        });
        view.push(Entry::Tools(runs.collect()), None);
    }
    if !turn.draft.is_empty() {
        view.push(Entry::Assistant(turn.draft.clone()), None);
    }
    view.thinking = turn.tools.is_empty() && turn.draft.is_empty();
}

/// What a file is, in the macOS app's words (`chat.kind.*`, `chat.voiceNote`), and its name.
pub fn media_line(media: &MediaRef) -> String {
    let kind = match (media.kind.as_str(), media.mime.as_str()) {
        (_, "audio/ogg") => "Voice note",
        ("image", _) => "Picture",
        ("video", _) => "Video",
        ("audio", _) => "Sound",
        (_, "application/pdf") => "PDF",
        _ => "File",
    };
    match &media.filename {
        Some(name) => format!("{kind} · {name}"),
        None => kind.to_owned(),
    }
}

/// The width to draw a `width` by `height` picture at so it fits a `bound`
/// pixel square, never larger than itself and never zero wide.
pub fn picture_width(width: i32, height: i32, bound: i32) -> i32 {
    assert!(
        width > 0 && height > 0 && bound > 0,
        "a picture that has no size: {width}x{height} in {bound}"
    );
    let for_height = i64::from(bound) * i64::from(width) / i64::from(height);
    let fitted = i64::from(width)
        .min(i64::from(bound))
        .min(for_height)
        .max(1);
    i32::try_from(fitted).expect("no wider than the picture itself")
}

/// The name a saved picture starts with: its type's usual extension, and none
/// when the type is not a plain word, since the agent chose it.
pub fn picture_file_name(mime: &str) -> String {
    let subtype = mime.strip_prefix("image/").unwrap_or_default();
    let extension = match subtype {
        "jpeg" => "jpg",
        "svg+xml" => "svg",
        other => other,
    };
    let plain = !extension.is_empty() && extension.chars().all(|c| c.is_ascii_alphanumeric());
    if plain {
        format!("Fermix image.{extension}")
    } else {
        "Fermix image".into()
    }
}
