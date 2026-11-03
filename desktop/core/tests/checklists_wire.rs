//! The lists over a fake daemon (M85 lists §8, "Routing" and "End to end"): which events are the
//! lists', a reply's card from the handshake to the push that confirms a tick, a refused tick, a
//! tick made offline and one unanswered when the connection closes, and the pulls of more lists
//! on screen than a connection holds. Each step feeds the daemon's line as the chat session does:
//! to the lists when they take it (`routing::lists_event`), else to the timeline.

use fermix_client::chat::{view, Entry, ListsCard, RowsSeen};
use fermix_client::chat_wire::protocol::{read_line, ClientEvent, ListOp, ServerEvent};
use fermix_client::chat_wire::timeline::{self, Chat, REFUSED_UNREAD};
use fermix_client::checklists::card::{drawing, Drawing, Stands};
use fermix_client::checklists::library::Library;
use fermix_client::checklists::routing::lists_event;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const LIST: &str = "ck_aaaaaaaaaa";
const LEMONS: &str = "ci_aaaaaaaaaa";
const MILK: &str = "ci_bbbbbbbbbb";

fn event(line: Value) -> ServerEvent {
    read_line(line.to_string().as_bytes()).expect("a framed line")
}

/// The chat and the lists as the session holds them, and every event the session sent.
#[derive(Default)]
struct Wire {
    chat: Chat,
    lists: Library,
    sent: Vec<ClientEvent>,
}

impl Wire {
    fn handshake(&mut self) {
        self.sent
            .extend(timeline::negotiated(&mut self.chat, &BTreeSet::new()));
        self.sent.extend(self.lists.negotiated(7));
    }

    fn disconnect(&mut self) {
        timeline::disconnected(&mut self.chat);
        self.lists.disconnected();
    }

    fn receive(&mut self, line: Value) {
        let event = event(line);
        match lists_event(&mut self.lists, &event) {
            Some(sent) => self.sent.extend(sent),
            None => self
                .sent
                .extend(timeline::receive(&mut self.chat, event, 0)),
        }
    }

    /// The only reply's lists card.
    fn card(&self) -> ListsCard {
        let shown = view(&self.chat);
        let cards = shown.entries.into_iter().filter_map(|entry| match entry {
            Entry::Assistant { lists, .. } => lists,
            _ => None,
        });
        let cards: Vec<ListsCard> = cards.collect();
        let [card] = cards.as_slice() else {
            panic!("one lists card: {cards:?}");
        };
        card.clone()
    }

    /// The card comes on screen: it pulls the lists it draws.
    fn on_screen(&mut self) {
        let card = self.card();
        for list in card.pulls() {
            self.sent.extend(self.lists.draw(&list.list_id, false));
        }
    }

    /// The page looks at what is on screen, the rows `on_screen`, as after a scroll or a draw:
    /// each live card that came on screen since the last look asks for the lists it draws.
    fn look(&mut self, seen: &mut RowsSeen, on_screen: &[u64]) {
        let came = seen.look(on_screen.iter().copied().collect());
        let shown = view(&self.chat);
        for (entry, seq) in shown.entries.iter().zip(&shown.seqs) {
            let (
                Entry::Assistant {
                    lists: Some(card), ..
                },
                Some(seq),
            ) = (entry, seq)
            else {
                continue;
            };
            if !came.contains(seq) {
                continue;
            }
            for list in card.pulls() {
                self.sent.extend(self.lists.draw(&list.list_id, false));
            }
        }
    }

    fn tick(&mut self, row_id: &str, client_msg_id: &str) {
        let op = ListOp::Check {
            row_id: row_id.into(),
        };
        self.sent
            .extend(self.lists.edit(LIST, op, client_msg_id.into()));
    }

    /// What the card draws now, with nothing settling.
    fn drawn(&self) -> Drawing {
        let card = self.card();
        drawing(&card.lists, card.live, &self.lists, false, &BTreeSet::new()).expect("a card")
    }

    /// The card's open item ids, its counts, and the sentence under its rows.
    fn rows(&self) -> (Vec<String>, Stands, Option<String>) {
        let Drawing::One {
            head,
            rows,
            refusal,
        } = self.drawn()
        else {
            panic!("a one-list card");
        };
        let ids = rows.rows.iter().map(|row| row.row_id.clone()).collect();
        (ids, head.stands, refusal)
    }

    /// The kinds of what was sent since the last call, each with the list or request it names.
    fn took(&mut self) -> Vec<String> {
        let sent = std::mem::take(&mut self.sent);
        sent.iter().filter_map(named).collect()
    }
}

fn named(event: &ClientEvent) -> Option<String> {
    match event {
        ClientEvent::ChecklistsPull { .. } => Some("index".into()),
        ClientEvent::ChecklistPull { list_id, .. } => Some(format!("pull {list_id}")),
        ClientEvent::ChecklistEdit { client_msg_id, .. } => Some(format!("edit {client_msg_id}")),
        _ => None,
    }
}

fn index(open: u64, done: u64, rev: u64) -> Value {
    json!({"type": "checklists", "profile_id": "main", "lists": [
        {"list_id": LIST, "title": "Groceries", "open": open, "done": done, "rev": rev}
    ]})
}

fn list(rev: u64, lemons_done: bool) -> Value {
    json!({"type": "checklist", "profile_id": "main", "list": {
    "list_id": LIST, "title": "Groceries", "rev": rev, "rows": [
        {"row_id": LEMONS, "kind": "item", "text": "Lemons", "done": lemons_done},
        {"row_id": MILK, "kind": "item", "text": "Milk", "done": false}
    ]}})
}

fn reply_with_the_list() -> Value {
    json!({"type": "row", "profile_id": "main", "server_seq": 1, "role": "assistant",
    "text": "Added lemons.", "ts": "2026-11-03T09:00:00Z", "media_refs": [],
    "blocks": [{"type": "checklists", "lists": [
        {"list_id": LIST, "title": "Groceries", "rows": [LEMONS]}
    ]}]})
}

fn accepted(id: &str, duplicate: bool) -> Value {
    json!({"type": "accepted", "client_msg_id": id, "duplicate": duplicate})
}

/// Connected, the index answered, the reply drawn and its card on screen with its pull answered.
fn drawing_the_card() -> Wire {
    let mut wire = Wire::default();
    wire.handshake();
    assert_eq!(wire.took(), ["index"], "the handshake asks for the index");
    wire.receive(index(2, 0, 1));
    wire.receive(json!({"type": "history_page", "profile_id": "main",
        "history_head_seq": 0, "messages": []}));
    wire.receive(reply_with_the_list());
    assert!(wire.card().live);
    assert_eq!(
        wire.took(),
        Vec::<String>::new(),
        "a card does not pull when it is built"
    );
    wire.on_screen();
    assert_eq!(wire.took(), [format!("pull {LIST}")]);
    wire.receive(list(1, false));
    wire
}

/// A handshake, the index, a reply with a block, the card pulling as it comes on screen, a tick
/// drawn at once, `accepted`, and the push that confirms it.
#[test]
fn a_tick_on_a_cards_list_is_drawn_at_once_and_confirmed_by_the_push() {
    let mut wire = drawing_the_card();
    let counted = |open, done| Stands::Counted { open, done };
    assert_eq!(
        wire.rows(),
        (vec![LEMONS.into(), MILK.into()], counted(2, 0), None)
    );
    wire.tick(LEMONS, "linux-t1");
    assert_eq!(wire.took(), ["edit linux-t1"]);
    assert_eq!(
        wire.rows(),
        (vec![MILK.into()], counted(1, 1), None),
        "drawn at once"
    );
    wire.receive(accepted("linux-t1", false));
    assert_eq!(wire.took(), Vec::<String>::new());
    wire.receive(list(2, true));
    wire.receive(index(1, 1, 2));
    assert_eq!(wire.rows(), (vec![MILK.into()], counted(1, 1), None));
    assert!(wire.lists.state(LIST).expect("held").pending().is_empty());
    assert_eq!(
        wire.chat.last_error, None,
        "the lists' answers are not the chat's"
    );
}

/// A second tick refused: the row is back as the engine has it, the sentence under the rows, and
/// the list pulled again.
#[test]
fn a_refused_tick_puts_the_row_back_with_the_sentence_and_pulls_the_list_again() {
    let mut wire = drawing_the_card();
    wire.tick(LEMONS, "linux-t1");
    wire.receive(accepted("linux-t1", false));
    wire.receive(list(2, true));
    wire.took();
    wire.tick(MILK, "linux-t2");
    let sentence = "That row is not in this list any more; pull the list again.";
    wire.receive(
        json!({"type": "error", "reason": "request_failed", "message": sentence,
        "client_msg_id": "linux-t2"}),
    );
    assert_eq!(wire.took(), ["edit linux-t2", &format!("pull {LIST}")]);
    let counted = Stands::Counted { open: 1, done: 1 };
    assert_eq!(
        wire.rows(),
        (vec![MILK.into()], counted, Some(sentence.into()))
    );
    assert_eq!(wire.chat.last_error, None);
}

/// Offline: a tick drawn while the chat is down, a reconnect that sends it again before it asks
/// for the index, `duplicate: true` for a tick the engine had applied, and the tick drawn once.
#[test]
fn a_tick_made_offline_is_sent_again_before_the_pull_and_drawn_once() {
    let mut wire = drawing_the_card();
    wire.disconnect();
    wire.tick(LEMONS, "linux-t1");
    assert_eq!(
        wire.took(),
        Vec::<String>::new(),
        "nothing is sent while down"
    );
    let counted = |open, done| Stands::Counted { open, done };
    assert_eq!(wire.rows(), (vec![MILK.into()], counted(1, 1), None));
    wire.handshake();
    assert_eq!(wire.took(), ["edit linux-t1", "index"]);
    wire.receive(accepted("linux-t1", true));
    wire.receive(index(1, 1, 2));
    assert_eq!(
        wire.took(),
        [format!("pull {LIST}")],
        "the list held is pulled again"
    );
    wire.receive(list(2, true));
    assert_eq!(
        wire.rows(),
        (vec![MILK.into()], counted(1, 1), None),
        "the tick counted once"
    );
}

/// A tick refused because the server cannot be reached or the home is moving away is kept: it
/// stays drawn with no sentence, the refusal is the chat's, and the next handshake sends it again
/// before it asks for the index.
#[test]
fn a_tick_refused_while_the_server_is_unreachable_waits_and_is_sent_again() {
    let mut wire = drawing_the_card();
    wire.tick(LEMONS, "linux-t1");
    assert_eq!(wire.took(), ["edit linux-t1"]);
    let moving = "Fermix is moving this home to studio; your message is kept until it is done.";
    let refused = json!({"type": "error", "reason": "primary_unreachable", "message": moving,
        "client_msg_id": "linux-t1"});
    let taken = lists_event(&mut wire.lists, &event(refused));
    assert_eq!(taken, None, "the chat's");
    let counted = Stands::Counted { open: 1, done: 1 };
    assert_eq!(
        wire.rows(),
        (vec![MILK.into()], counted, None),
        "the tick still drawn"
    );
    wire.disconnect();
    wire.handshake();
    assert_eq!(wire.took(), ["edit linux-t1", "index"]);
}

/// Any other refusal that names a tick refuses it, `desktop_revoked` too were it to name one: a
/// tick refused for its shape would be refused again at every handshake. Each puts the row back
/// with the engine's sentence and pulls the list, and the next handshake does not send the tick
/// again.
#[test]
fn any_other_refusal_naming_a_tick_refuses_it_and_it_is_not_sent_again() {
    let revoked = "The server studio does not know this computer.";
    let shape = "op is not a valid list op.";
    for (reason, sentence) in [("desktop_revoked", revoked), ("invalid_field", shape)] {
        let mut wire = drawing_the_card();
        wire.tick(LEMONS, "linux-t1");
        wire.took();
        wire.receive(
            json!({"type": "error", "reason": reason, "message": sentence,
            "client_msg_id": "linux-t1"}),
        );
        assert_eq!(wire.took(), [format!("pull {LIST}")], "{reason}");
        let counted = Stands::Counted { open: 2, done: 0 };
        let back = (
            vec![LEMONS.into(), MILK.into()],
            counted,
            Some(sentence.into()),
        );
        assert_eq!(wire.rows(), back, "{reason}");
        wire.disconnect();
        wire.handshake();
        assert_eq!(wire.took(), ["index"], "{reason}: not sent again");
    }
}

/// The index and a list are the lists'. An `accepted` or a refusal is theirs only when it names an
/// edit sent here; any other is the timeline's, a refused pull's included, which names no
/// request.
#[test]
fn the_lists_take_their_events_and_only_the_answers_to_their_edits() {
    let refused = |id: Option<&str>| {
        let mut line = json!({"type": "error", "reason": "request_failed", "message": "No."});
        if let Some(id) = id {
            line["client_msg_id"] = json!(id);
        }
        line
    };
    let mut wire = drawing_the_card();
    wire.tick(LEMONS, "linux-1");
    let others = [
        accepted("linux-9", false),
        refused(None),
        refused(Some("linux-9")),
        json!({"type": "text_delta", "turn_id": "t1", "text": "Hi"}),
    ];
    for other in others {
        let taken = lists_event(&mut wire.lists, &event(other.clone()));
        assert_eq!(taken, None, "{other}");
    }
    let taken = lists_event(&mut wire.lists, &event(refused(Some("linux-1"))));
    let pulled: Vec<String> = taken
        .expect("the lists'")
        .iter()
        .filter_map(named)
        .collect();
    assert_eq!(pulled, [format!("pull {LIST}")], "the list is pulled again");
    let state = wire.lists.state(LIST).expect("held");
    assert_eq!(state.refusal(), Some("No."));
}

/// An answer to an edit sent here that this build cannot read is still the edit's: a refusal in
/// this app's words, an `accepted` taken. One naming a file, a list event, or one naming no edit
/// sent here is not the lists'.
#[test]
fn an_unreadable_answer_to_an_edit_sent_here_is_the_lists() {
    let unread = |line: Value| {
        let unread = event(line);
        assert!(matches!(unread, ServerEvent::Unreadable(_)), "{unread:?}");
        unread
    };
    let mut wire = drawing_the_card();
    wire.tick(LEMONS, "linux-1");
    let others = [
        json!({"type": "error", "reason": 5, "client_msg_id": "linux-1", "ref": "f1"}),
        json!({"type": "error", "reason": 5, "client_msg_id": "linux-9"}),
        json!({"type": "checklist", "profile_id": "main", "list": 5}),
        json!({"type": "checklists", "profile_id": "main", "lists": 5}),
    ];
    for other in others {
        let taken = lists_event(&mut wire.lists, &unread(other.clone()));
        assert_eq!(taken, None, "{other}");
    }
    let refusal = json!({"type": "error", "reason": 5, "client_msg_id": "linux-1"});
    assert!(lists_event(&mut wire.lists, &unread(refusal)).is_some());
    let state = wire.lists.state(LIST).expect("held");
    assert_eq!(state.refusal(), Some(REFUSED_UNREAD));
    let mut wire = drawing_the_card();
    wire.tick(LEMONS, "linux-1");
    let accepted = json!({"type": "accepted", "client_msg_id": "linux-1", "duplicate": "no"});
    assert_eq!(
        lists_event(&mut wire.lists, &unread(accepted)),
        Some(Vec::new())
    );
}

/// The refusals that end a connection, a revoke and a lost server, name no request (PROTOCOL.md
/// "This desktop revoked"): they are the chat's, and a tick unanswered when one comes stays drawn
/// and is sent again at the next handshake, as the chat's outbox is.
#[test]
fn a_tick_unanswered_when_the_connection_is_refused_waits_and_is_sent_again() {
    let revoked = "The server studio does not know this computer.";
    let lost = "Fermix lost its link to the server.";
    for (reason, sentence) in [("desktop_revoked", revoked), ("primary_unreachable", lost)] {
        let mut wire = drawing_the_card();
        wire.tick(LEMONS, "linux-t1");
        wire.took();
        let closing = json!({"type": "error", "reason": reason, "message": sentence});
        assert_eq!(
            lists_event(&mut wire.lists, &event(closing.clone())),
            None,
            "{reason}: the chat's"
        );
        wire.receive(closing);
        let counted = Stands::Counted { open: 1, done: 1 };
        assert_eq!(
            wire.rows(),
            (vec![MILK.into()], counted, None),
            "{reason}: the tick still drawn"
        );
        wire.disconnect();
        wire.handshake();
        assert_eq!(wire.took(), ["edit linux-t1", "index"], "{reason}");
    }
}

/// Eighteen lists on screen, two past what a connection holds: six replies, each a card of three.
/// Each card asks for its lists as it comes on screen, once until it leaves, so the pulls end
/// with every answer drawn; asked at every look instead, each answer would let go of a list
/// another card then pulled again, without end (M85 lists §2.1, §5.2). A card that leaves and
/// comes back asks again, once.
#[test]
fn more_lists_on_screen_than_a_connection_holds_are_each_pulled_once_per_arrival() {
    let ids: Vec<String> = (0..18).map(|n| format!("ck_{n:010}")).collect();
    let entries: Vec<Value> = ids
        .iter()
        .map(|id| json!({"list_id": id, "title": id, "open": 0, "done": 0, "rev": 1}))
        .collect();
    let mut wire = Wire::default();
    wire.handshake();
    wire.receive(json!({"type": "checklists", "profile_id": "main", "lists": entries}));
    wire.receive(json!({"type": "history_page", "profile_id": "main",
        "history_head_seq": 0, "messages": []}));
    for (seq, three) in (1u64..).zip(ids.chunks(3)) {
        let lists: Vec<Value> = three
            .iter()
            .map(|id| json!({"list_id": id, "title": id, "rows": []}))
            .collect();
        wire.receive(
            json!({"type": "row", "profile_id": "main", "server_seq": seq,
            "role": "assistant", "text": "Read them.", "ts": "2026-11-03T09:00:00Z",
            "media_refs": [], "blocks": [{"type": "checklists", "lists": lists}]}),
        );
    }
    wire.took();
    let all = [1, 2, 3, 4, 5, 6];
    let mut seen = RowsSeen::default();
    wire.look(&mut seen, &all);
    let pulls: Vec<String> = ids.iter().map(|id| format!("pull {id}")).collect();
    assert_eq!(wire.took(), pulls, "each list once, as its card came");
    for id in &ids {
        wire.receive(json!({"type": "checklist", "profile_id": "main",
            "list": {"list_id": id, "title": id, "rev": 1, "rows": []}}));
        wire.look(&mut seen, &all);
        assert_eq!(
            wire.took(),
            Vec::<String>::new(),
            "{id}'s answer pulls nothing"
        );
    }
    wire.look(&mut seen, &all[1..]);
    assert_eq!(
        wire.took(),
        Vec::<String>::new(),
        "a card leaving asks nothing"
    );
    wire.look(&mut seen, &all);
    assert_eq!(
        wire.took(),
        pulls[..3],
        "back on screen, the first card asks again: its first two lists were let go, and \
        pulling them let go of its third"
    );
    wire.look(&mut seen, &all);
    assert_eq!(wire.took(), Vec::<String>::new(), "once");
}
