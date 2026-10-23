//! The Chat page's entries, read off the timeline the chat wire keeps (`chat_wire::timeline`):
//! the held rows, the questions still sending, the turn answering now with its tools, and why
//! the last request failed. Nothing here is decided: it only says how the held facts read.

use crate::activity::ToolKind;
use crate::chat_wire::protocol::Row;
use crate::chat_wire::timeline::{self, Chat, ToolState};
use crate::media::{strip, words, Strip};
use crate::places::{cards, Card};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// The person's words, and what their row shows under them.
    User { text: String, media: Strip },
    /// A question the daemon has not accepted yet.
    Sending(String),
    /// Markdown, growing while it streams, the cards of the places it links, and what its row
    /// shows under it, which come with its row (none while it streams).
    Assistant {
        text: String,
        places: Vec<Card>,
        media: Strip,
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

/// What an entry draws, which stays the same while the entry grows or is read again: the page
/// keeps each entry's widget by it, so rows arriving above or below leave the rest drawn.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// A held row's words and what it shows under them, by seq.
    Row(u64),
    /// A reply a turn streams, by the turn and its place among the turn's replies (a turn may
    /// send more than one): its draft, and then the held row it became.
    Reply(String, usize),
    /// A question still sending, by its `client_msg_id`.
    Sending(String),
    /// The running turn's tools, by the turn.
    Tools(String),
    Failure,
}

/// The page's entries, each with its key, the daemon's time for it (RFC 3339) where it has
/// one, and the held row it draws.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub entries: Vec<Entry>,
    pub keys: Vec<Key>,
    pub times: Vec<Option<String>>,
    pub seqs: Vec<Option<u64>>,
    pub turn: Option<LiveTurn>,
    /// A turn runs and nothing on screen shows its work yet: the page shows its own live line.
    pub thinking: bool,
    /// The oldest seq held, which a page read backward moves.
    pub first_seq: Option<u64>,
}

impl View {
    fn push(&mut self, entry: Entry, key: Key, time: Option<String>, seq: Option<u64>) {
        self.entries.push(entry);
        self.keys.push(key);
        self.times.push(time);
        self.seqs.push(seq);
    }
}

pub fn view(chat: &Chat) -> View {
    let mut view = View {
        first_seq: chat.rows.first().map(|held| held.row.server_seq),
        ..View::default()
    };
    // A turn's replies are numbered in the order their rows are held; its draft takes the next
    // number, which its row will have.
    let mut replies: HashMap<&str, usize> = HashMap::new();
    for held in &chat.rows {
        let reply = held.turn_id.as_deref().map(|turn| {
            let count = replies.entry(turn).or_default();
            *count += 1;
            Key::Reply(turn.to_owned(), *count - 1)
        });
        push_row(&mut view, &held.row, reply);
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
            let key = Key::Sending(out.client_msg_id.clone());
            view.push(Entry::Sending(out.text.clone()), key, None, None);
        }
    }
    let drafted = chat
        .turn
        .as_ref()
        .and_then(|turn| replies.get(turn.turn_id.as_str()));
    push_turn(&mut view, chat, drafted.copied().unwrap_or(0));
    if let Some(error) = &chat.last_error {
        view.push(Entry::Failure(error.clone()), Key::Failure, None, None);
    }
    view
}

/// A held row: its words and what it shows under them, as one entry, or none where it has
/// neither. `reply` is its key as a reply this connection streamed; any other row is keyed by its
/// seq.
fn push_row(view: &mut View, row: &Row, reply: Option<Key>) {
    let media = strip(row);
    let text = words(row, &media);
    if text.trim().is_empty() && media.is_empty() {
        return;
    }
    let entry = match row.role.as_str() {
        "user" => Entry::User {
            text: text.trim().to_owned(),
            media,
        },
        _ => Entry::Assistant {
            text,
            places: cards(&row.blocks),
            media,
        },
    };
    let key = reply.unwrap_or(Key::Row(row.server_seq));
    view.push(entry, key, row.ts.clone(), Some(row.server_seq));
}

/// The running turn: its tools, then its draft, the turn's reply after the `replied` it holds.
fn push_turn(view: &mut View, chat: &Chat, replied: usize) {
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
        let key = Key::Tools(turn.turn_id.clone());
        view.push(Entry::Tools(runs.collect()), key, None, None);
    }
    if !turn.draft.is_empty() {
        let key = Key::Reply(turn.turn_id.clone(), replied);
        let draft = Entry::Assistant {
            text: turn.draft.clone(),
            places: Vec::new(),
            media: Strip::default(),
        };
        view.push(draft, key, None, None);
    }
    view.thinking = turn.tools.is_empty() && turn.draft.is_empty();
}
