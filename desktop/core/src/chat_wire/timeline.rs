//! What the chat holds and does with each server event and each request, as values: no socket,
//! no clock, no GTK (a port of macOS `CompanionReducer`). Every step changes the `Chat` and
//! returns the events to send in answer.
//!
//! The rules are the vendored PROTOCOL.md's "Keeping a client's timeline": rows are held by
//! `server_seq` behind a cursor, a live row is shown only when it is the next seq, a gap is
//! closed by a read from the cursor, and at most one read is out at a time. Requests wait in the
//! outbox until the daemon accepts them, and the outbox is sent again after every handshake.

use crate::chat_wire::protocol::{
    Approval, ClientEvent, Cursor, Page, Reply, Row, ServerError, ServerEvent, ToolPhase,
};
use std::collections::BTreeSet;

/// Rows a history read asks for.
pub const PAGE_LIMIT: u16 = 50;
/// The most rows one read of held rows spans (the wire's `limit` maximum).
pub const MAX_READ: u16 = 200;
/// Requests waiting for `accepted`; a send past it is refused with a sentence.
pub const MAX_PENDING: usize = 64;
/// Rows held while live rows arrive; past it the oldest is forgotten and read again on demand.
pub const MAX_HELD: usize = 1000;
/// The `before_seq` a cold start reads below: one past every row, so the newest page comes.
pub const NEWEST: u64 = u64::MAX;

// The macOS app's ProductStrings wording (`companion.error.*`).
const STOPPED: &str = "The reply was stopped";
const INTERRUPTED: &str = "The reply was interrupted";
const TOO_LONG: &str = "The message is too long to send";
/// No macOS string: its outbox has no cap.
const OUTBOX_FULL: &str = "Too many messages are waiting to send";

/// One held row. A reply held from its turn's `text_done` names the turn, and has no time.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldRow {
    pub row: Row,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Done,
}

/// One tool call of the live turn, by the engine's name for the tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    pub name: String,
    pub state: ToolState,
    /// On the caller's clock, seconds.
    pub started: i64,
    pub ended: Option<i64>,
}

/// The turn answering now. Its tools are live only: the group goes when the turn ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub turn_id: String,
    /// The request it answers; absent for a turn joined midway.
    pub in_reply_to: Option<String>,
    /// Every `text_delta`, appended exactly as sent.
    pub draft: String,
    pub tools: Vec<ToolRun>,
    /// When this client first heard of it, on the caller's clock.
    pub started: i64,
    /// Stop was asked for; the turn has not ended yet.
    pub stopping: bool,
}

/// A request the daemon has not accepted yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub client_msg_id: String,
    pub event: ClientEvent,
    /// What the person typed, for the bubble shown as sending; empty for an approval's route.
    pub text: String,
    /// The card an approval's route answers.
    pub approval_id: Option<String>,
}

/// An approval card and the second (on the caller's clock) its own ttl runs out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldApproval {
    pub approval: Approval,
    pub expires_at: i64,
}

/// The one history read out.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pull {
    /// A page read forward (the catch-up) or backward (a cold start, or the reader at the top).
    Page(Cursor),
    /// Held rows read again, and which.
    Rows(BTreeSet<u64>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chat {
    /// Ascending by seq, each seq once.
    pub rows: Vec<HeldRow>,
    /// The newest seq held, 0 before any.
    pub cursor: u64,
    /// The newest seq the daemon had when it read the latest page.
    pub head: u64,
    pub has_older: bool,
    /// The read frontier, as the daemon last sent it.
    pub read_up_to: u64,
    pub turn: Option<Turn>,
    /// Oldest first.
    pub pending: Vec<Outgoing>,
    pub approvals: Vec<HeldApproval>,
    /// The latest failure, in a sentence, until the next send.
    pub last_error: Option<String>,
    connected: bool,
    pull: Option<Pull>,
    older_waiting: Option<u64>,
    rows_to_read: BTreeSet<u64>,
    read_before_disconnect: BTreeSet<u64>,
    read_sent: u64,
}

/// A handshake completed: the catch-up read first, then every request not yet accepted, in
/// order. Held rows on screen are read again, since notices sent while away are lost.
pub fn negotiated(chat: &mut Chat, on_screen: &BTreeSet<u64>) -> Vec<ClientEvent> {
    chat.connected = true;
    chat.approvals.clear();
    let cursor = if chat.rows.is_empty() {
        Cursor::BeforeSeq(NEWEST)
    } else {
        Cursor::AfterSeq(chat.cursor)
    };
    read_again(chat, on_screen);
    let mut sent = read_page(chat, cursor);
    sent.extend(chat.pending.iter().map(|out| out.event.clone()));
    sent
}

/// The connection ended. Nothing asked on it will be answered and a turn's end is live-only,
/// so the read and the turn go; every held row is marked to be read again on screen.
pub fn disconnected(chat: &mut Chat) {
    if let Some(Pull::Rows(seqs)) = chat.pull.take() {
        chat.rows_to_read.extend(seqs);
    }
    chat.connected = false;
    chat.read_before_disconnect = chat.rows.iter().map(|held| held.row.server_seq).collect();
    chat.older_waiting = None;
    chat.turn = None;
}

/// A message to the agent. It enters the outbox now, and is sent now when connected.
pub fn send(chat: &mut Chat, text: &str, client_msg_id: String) -> Vec<ClientEvent> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if chat.pending.len() >= MAX_PENDING {
        chat.last_error = Some(OUTBOX_FULL.into());
        return Vec::new();
    }
    chat.last_error = None;
    let event = ClientEvent::msg(&client_msg_id, text);
    enqueue(chat, client_msg_id, event, text.to_owned(), None)
}

fn enqueue(
    chat: &mut Chat,
    client_msg_id: String,
    event: ClientEvent,
    text: String,
    approval_id: Option<String>,
) -> Vec<ClientEvent> {
    // The outbox sends what it holds after every handshake: a line past the daemon's cap
    // would be refused and resent forever.
    if event.line().is_err() {
        chat.last_error = Some(TOO_LONG.into());
        return Vec::new();
    }
    chat.pending.push(Outgoing {
        client_msg_id,
        event: event.clone(),
        text,
        approval_id,
    });
    if chat.connected {
        vec![event]
    } else {
        Vec::new()
    }
}

/// Whether the running turn can be stopped: only a turn this connection started names the
/// request it answers.
pub fn can_stop(chat: &Chat) -> bool {
    chat.turn
        .as_ref()
        .is_some_and(|turn| turn.in_reply_to.is_some())
}

/// Stops the running turn by the request it answers. Never a GPT-Live task.
pub fn stop(chat: &mut Chat) -> Vec<ClientEvent> {
    let request = chat.turn.as_ref().and_then(|turn| turn.in_reply_to.clone());
    let Some(id) = request.filter(|_| chat.connected) else {
        return Vec::new();
    };
    if let Some(turn) = chat.turn.as_mut() {
        turn.stopping = true;
    }
    vec![ClientEvent::cancel(&id)]
}

/// `/soul apply TOKEN` as the command `soul` with the arguments `apply TOKEN`.
pub fn route(command: &str) -> Option<(String, Option<String>)> {
    let line = command.strip_prefix('/')?;
    let (name, args) = match line.split_once(' ') {
        Some((name, args)) => (name, Some(args.trim().to_owned())),
        None => (line, None),
    };
    if name.is_empty() {
        return None;
    }
    Some((name.to_owned(), args.filter(|args| !args.is_empty())))
}

/// Whether a card's answer is on its way, so its buttons wait.
pub fn answering(chat: &Chat, approval_id: &str) -> bool {
    chat.pending
        .iter()
        .any(|out| out.approval_id.as_deref() == Some(approval_id))
}

/// Answers a card with one of its routes, sent as a `command`, once.
pub fn answer(
    chat: &mut Chat,
    approval_id: &str,
    approve: bool,
    client_msg_id: String,
) -> Vec<ClientEvent> {
    if answering(chat, approval_id) {
        return Vec::new();
    }
    let Some(held) = chat
        .approvals
        .iter()
        .find(|held| held.approval.approval_id == approval_id)
    else {
        return Vec::new();
    };
    let command = if approve {
        &held.approval.approve_command
    } else {
        &held.approval.deny_command
    };
    let Some((name, args)) = route(command) else {
        return Vec::new();
    };
    let event = ClientEvent::command(&client_msg_id, &name, args.as_deref());
    enqueue(
        chat,
        client_msg_id,
        event,
        String::new(),
        Some(approval_id.to_owned()),
    )
}

/// Drops the cards whose own ttl ran out: a restarted daemon forgets them unresolved.
pub fn expire_approvals(chat: &mut Chat, now: i64) {
    chat.approvals.retain(|held| held.expires_at > now);
}

/// The reader reached the top: the page before the oldest held row, now or after the read out.
pub fn pull_older(chat: &mut Chat) -> Vec<ClientEvent> {
    let Some(oldest) = chat.rows.first().map(|held| held.row.server_seq) else {
        return Vec::new();
    };
    if !chat.has_older {
        return Vec::new();
    }
    let cursor = Cursor::BeforeSeq(oldest);
    // A page landing after a reconnect could move a reader who has scrolled on, and the read
    // out may be this very page: neither is asked again.
    if !chat.connected || chat.pull == Some(Pull::Page(cursor)) {
        return Vec::new();
    }
    if chat.pull.is_some() {
        chat.older_waiting = Some(oldest);
        return Vec::new();
    }
    read_page(chat, cursor)
}

/// The newest held row is on screen: moves the shared read frontier, once per new row.
pub fn newest_seen(chat: &mut Chat) -> Vec<ClientEvent> {
    if !chat.connected || chat.cursor <= chat.read_up_to.max(chat.read_sent) {
        return Vec::new();
    }
    chat.read_sent = chat.cursor;
    vec![ClientEvent::read_state(chat.cursor)]
}

/// Held rows came on screen: those a disconnect may have changed are read again, once.
pub fn rows_on_screen(chat: &mut Chat, seqs: &BTreeSet<u64>) -> Vec<ClientEvent> {
    read_again(chat, seqs);
    send_waiting(chat)
}

fn read_again(chat: &mut Chat, seqs: &BTreeSet<u64>) {
    let stale: BTreeSet<u64> = chat
        .read_before_disconnect
        .intersection(seqs)
        .copied()
        .collect();
    chat.read_before_disconnect
        .retain(|seq| !stale.contains(seq));
    chat.rows_to_read.extend(stale);
}

/// One server event. `now` is the caller's clock in seconds, for approval deadlines.
pub fn receive(chat: &mut Chat, event: ServerEvent, now: i64) -> Vec<ClientEvent> {
    match event {
        ServerEvent::Accepted { client_msg_id, .. } => settle(chat, &client_msg_id),
        ServerEvent::TurnStarted {
            turn_id,
            in_reply_to,
        } => start_turn(chat, turn_id, Some(in_reply_to), now),
        ServerEvent::TextDelta { turn_id, text } => {
            held_turn(chat, &turn_id, now).draft.push_str(&text)
        }
        ServerEvent::ToolEvent {
            turn_id,
            tool,
            phase,
            ..
        } => tool_event(held_turn(chat, &turn_id, now), tool, &phase, now),
        ServerEvent::TextDone(reply) => return text_done(chat, reply),
        ServerEvent::TurnError {
            turn_id,
            code,
            message,
        } => turn_failed(chat, &turn_id, &code, &message),
        ServerEvent::TurnDone { turn_id } => end_turn(chat, &turn_id),
        ServerEvent::Row(row) => return apply_live(chat, HeldRow { row, turn_id: None }),
        ServerEvent::RowUpdated { server_seq } => return updated(chat, server_seq),
        ServerEvent::Approval(approval) => hold_approval(chat, approval, now),
        ServerEvent::ApprovalResolved { approval_id, .. } => {
            chat.approvals
                .retain(|held| held.approval.approval_id != approval_id);
        }
        ServerEvent::ReadState { read_up_to_seq } => chat.read_up_to = read_up_to_seq,
        ServerEvent::HistoryPage(page) => return merge(chat, page),
        ServerEvent::Error(error) => refused(chat, &error),
        ServerEvent::ServerHello { .. }
        | ServerEvent::SearchResults { .. }
        | ServerEvent::MediaPath(_)
        | ServerEvent::MediaMissing { .. }
        | ServerEvent::Unknown(_) => {}
    }
    Vec::new()
}

fn settle(chat: &mut Chat, client_msg_id: &str) {
    chat.pending
        .retain(|out| out.client_msg_id != client_msg_id);
}

fn start_turn(chat: &mut Chat, turn_id: String, in_reply_to: Option<String>, now: i64) {
    chat.turn = Some(Turn {
        turn_id,
        in_reply_to,
        draft: String::new(),
        tools: Vec::new(),
        started: now,
        stopping: false,
    });
}

/// The turn `turn_id`, begun here when this connection joined it midway.
fn held_turn<'a>(chat: &'a mut Chat, turn_id: &str, now: i64) -> &'a mut Turn {
    let same = chat
        .turn
        .as_ref()
        .is_some_and(|turn| turn.turn_id == turn_id);
    if !same {
        start_turn(chat, turn_id.to_owned(), None, now);
    }
    chat.turn.as_mut().expect("a turn was just held")
}

/// A start adds a running call; a stop ends the oldest running call of that name, if any.
fn tool_event(turn: &mut Turn, tool: String, phase: &ToolPhase, now: i64) {
    match phase {
        ToolPhase::Start => turn.tools.push(ToolRun {
            name: tool,
            state: ToolState::Running,
            started: now,
            ended: None,
        }),
        ToolPhase::Stop => {
            let running = turn
                .tools
                .iter_mut()
                .find(|run| run.name == tool && run.state == ToolState::Running);
            if let Some(run) = running {
                run.state = ToolState::Done;
                run.ended = Some(now);
            }
        }
        ToolPhase::Other(_) => {}
    }
}

/// A reply at its row. It ends its own turn only; a turn may send more than one.
fn text_done(chat: &mut Chat, reply: Reply) -> Vec<ClientEvent> {
    let Reply {
        turn_id,
        server_seq,
        text,
        blocks,
        skipped,
    } = reply;
    let ours = chat.turn.take_if(|turn| turn.turn_id == turn_id);
    let row = Row {
        server_seq,
        role: "assistant".into(),
        text,
        ts: None,
        client_msg_id: None,
        in_reply_to: ours.and_then(|turn| turn.in_reply_to),
        kind: None,
        metadata: None,
        media_refs: Vec::new(),
        link_previews: Vec::new(),
        blocks,
        skipped,
    };
    apply_live(
        chat,
        HeldRow {
            row,
            turn_id: Some(turn_id),
        },
    )
}

fn end_turn(chat: &mut Chat, turn_id: &str) {
    chat.turn.take_if(|turn| turn.turn_id == turn_id);
}

fn turn_failed(chat: &mut Chat, turn_id: &str, code: &str, message: &str) {
    end_turn(chat, turn_id);
    chat.last_error = Some(match code {
        "cancelled" => STOPPED.into(),
        "interrupted" => INTERRUPTED.into(),
        _ => format!("The reply did not finish: {message}"),
    });
}

fn hold_approval(chat: &mut Chat, approval: Approval, now: i64) {
    let expires_at = now.saturating_add(i64::try_from(approval.ttl_s).unwrap_or(i64::MAX));
    let held = HeldApproval {
        approval,
        expires_at,
    };
    let id = &held.approval.approval_id;
    match chat
        .approvals
        .iter_mut()
        .find(|card| &card.approval.approval_id == id)
    {
        Some(card) => *card = held,
        None => chat.approvals.push(held),
    }
}

/// A refusal the connection survives, in the daemon's word. One naming a request ends that
/// request's delivery. A media failure (`ref`) is the grants', not the timeline's.
fn refused(chat: &mut Chat, error: &ServerError) {
    if error.r#ref.is_some() {
        return;
    }
    if let Some(id) = &error.client_msg_id {
        settle(chat, id);
    }
    chat.last_error = Some(format!("The daemon reported: {}", error.reason));
}

/// A notice that a held row changed: it is read again. A row not held is read as it stands
/// when a page brings it.
fn updated(chat: &mut Chat, seq: u64) -> Vec<ClientEvent> {
    if !chat.rows.iter().any(|held| held.row.server_seq == seq) {
        return Vec::new();
    }
    chat.rows_to_read.insert(seq);
    send_waiting(chat)
}

/// A live row is shown when it is the next seq, dropped at or below the cursor, and on a gap
/// pulled from the cursor instead: the pull brings every row the gap hides.
fn apply_live(chat: &mut Chat, held: HeldRow) -> Vec<ClientEvent> {
    let seq = held.row.server_seq;
    if seq <= chat.cursor {
        return Vec::new();
    }
    if seq != chat.cursor + 1 {
        return read_page(chat, Cursor::AfterSeq(chat.cursor));
    }
    chat.rows.push(held);
    chat.cursor = seq;
    if chat.rows.len() > MAX_HELD {
        let forgotten = chat.rows.remove(0).row.server_seq;
        chat.rows_to_read.remove(&forgotten);
        chat.read_before_disconnect.remove(&forgotten);
        chat.has_older = true;
    }
    Vec::new()
}

/// A page answers the read out. Its rows go over the held ones by seq; the cursor moves to its
/// newest row, and while that is below the head the read goes on, ahead of anything waiting.
fn merge(chat: &mut Chat, page: Page) -> Vec<ClientEvent> {
    let answered = chat.pull.take();
    let page_seqs: BTreeSet<u64> = page.messages.iter().map(|row| row.server_seq).collect();
    let newest = page_seqs.last().copied();
    put_rows(chat, page.messages);
    chat.cursor = chat.cursor.max(newest.unwrap_or(0));
    chat.head = page.history_head_seq;
    if let Some(Pull::Page(Cursor::BeforeSeq(_))) = answered {
        chat.has_older = page.next_before_seq.is_some();
    }
    chat.read_before_disconnect
        .retain(|seq| !page_seqs.contains(seq));
    // An empty page found no row from the lowest asked on: nothing is left to read.
    if let (Some(Pull::Rows(asked)), Some(reached)) = (answered, newest) {
        chat.rows_to_read
            .extend(asked.into_iter().filter(|seq| *seq > reached));
    }
    if chat.cursor < chat.head {
        let cursor = Cursor::AfterSeq(chat.cursor);
        return read_page(chat, cursor);
    }
    send_waiting(chat)
}

/// Puts a page's rows over the held ones by seq. A held row keeps the turn it was streamed in,
/// so the page draws it as the same reply; one whose content changed is drawn again by its key.
fn put_rows(chat: &mut Chat, rows: Vec<Row>) {
    for row in rows {
        let seq = row.server_seq;
        match chat
            .rows
            .binary_search_by_key(&seq, |held| held.row.server_seq)
        {
            Ok(index) => chat.rows[index].row = row,
            Err(index) => chat.rows.insert(index, HeldRow { row, turn_id: None }),
        }
    }
}

/// Reads a page from `cursor`, when no read is out.
fn read_page(chat: &mut Chat, cursor: Cursor) -> Vec<ClientEvent> {
    send_pull(chat, Pull::Page(cursor), cursor, PAGE_LIMIT)
}

/// Sends a read when none is out and the connection is up. Every read is asked here.
fn send_pull(chat: &mut Chat, pull: Pull, cursor: Cursor, limit: u16) -> Vec<ClientEvent> {
    if chat.pull.is_some() || !chat.connected {
        return Vec::new();
    }
    chat.pull = Some(pull);
    vec![ClientEvent::pull(cursor, limit)]
}

/// The read waiting first, when none is out: the page the reader scrolled to, then the held
/// rows to read again, at most `MAX_READ` seqs of them per read.
fn send_waiting(chat: &mut Chat) -> Vec<ClientEvent> {
    if chat.pull.is_some() || !chat.connected {
        return Vec::new();
    }
    if let Some(oldest) = chat.older_waiting.take() {
        return read_page(chat, Cursor::BeforeSeq(oldest));
    }
    let Some(&first) = chat.rows_to_read.first() else {
        return Vec::new();
    };
    let last_allowed = first.saturating_add(u64::from(MAX_READ) - 1);
    let taken: BTreeSet<u64> = chat
        .rows_to_read
        .range(first..=last_allowed)
        .copied()
        .collect();
    let last = *taken.last().expect("the first seq is taken");
    chat.rows_to_read.retain(|seq| !taken.contains(seq));
    let limit = u16::try_from(last - first + 1).expect("a span within MAX_READ");
    send_pull(
        chat,
        Pull::Rows(taken),
        Cursor::AfterSeq(first.saturating_sub(1)),
        limit,
    )
}
