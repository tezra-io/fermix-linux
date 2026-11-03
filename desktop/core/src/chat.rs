//! The Chat page's entries, read off the timeline the chat wire keeps (`chat_wire::timeline`):
//! the held rows, the proposal cards placed among them by when each was made, the placeholder of
//! the cards that could not be read, the questions still sending, the turn answering now with its
//! tools, and why the last request failed.
//! Nothing here is decided: it only says how the held facts read.

use crate::activity::ToolKind;
use crate::chat_wire::proposals;
use crate::chat_wire::protocol::Row;
use crate::chat_wire::timeline::{self, Chat, HeldProposal, HeldRow, ToolState};
use crate::chat_wire::unshown::{self, Asks, Unshown};
use crate::media::{strip, words, Strip};
use crate::phone::unix_seconds;
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
    /// A mandate's proposal, drawn apart from the messages: it answers none of them.
    Proposal(proposals::Card),
    /// One placeholder for every proposal the daemon sent that could not be read
    /// (`chat_wire::unshown`).
    Unshown,
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
    /// A proposal's card, by its id.
    Proposal(String),
    Unshown,
    Failure,
}

/// The page's entries, each with its key, the daemon's time for it (RFC 3339) where it has
/// one this app can read, and the held row it draws.
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
    let mut placed = placed_cards(&chat.proposals).into_iter().peekable();
    for held in &chat.rows {
        if let Some(at) = row_time(held) {
            while let Some(card) = placed.next_if(|card| card.created.is_some_and(|made| made < at))
            {
                push_card(&mut view, card);
            }
        }
        let reply = held.turn_id.as_deref().map(|turn| {
            let count = replies.entry(turn).or_default();
            *count += 1;
            Key::Reply(turn.to_owned(), *count - 1)
        });
        push_row(&mut view, &held.row, reply);
    }
    placed.for_each(|card| push_card(&mut view, card));
    if unshown::waiting(&chat.unshown, Asks::Proposal) {
        view.push(Entry::Unshown, Key::Unshown, None, None);
    }
    let held_ids: Vec<&str> = chat
        .rows
        .iter()
        .filter_map(|held| held.row.client_msg_id.as_deref())
        .collect();
    for out in &chat.pending {
        // A card's answer is not a question: it draws no bubble.
        if out.is_question() && !held_ids.contains(&out.client_msg_id.as_str()) {
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

/// What a request to bring `proposal_id`'s card into view scrolls to: its card where it is held,
/// or the placeholder where that card could not be read. `None` while neither is drawn: the request
/// waits for the render that draws one.
pub fn reveal_key(chat: &Chat, proposal_id: &str) -> Option<Key> {
    let held = chat
        .proposals
        .iter()
        .any(|held| held.proposal.proposal_id == proposal_id);
    if held {
        return Some(Key::Proposal(proposal_id.to_owned()));
    }
    let unshown = Unshown {
        asks: Asks::Proposal,
        id: Some(proposal_id.to_owned()),
    };
    chat.unshown.contains(&unshown).then_some(Key::Unshown)
}

/// The proposal cards in the order they stand in the conversation: by when each was made, a card
/// whose time cannot be read last, in the order the cards came.
fn placed_cards(proposals: &[HeldProposal]) -> Vec<&HeldProposal> {
    let mut placed: Vec<&HeldProposal> = proposals.iter().collect();
    placed.sort_by_key(|held| (held.created.is_none(), held.created));
    placed
}

/// When a held row was written: the daemon's time, or when a streamed reply arrived here.
fn row_time(held: &HeldRow) -> Option<i64> {
    held.row
        .ts
        .as_deref()
        .and_then(unix_seconds)
        .or(held.arrived)
}

/// A proposal's card, with no time of its own under it: its facts say when it waits until.
fn push_card(view: &mut View, held: &HeldProposal) {
    let card = proposals::card(held);
    let key = Key::Proposal(card.id.clone());
    view.push(Entry::Proposal(card), key, None, None);
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
    // A time this app cannot read is drawn as none, so the page never tries it at each draw.
    let time = row.ts.clone().filter(|ts| unix_seconds(ts).is_some());
    view.push(entry, key, time, Some(row.server_seq));
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

/// A time on the reader's clock, in their zone: the year, the month 1 to 12, the day of the month,
/// the hour and the minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A daemon's time as Chat writes it, beside its rows and on its proposal cards alike (the Mac's
/// `CueTimes`): "14:05" on the reader's day, "Oct 28, 14:05" on another day of their year, and
/// "Oct 28, 2025, 14:05" in another year. `now` is the reader's clock.
pub fn written_time(at: LocalTime, now: LocalTime) -> String {
    let month = usize::try_from(at.month - 1)
        .ok()
        .and_then(|index| MONTHS.get(index))
        .unwrap_or_else(|| panic!("a month is 1 to 12, not {}", at.month));
    let clock = format!("{:02}:{:02}", at.hour, at.minute);
    if (at.year, at.month, at.day) == (now.year, now.month, now.day) {
        return clock;
    }
    if at.year == now.year {
        return format!("{month} {}, {clock}", at.day);
    }
    format!("{month} {}, {}, {clock}", at.day, at.year)
}
