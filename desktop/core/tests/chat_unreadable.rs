//! A line that is an event this build knows, but whose fields it cannot read, drops only itself
//! once the handshake is done: the connection stays up, what it still names settles what it would
//! have, a card that asks the owner leaves a placeholder where its answer is owed, and a read or a
//! turn it would have ended is not left hanging. A line that is no event at all, and an
//! unreadable `server_hello` or `error` while shaking hands, still end the connection
//! (PROTOCOL.md "Handshake state machine", "Approvals", "Proposals").

use fermix_client::chat::{reveal_key, view, Entry, Key};
use fermix_client::chat_wire::grants::{Answer, Grants, Outcome, UNREADABLE};
use fermix_client::chat_wire::protocol::{
    line_buffer, read_line, ClientEvent, Cursor, Decision, DecodeError, ServerEvent, Unreadable,
    MAX_FRAME_BYTES,
};
use fermix_client::chat_wire::session::{handshake, Handshake, Phase, Session};
use fermix_client::chat_wire::timeline::{
    self, Chat, Standing, MAX_UNREAD_PAGES, NEWEST, PAGE_LIMIT, REFUSED_UNREAD, SETTLED_UNREAD,
    TURN_UNREAD,
};
use fermix_client::chat_wire::unshown::{self, Asks, MAX_KEPT, STRINGS};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");
const MEDIA: &str = "/home/me/.fermix/companion/media";
const NOTE: &str = "9879f6d3c9efefe8c4f4aabe9d3269762264be52794e96f62ab8ffa53b7fadec";

fn read(value: Value) -> ServerEvent {
    read_line(value.to_string().as_bytes()).expect("a line that is an event reads")
}

fn receive(chat: &mut Chat, value: Value) -> Vec<ClientEvent> {
    timeline::receive(chat, read(value), 0)
}

fn row(seq: u64) -> Value {
    json!({"type": "row", "profile_id": "main", "server_seq": seq, "role": "assistant",
        "text": format!("row {seq}"), "ts": "t", "media_refs": []})
}

/// A proposal whose card lacks everything but its state, and its id where one is given.
fn bad_proposal(id: Option<&str>) -> Value {
    let mut card = json!({"state": "waiting"});
    if let Some(id) = id {
        card["proposal_id"] = json!(id);
    }
    json!({"type": "proposal", "profile_id": "main", "proposal": card})
}

fn bad_approval(id: &str) -> Value {
    json!({"type": "approval", "profile_id": "main", "approval_id": id, "text": 5})
}

fn resolved(id: &str) -> Value {
    json!({"type": "proposal_resolved", "profile_id": "main", "proposal_id": id,
        "state": "denied", "sentence": "Denied; nothing ran."})
}

/// An `error` whose reason is no string, with the names it carries.
fn bad_error(names: Value) -> Value {
    let mut error = json!({"type": "error", "profile_id": "main", "reason": 5});
    for (key, value) in names.as_object().expect("names") {
        error[key] = value.clone();
    }
    error
}

/// A page whose messages are no list.
fn bad_page() -> Value {
    json!({"type": "history_page", "profile_id": "main", "messages": "none"})
}

fn empty_page() -> Value {
    json!({"type": "history_page", "profile_id": "main", "messages": [], "history_head_seq": 0})
}

/// A connected chat holding the golden approval card, and the card's id.
fn holding_an_approval() -> (Chat, String) {
    let mut chat = connected();
    let golden = SERVER
        .lines()
        .find(|line| line.starts_with(r#"{"type":"approval","#))
        .expect("an approval golden");
    let event = read_line(golden.as_bytes()).expect("a golden line");
    let ServerEvent::Approval(approval) = &event else {
        panic!("an approval reads: {event:?}");
    };
    let id = approval.approval_id.clone();
    timeline::receive(&mut chat, event, 0);
    assert_eq!(chat.approvals.len(), 1);
    (chat, id)
}

fn turn_held(chat: &Chat) -> Option<&str> {
    chat.turn.as_ref().map(|turn| turn.turn_id.as_str())
}

fn started() -> Value {
    json!({"type": "turn_started", "profile_id": "main", "turn_id": "t1",
        "in_reply_to": "linux-1"})
}

/// A chat whose handshake is done and whose cold start read found nothing.
fn connected() -> Chat {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert!(receive(&mut chat, empty_page()).is_empty());
    chat
}

/// A connected chat holding the golden proposal card, and the card's id.
fn holding_a_card() -> (Chat, String) {
    let mut chat = connected();
    let golden = SERVER
        .lines()
        .find(|line| line.starts_with(r#"{"type":"proposal","#))
        .expect("a proposal golden");
    let event = read_line(golden.as_bytes()).expect("a golden line");
    let ServerEvent::Proposal { proposal } = &event else {
        panic!("a proposal reads: {event:?}");
    };
    let id = proposal.proposal_id.clone();
    timeline::receive(&mut chat, event, 0);
    (chat, id)
}

fn placeholders(chat: &Chat) -> usize {
    view(chat)
        .entries
        .iter()
        .filter(|entry| matches!(entry, Entry::Unshown))
        .count()
}

fn handshaking() -> Session {
    let mut session = Session::default();
    assert!(session.start());
    assert!(session.connected());
    session
}

fn unread(
    kind: &str,
    card: Option<&str>,
    request: Option<&str>,
    file: Option<&str>,
) -> ServerEvent {
    ServerEvent::Unreadable(Unreadable {
        kind: kind.into(),
        card_id: card.map(Into::into),
        client_msg_id: request.map(Into::into),
        r#ref: file.map(Into::into),
        ..Unreadable::default()
    })
}

#[test]
fn after_the_hello_an_unreadable_event_drops_alone_and_the_next_one_lands() {
    let mut session = handshaking();
    let hello = read_line(SERVER.lines().next().unwrap().as_bytes()).unwrap();
    assert_eq!(session.hello(&hello), Handshake::Ok);
    let mut chat = connected();
    let bytes = format!("{}\n{}\n", bad_proposal(Some("cpr_x")), row(1));
    let lines = line_buffer().push(bytes.as_bytes()).expect("two lines");
    let events: Vec<ServerEvent> = lines
        .iter()
        .map(|line| read_line(line).expect("each line reads, the bad card too"))
        .collect();
    assert_eq!(events[0], unread("proposal", Some("cpr_x"), None, None));
    for event in events {
        assert!(timeline::receive(&mut chat, event, 0).is_empty());
    }
    let shown = view(&chat);
    assert_eq!(shown.keys, [Key::Row(1), Key::Unshown]);
    assert!(matches!(&shown.entries[0], Entry::Assistant { text, .. } if text == "row 1"));
}

#[test]
fn a_bad_event_reads_as_unreadable_by_its_type_and_ids_and_never_its_content() {
    assert_eq!(
        read(bad_proposal(Some("cpr_x"))),
        unread("proposal", Some("cpr_x"), None, None)
    );
    let long = "p".repeat(300);
    assert_eq!(
        read(bad_proposal(Some(&long))),
        unread("proposal", None, None, None),
        "an id past its bound is not kept"
    );
    let accepted = json!({"type": "accepted", "client_msg_id": "linux-1", "duplicate": "no"});
    assert_eq!(
        read(accepted),
        unread("accepted", None, Some("linux-1"), None)
    );
    let error = bad_error(json!({"client_msg_id": "linux-2", "ref": NOTE}));
    assert_eq!(
        read(error),
        unread("error", None, Some("linux-2"), Some(NOTE))
    );
    let grant = json!({"type": "media_path", "ref": NOTE, "path": 5});
    assert_eq!(read(grant), unread("media_path", None, None, Some(NOTE)));
    let row = json!({"type": "row", "server_seq": "two", "client_msg_id": "linux-3", "ref": NOTE});
    assert_eq!(
        read(row),
        unread("row", None, None, None),
        "only a request's answer names a request, and only a file's answer a file"
    );
    let secret = read(
        json!({"type": "approval", "approval_id": "apr_1", "token": 7,
        "text": "a password: hunter2"}),
    );
    assert!(!format!("{secret:?}").contains("hunter2"), "{secret:?}");
}

#[test]
fn a_line_that_is_no_event_still_ends_the_connection() {
    assert!(matches!(
        read_line(b"{not json"),
        Err(DecodeError::NotJson(_))
    ));
    assert_eq!(read_line(b"[1]"), Err(DecodeError::NotAnObject));
    assert_eq!(
        read_line(br#"{"text":"no type"}"#),
        Err(DecodeError::MissingType)
    );
    assert_eq!(
        read_line(br#"{"type":7}"#),
        Err(DecodeError::MissingType),
        "a type that is no string is none"
    );
    let mut huge = vec![b'x'; MAX_FRAME_BYTES + 1];
    huge.push(b'\n');
    assert!(matches!(
        line_buffer().push(&huge),
        Err(DecodeError::FrameTooLarge(_))
    ));
    assert_eq!(
        read(json!({"type": "from_the_future", "x": 1})),
        ServerEvent::Unknown("from_the_future".into()),
        "a type this build does not know was never fatal"
    );
}

#[test]
fn an_unreadable_hello_or_error_ends_the_handshake_and_any_other_is_waited_past() {
    let hello = read(json!({"type": "server_hello", "min_version": "three", "max_version": 4}));
    assert_eq!(
        handshake(&hello),
        Handshake::Lost("the daemon's server_hello could not be read".into())
    );
    let error = read(json!({"type": "error", "reason": 5}));
    assert_eq!(
        handshake(&error),
        Handshake::Lost("the daemon's error could not be read".into())
    );
    assert_eq!(handshake(&read(bad_proposal(None))), Handshake::Ignore);
    let mut session = handshaking();
    assert!(matches!(session.hello(&hello), Handshake::Lost(_)));
    assert_eq!(session.phase(), &Phase::Handshaking, "the caller ends it");
}

#[test]
fn the_placeholder_is_one_for_every_unreadable_card_and_a_handshake_clears_it() {
    let mut chat = connected();
    for id in [Some("cpr_a"), Some("cpr_b"), None, Some("cpr_a")] {
        assert!(receive(&mut chat, bad_proposal(id)).is_empty());
    }
    assert_eq!(placeholders(&chat), 1);
    timeline::disconnected(&mut chat);
    assert_eq!(placeholders(&chat), 1, "kept while away");
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(
        placeholders(&chat),
        0,
        "a card still waiting is sent again after the hello"
    );
}

#[test]
fn a_resolution_takes_its_cards_placeholder_and_one_with_no_id_waits_for_the_handshake() {
    let mut chat = connected();
    receive(&mut chat, bad_proposal(Some("cpr_a")));
    receive(&mut chat, bad_proposal(Some("cpr_b")));
    receive(&mut chat, resolved("cpr_a"));
    assert_eq!(placeholders(&chat), 1, "cpr_b still waits");
    receive(&mut chat, resolved("cpr_b"));
    assert_eq!(placeholders(&chat), 0);
    receive(&mut chat, bad_proposal(None));
    receive(&mut chat, resolved("cpr_c"));
    assert_eq!(placeholders(&chat), 1);
}

#[test]
fn past_the_cap_a_cards_placeholder_stands_until_the_handshake() {
    let mut chat = connected();
    let ids: Vec<String> = (0..=MAX_KEPT).map(|n| format!("cpr_{n}")).collect();
    for id in &ids {
        receive(&mut chat, bad_proposal(Some(id)));
    }
    for id in &ids {
        receive(&mut chat, resolved(id));
    }
    assert_eq!(
        placeholders(&chat),
        1,
        "the card past the cap was kept without its id, so no resolution took it"
    );
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(placeholders(&chat), 0);
}

#[test]
fn each_kind_of_card_is_capped_apart() {
    let mut chat = connected();
    for n in 0..MAX_KEPT {
        receive(&mut chat, bad_proposal(Some(&format!("cpr_{n}"))));
    }
    receive(&mut chat, bad_approval("apr_1"));
    assert!(
        unshown::waiting(&chat.unshown, Asks::Approval),
        "the proposals' cap leaves the approval its placeholder"
    );
    let gone = json!({"type": "approval_resolved", "profile_id": "main",
        "approval_id": "apr_1", "outcome": "expired"});
    receive(&mut chat, gone);
    assert!(
        !unshown::waiting(&chat.unshown, Asks::Approval),
        "kept by its id"
    );
}

#[test]
fn an_unreadable_approval_is_said_with_the_approvals_never_in_the_conversation() {
    let mut chat = connected();
    assert!(!unshown::waiting(&chat.unshown, Asks::Approval));
    receive(&mut chat, bad_approval("apr_1"));
    assert!(unshown::waiting(&chat.unshown, Asks::Approval));
    assert!(!unshown::waiting(&chat.unshown, Asks::Proposal));
    assert_eq!(placeholders(&chat), 0);
    let resolved = json!({"type": "approval_resolved", "profile_id": "main",
        "approval_id": "apr_1", "outcome": "expired"});
    receive(&mut chat, resolved);
    assert!(!unshown::waiting(&chat.unshown, Asks::Approval));
}

#[test]
fn the_placeholder_stands_after_the_rows_and_before_what_is_sending() {
    let mut chat = connected();
    receive(&mut chat, row(1));
    timeline::send(&mut chat, "Is it done?", "linux-1".into());
    receive(&mut chat, bad_proposal(Some("cpr_x")));
    assert_eq!(
        view(&chat).keys,
        vec![Key::Row(1), Key::Unshown, Key::Sending("linux-1".into())]
    );
}

#[test]
fn show_in_chat_for_an_unreadable_card_scrolls_to_its_placeholder() {
    let (mut chat, held) = holding_a_card();
    assert_eq!(reveal_key(&chat, &held), Some(Key::Proposal(held.clone())));
    receive(&mut chat, bad_proposal(Some("cpr_x")));
    assert_eq!(reveal_key(&chat, "cpr_x"), Some(Key::Unshown));
    assert_eq!(
        reveal_key(&chat, "cpr_y"),
        None,
        "a card neither held nor unreadable waits for the render that draws it"
    );
    receive(&mut chat, resolved("cpr_x"));
    assert_eq!(reveal_key(&chat, "cpr_x"), None);
}

#[test]
fn an_unreadable_page_forgets_its_read_so_the_next_gap_reads_again() {
    let mut chat = connected();
    let gap = ClientEvent::pull(Cursor::BeforeSeq(NEWEST), PAGE_LIMIT);
    assert_eq!(receive(&mut chat, row(2)), vec![gap.clone()]);
    assert!(receive(&mut chat, bad_page()).is_empty());
    assert_eq!(receive(&mut chat, row(3)), vec![gap]);
}

#[test]
fn an_unreadable_reread_of_held_rows_reads_them_when_next_on_screen() {
    let mut chat = connected();
    receive(&mut chat, row(1));
    let reread = ClientEvent::pull(Cursor::AfterSeq(0), 1);
    let changed = json!({"type": "row_updated", "profile_id": "main", "server_seq": 1});
    assert_eq!(receive(&mut chat, changed), vec![reread.clone()]);
    let unreadable = json!({"type": "history_page", "profile_id": "main", "messages": "none"});
    assert!(
        receive(&mut chat, unreadable).is_empty(),
        "not asked again at once: a page that never reads would be asked forever"
    );
    let on_screen = BTreeSet::from([1]);
    assert_eq!(
        timeline::rows_on_screen(&mut chat, &on_screen),
        vec![reread]
    );
}

/// An end whose turn id cannot be read ends the turn held: this connection streams one at a time.
#[test]
fn an_unreadable_end_of_a_turn_ends_the_turn_held() {
    for kind in ["text_done", "turn_done", "turn_error"] {
        let mut chat = connected();
        receive(&mut chat, started());
        assert!(view(&chat).turn.is_some());
        receive(
            &mut chat,
            json!({"type": kind, "profile_id": "main", "turn_id": 7}),
        );
        assert_eq!(chat.turn, None, "{kind}");
        assert_eq!(view(&chat).turn, None, "{kind}");
    }
}

#[test]
fn an_unreadable_reply_is_read_from_the_cursor_and_lands() {
    let mut chat = connected();
    receive(&mut chat, row(1));
    receive(&mut chat, started());
    let delta = json!({"type": "text_delta", "profile_id": "main", "turn_id": "t1",
        "text": "It is"});
    receive(&mut chat, delta);
    let done = json!({"type": "text_done", "profile_id": "main", "turn_id": "t1",
        "server_seq": 2, "text": null});
    assert_eq!(
        receive(&mut chat, done),
        vec![ClientEvent::pull(Cursor::AfterSeq(1), PAGE_LIMIT)]
    );
    assert_eq!(chat.turn, None);
    let page = json!({"type": "history_page", "profile_id": "main", "history_head_seq": 2,
        "next_after_seq": 2, "messages": [{"server_seq": 2, "role": "assistant",
        "content": "It is done.", "ts": "t", "media_refs": []}]});
    assert!(receive(&mut chat, page).is_empty());
    assert_eq!(view(&chat).keys, [Key::Row(1), Key::Row(2)]);
}

#[test]
fn a_lone_unreadable_row_is_read_from_the_cursor_and_leaves_the_turn() {
    let mut chat = connected();
    receive(&mut chat, row(1));
    receive(&mut chat, started());
    let bad = json!({"type": "row", "profile_id": "main", "server_seq": "two"});
    assert_eq!(
        receive(&mut chat, bad),
        vec![ClientEvent::pull(Cursor::AfterSeq(1), PAGE_LIMIT)]
    );
    assert!(chat.turn.is_some(), "a row ends no turn");
}

#[test]
fn an_unreadable_accepted_settles_the_request_it_names() {
    let mut chat = connected();
    timeline::send(&mut chat, "Is it done?", "linux-1".into());
    timeline::send(&mut chat, "And the other?", "linux-2".into());
    let accepted = json!({"type": "accepted", "profile_id": "main", "client_msg_id": "linux-1",
        "duplicate": "no"});
    assert!(receive(&mut chat, accepted).is_empty());
    assert_eq!(chat.pending.len(), 1);
    assert_eq!(chat.pending[0].client_msg_id, "linux-2");
    assert_eq!(view(&chat).keys, [Key::Sending("linux-2".into())]);
}

#[test]
fn an_unreadable_refusal_of_a_message_fails_it_in_this_apps_words() {
    let mut chat = connected();
    timeline::send(&mut chat, "Is it done?", "linux-1".into());
    let refusal = bad_error(json!({"client_msg_id": "linux-1"}));
    assert!(receive(&mut chat, refusal).is_empty());
    assert!(chat.pending.is_empty());
    assert_eq!(
        view(&chat).entries,
        [Entry::Failure(REFUSED_UNREAD.into())],
        "no Sending bubble waits on, and the failure says so"
    );
}

#[test]
fn an_unreadable_refusal_of_an_answer_refuses_its_card() {
    let (mut chat, id) = holding_a_card();
    let sent = timeline::answer_proposal(&mut chat, &id, Decision::Confirm, "linux-2".into());
    assert_eq!(sent.len(), 1);
    let refusal = bad_error(json!({"client_msg_id": "linux-2"}));
    assert!(receive(&mut chat, refusal).is_empty());
    assert!(chat.pending.is_empty());
    assert_eq!(
        chat.proposals[0].standing,
        Standing::Refused {
            sentence: REFUSED_UNREAD.into()
        }
    );
    assert_eq!(chat.last_error, None, "the card says it, not the chat");
}

#[test]
fn an_unreadable_grant_fails_the_reads_of_its_ref_and_lets_it_go() {
    let mut grants = Grants::new(PathBuf::from(MEDIA));
    assert!(grants.negotiated().is_empty());
    let (ticket, answer, events) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Later);
    assert_eq!(events, vec![ClientEvent::media_fetch(NOTE)]);
    let bad = read(json!({"type": "media_path", "profile_id": "main", "ref": NOTE, "path": 5}));
    let answered = grants
        .answered(&bad)
        .expect("the answer to the read of NOTE");
    assert_eq!(answered.tickets, vec![ticket]);
    assert_eq!(answered.outcome, Outcome::Failed(UNREADABLE.into()));
    assert_eq!(answered.events, vec![ClientEvent::media_release(NOTE)]);
    let no_ref = read(json!({"type": "media_path", "profile_id": "main", "ref": 64}));
    assert_eq!(
        grants.answered(&no_ref),
        None,
        "a ref that cannot be read answers no read"
    );
}

#[test]
fn an_unreadable_failure_fails_the_reads_of_its_ref_and_releases_nothing() {
    let mut grants = Grants::new(PathBuf::from(MEDIA));
    assert!(grants.negotiated().is_empty());
    let (ticket, answer, _) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Later);
    let failed = bad_error(json!({"ref": NOTE, "message": {}}));
    let answered = grants
        .answered(&read(failed.clone()))
        .expect("the answer to the read of NOTE");
    assert_eq!(answered.tickets, vec![ticket]);
    assert_eq!(answered.outcome, Outcome::Failed(UNREADABLE.into()));
    assert!(answered.events.is_empty(), "nothing was granted to let go");
    let mut chat = connected();
    assert!(receive(&mut chat, failed).is_empty());
    assert_eq!(chat.last_error, None, "a file's failure is the grants'");
}

#[test]
fn the_placeholders_say_where_an_answer_can_come_from() {
    for text in STRINGS
        .into_iter()
        .chain([REFUSED_UNREAD, TURN_UNREAD, SETTLED_UNREAD])
    {
        let first = text.chars().next().expect("not empty");
        assert!(first.is_uppercase(), "{text} starts a sentence");
        assert!(!text.contains("Mac"), "{text} names the Mac");
        assert!(
            !text.contains("iMessage"),
            "{text} names a chat Linux has not"
        );
        assert!(!text.contains('_'), "{text} carries a wire token");
        assert!(!text.contains("  "), "{text} has a double space");
        assert!(!text.contains("fermix "), "{text} names a command");
    }
    assert!(
        unshown::PROPOSAL.contains("in your Telegram, Signal or WhatsApp chat with Fermix"),
        "the chats that carry a proposal to the owner on Linux"
    );
    assert!(
        !unshown::PROPOSAL.contains("phone"),
        "a paired phone is never sent a proposal"
    );
    assert!(unshown::PROPOSAL.contains("it expires and nothing runs"));
    assert!(unshown::APPROVAL.contains("No other chat can answer it"));
}

/// A history whose every page is unreadable cannot be shown at all: past `MAX_UNREAD_PAGES`
/// pages in a row the connection is given up and nothing more is asked. A new connection keeps
/// the count, so its first unreadable page gives it up again, while what comes before that page
/// does not.
#[test]
fn a_history_unreadable_page_after_page_ends_the_connection() {
    let mut chat = connected();
    for page in 1..=MAX_UNREAD_PAGES {
        receive(&mut chat, bad_page());
        assert!(!timeline::history_unreadable(&chat), "page {page}");
    }
    assert!(receive(&mut chat, bad_page()).is_empty());
    assert!(timeline::history_unreadable(&chat));
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert!(
        !timeline::history_unreadable(&chat),
        "a new connection starts"
    );
    receive(&mut chat, row(1));
    assert!(!timeline::history_unreadable(&chat), "a row is no page");
    receive(&mut chat, bad_page());
    assert!(
        timeline::history_unreadable(&chat),
        "still unreadable: given up at once"
    );
}

/// A readable page counts the unreadable ones from zero, and a card that cannot be read costs
/// only itself.
#[test]
fn a_readable_page_counts_again_and_a_bad_card_never_counts() {
    let mut chat = connected();
    for _ in 0..MAX_UNREAD_PAGES {
        receive(&mut chat, bad_page());
    }
    for n in 0..10 {
        receive(&mut chat, bad_proposal(Some(&format!("cpr_{n}"))));
        receive(&mut chat, bad_approval(&format!("apr_{n}")));
    }
    assert!(!timeline::history_unreadable(&chat), "cards are no pages");
    receive(&mut chat, empty_page());
    for _ in 0..MAX_UNREAD_PAGES {
        receive(&mut chat, bad_page());
    }
    assert!(
        !timeline::history_unreadable(&chat),
        "counted from the readable page"
    );
    receive(&mut chat, bad_page());
    assert!(timeline::history_unreadable(&chat));
}

/// A failure of the turn held says the reply did not finish, though why could not be read; a
/// turn's plain end says nothing, as its readable form does not.
#[test]
fn an_unreadable_failure_of_the_turn_held_says_the_reply_did_not_finish() {
    let mut chat = connected();
    receive(&mut chat, started());
    let failed = json!({"type": "turn_error", "profile_id": "main", "turn_id": "t1", "code": 5});
    receive(&mut chat, failed);
    assert_eq!(chat.turn, None);
    assert_eq!(view(&chat).entries, [Entry::Failure(TURN_UNREAD.into())]);
    let mut chat = connected();
    receive(&mut chat, started());
    receive(
        &mut chat,
        json!({"type": "turn_done", "profile_id": "main", "turn_id": 7}),
    );
    assert_eq!(chat.turn, None);
    assert_eq!(chat.last_error, None, "a turn's end is no failure");
}

/// A failure whose turn reads is said whether or not that turn is held, as a readable failure
/// is: a turn the connection no longer holds (one cut by a reconnect, or one queued behind the
/// reply streaming) still owes the owner a word. One whose turn cannot be read, with no turn
/// held, names nothing to say it of.
#[test]
fn an_unreadable_failure_of_a_turn_not_held_still_says_the_reply_did_not_finish() {
    let failed = |turn: Value| json!({"type": "turn_error", "profile_id": "main", "turn_id": turn, "code": 5});
    let mut chat = connected();
    receive(&mut chat, failed(json!("t5")));
    assert_eq!(view(&chat).entries, [Entry::Failure(TURN_UNREAD.into())]);
    let mut chat = connected();
    receive(&mut chat, failed(json!(7)));
    assert_eq!(chat.last_error, None, "no turn named and none held");
}

/// An unreadable end reads the turn it names, as far as `MAX_ID_BYTES`, and ends that turn only;
/// one naming another turn leaves the turn held, its draft and its Stop. Only a failure says so,
/// as its readable form does.
#[test]
fn an_unreadable_end_ends_only_the_turn_it_names() {
    let ends = [
        ("text_done", json!({"server_seq": "two"}), None),
        (
            "turn_error",
            json!({"code": 5}),
            Some(TURN_UNREAD.to_owned()),
        ),
    ];
    for (kind, mut end, said) in ends {
        end["type"] = json!(kind);
        end["profile_id"] = json!("main");
        let mut chat = connected();
        receive(&mut chat, started());
        end["turn_id"] = json!("t9");
        receive(&mut chat, end.clone());
        assert_eq!(turn_held(&chat), Some("t1"), "{kind} of another turn");
        assert_eq!(chat.last_error, said, "{kind} of another turn");
        end["turn_id"] = json!("t1");
        receive(&mut chat, end);
        assert_eq!(turn_held(&chat), None, "{kind} of the turn held");
    }
    // A turn_done carries nothing but its turn, so the wire cannot make one unreadable whose turn
    // reads; the timeline still ends only the turn one names.
    let done = |turn: &str| {
        ServerEvent::Unreadable(Unreadable {
            kind: "turn_done".into(),
            turn_id: Some(turn.into()),
            ..Unreadable::default()
        })
    };
    let mut chat = connected();
    receive(&mut chat, started());
    timeline::receive(&mut chat, done("t9"), 0);
    assert_eq!(turn_held(&chat), Some("t1"), "turn_done of another turn");
    assert_eq!(chat.last_error, None, "a turn's end is no failure");
    timeline::receive(&mut chat, done("t1"), 0);
    assert_eq!(turn_held(&chat), None, "turn_done of the turn held");
    let done = json!({"type": "text_done", "turn_id": "t1", "server_seq": "two"});
    let ServerEvent::Unreadable(unread) = read(done) else {
        panic!("an unreadable text_done");
    };
    assert_eq!(unread.turn_id.as_deref(), Some("t1"));
    let long = json!({"type": "turn_error", "turn_id": "t".repeat(300), "code": 5});
    let ServerEvent::Unreadable(unread) = read(long) else {
        panic!("an unreadable turn_error");
    };
    assert_eq!(unread.turn_id, None, "an id past its bound is not kept");
}

/// With no row held, a gap reads the newest page again, as a cold start does, never every row
/// from the first; with one held it reads on from the cursor.
#[test]
fn with_no_row_held_a_gap_reads_the_newest_page() {
    let cold = vec![ClientEvent::pull(Cursor::BeforeSeq(NEWEST), PAGE_LIMIT)];
    let mut chat = connected();
    assert_eq!(receive(&mut chat, row(40)), cold);
    let mut chat = connected();
    let bad = json!({"type": "row", "profile_id": "main", "server_seq": "forty"});
    assert_eq!(receive(&mut chat, bad), cold);
    let mut chat = connected();
    receive(&mut chat, started());
    let done = json!({"type": "text_done", "profile_id": "main", "turn_id": "t1",
        "server_seq": "forty"});
    assert_eq!(receive(&mut chat, done), cold);
    let mut chat = connected();
    receive(&mut chat, row(1));
    assert_eq!(
        receive(&mut chat, row(40)),
        vec![ClientEvent::pull(Cursor::AfterSeq(1), PAGE_LIMIT)]
    );
}

/// A resolution that cannot be read still settles the card it names, and only that one: its
/// placeholder goes, a held proposal's card says Fermix settled it, and a held approval goes with
/// its buttons. One naming another card leaves every card and placeholder as it was.
#[test]
fn an_unreadable_resolution_settles_the_card_it_names() {
    let (mut chat, id) = holding_a_card();
    receive(&mut chat, bad_proposal(Some("cpr_x")));
    let other = json!({"type": "proposal_resolved", "profile_id": "main",
        "proposal_id": "cpr_other", "state": 5});
    receive(&mut chat, other);
    assert_eq!(placeholders(&chat), 1, "another card's resolution");
    assert_eq!(chat.proposals[0].standing, Standing::Open);
    for card in [id.as_str(), "cpr_x"] {
        let bad = json!({"type": "proposal_resolved", "profile_id": "main",
            "proposal_id": card, "state": 5});
        assert_eq!(
            read(bad.clone()),
            unread("proposal_resolved", Some(card), None, None)
        );
        receive(&mut chat, bad);
    }
    assert_eq!(placeholders(&chat), 0);
    assert_eq!(
        chat.proposals[0].standing,
        Standing::Resolved {
            sentence: SETTLED_UNREAD.into(),
            result_text: None
        }
    );
    let (mut chat, id) = holding_an_approval();
    receive(&mut chat, bad_approval("apr_2"));
    let other = json!({"type": "approval_resolved", "profile_id": "main",
        "approval_id": "apr_other", "outcome": 5});
    receive(&mut chat, other);
    assert_eq!(chat.approvals.len(), 1, "another approval's resolution");
    assert!(unshown::waiting(&chat.unshown, Asks::Approval));
    for card in [id.as_str(), "apr_2"] {
        let bad = json!({"type": "approval_resolved", "profile_id": "main",
            "approval_id": card, "outcome": 5});
        assert_eq!(
            read(bad.clone()),
            unread("approval_resolved", Some(card), None, None)
        );
        receive(&mut chat, bad);
    }
    assert!(chat.approvals.is_empty());
    assert!(!unshown::waiting(&chat.unshown, Asks::Approval));
}

/// The first unreadable event of a type on a connection is warned of and the rest are not, so a
/// type that drifted cannot flood the log while a reply streams; a new connection warns again.
#[test]
fn each_type_of_unreadable_event_is_warned_of_once_a_connection() {
    let mut chat = connected();
    assert!(timeline::first_unreadable(&mut chat, "text_delta"));
    assert!(!timeline::first_unreadable(&mut chat, "text_delta"));
    assert!(timeline::first_unreadable(&mut chat, "tool_event"));
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert!(timeline::first_unreadable(&mut chat, "text_delta"));
}

#[test]
fn a_handshake_clears_the_approvals_placeholder_too() {
    let mut chat = connected();
    receive(&mut chat, bad_approval("apr_1"));
    receive(
        &mut chat,
        json!({"type": "approval", "profile_id": "main", "text": 5}),
    );
    timeline::disconnected(&mut chat);
    assert!(
        unshown::waiting(&chat.unshown, Asks::Approval),
        "kept while away"
    );
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert!(!unshown::waiting(&chat.unshown, Asks::Approval));
}

#[test]
fn an_unreadable_refusal_naming_no_request_is_the_chats_failure() {
    let mut chat = connected();
    timeline::send(&mut chat, "Is it done?", "linux-1".into());
    assert!(receive(&mut chat, bad_error(json!({}))).is_empty());
    assert_eq!(chat.last_error.as_deref(), Some(REFUSED_UNREAD));
    assert_eq!(
        chat.pending.len(),
        1,
        "the message still waits for its own answer"
    );
}

/// A refused answer to an approval ends that answer, so the card takes one again; an approval
/// card has no refusal of its own, so the chat says it.
#[test]
fn an_unreadable_refusal_of_an_approvals_answer_lets_the_card_be_answered_again() {
    let (mut chat, id) = holding_an_approval();
    assert_eq!(
        timeline::answer(&mut chat, &id, true, "linux-3".into()).len(),
        1
    );
    assert!(timeline::answering(&chat, &id));
    let refusal = bad_error(json!({"client_msg_id": "linux-3"}));
    assert!(receive(&mut chat, refusal).is_empty());
    assert!(!timeline::answering(&chat, &id));
    assert_eq!(chat.approvals.len(), 1, "the card stays");
    assert_eq!(chat.last_error.as_deref(), Some(REFUSED_UNREAD));
}

/// The approval strip says it waits for an answer only while it holds a card this app can
/// answer; holding only the placeholder, it says what it holds.
#[test]
fn the_approval_strip_asks_only_while_it_holds_a_card_it_can_answer() {
    assert_eq!(unshown::approvals_label(true), unshown::APPROVALS_ASK);
    assert_eq!(unshown::approvals_label(false), unshown::APPROVALS_UNSHOWN);
    assert!(!unshown::APPROVALS_UNSHOWN.contains("answer"));
    assert!(!unshown::APPROVALS_UNSHOWN.contains("Waiting"));
}
