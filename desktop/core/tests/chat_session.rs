//! The chat connection's life: the handshake's four outcomes (PROTOCOL.md "Handshake state
//! machine", macOS `CompanionSession`), the bounded backoff, and what the page says.

use fermix_client::chat_wire::protocol::{decode_line, ServerEvent};
use fermix_client::chat_wire::session::{
    closes, delay, ends_with, handshake, Handshake, Lost, Phase, Session, Shown, HANDSHAKE_TIMEOUT,
};
use fermix_client::chat_wire::timeline::{self, Chat};
use std::collections::BTreeSet;
use std::time::Duration;

/// The engine's own lines for a computer whose home moved (its companion export's
/// `fixtures/server_events.jsonl`, M75 §6), which the vendored version 3 contract predates.
const UNREACHABLE: &str = r#"{"type":"error","reason":"primary_unreachable","message":"The server fermix-vm is unreachable. Fermix keeps your message until it is back."}"#;
const UNATTENDED: &str = r#"{"type":"error","reason":"unattended_client","message":"Only the Fermix app chats with the server from this computer, and this process was started by Fermix or runs detached."}"#;
/// What a desktop's engine answers every connection once the server no longer knows this
/// computer, verbatim from the engine's companion `fixtures/server_events.jsonl`.
const REVOKED: &str = r#"{"type":"error","reason":"desktop_revoked","message":"The server fermix-vm does not know this computer: it was removed from the server's devices, or the server was restored from a snapshot taken before this computer joined it. Fermix stopped reaching it."}"#;
/// What the engine answers a message while its home moves away (its companion `PROTOCOL.md`,
/// "Errors"): the message is refused as for an unreachable server, naming the message, and the
/// connection is closed.
const MOVING: &str = r#"{"type":"error","reason":"primary_unreachable","message":"Fermix is moving this home to north-box; your message is kept until it is done.","client_msg_id":"linux-1"}"#;

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");

fn server(index: usize) -> ServerEvent {
    decode_line(SERVER.lines().nth(index).unwrap().as_bytes()).unwrap()
}

fn line(text: &str) -> ServerEvent {
    decode_line(text.as_bytes()).unwrap()
}

fn handshaking() -> Session {
    let mut session = Session::default();
    assert!(session.start());
    session.connected();
    session
}

#[test]
fn a_hello_whose_window_holds_this_version_connects() {
    assert_eq!(handshake(&server(0)), Handshake::Ok);
    let mut session = handshaking();
    assert_eq!(session.hello(&server(0)), Handshake::Ok);
    assert_eq!(session.phase(), &Phase::Connected);
    assert_eq!(session.shown(), Shown::Connected);
}

#[test]
fn a_hello_whose_window_excludes_this_version_refuses_by_direction() {
    let newer = line(r#"{"type":"server_hello","min_version":4,"max_version":5}"#);
    let older = line(r#"{"type":"server_hello","min_version":1,"max_version":2}"#);
    let Handshake::Refused(app) = handshake(&newer) else {
        panic!("refused");
    };
    assert!(app.contains("Update the app"), "{app}");
    let Handshake::Refused(fermix) = handshake(&older) else {
        panic!("refused");
    };
    assert!(fermix.contains("Update the engine"), "{fermix}");
}

#[test]
fn a_version_refusal_stops_retrying_with_its_directional_sentence() {
    let mut session = handshaking();
    let Handshake::Refused(sentence) = session.hello(&server(35)) else {
        panic!("client_too_new refuses");
    };
    assert!(sentence.contains("Update the engine"), "{sentence}");
    assert_eq!(session.shown(), Shown::Refused(sentence));
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        None,
        "the close is not a failure"
    );
    assert!(matches!(session.phase(), Phase::Refused(_)));
    assert!(session.start(), "chat asked for again tries again");
    assert_eq!(session.phase(), &Phase::Connecting);
}

#[test]
fn other_errors_before_hello_are_lost_and_retried() {
    for index in [34, 39] {
        let mut session = handshaking();
        assert!(
            matches!(session.hello(&server(index)), Handshake::Lost(_)),
            "{index}"
        );
        let wait = session.lost(Lost::Broken("refused".into()));
        assert_eq!(wait, Some(Duration::from_secs(1)));
        assert!(matches!(session.phase(), Phase::Waiting));
        assert!(session.start());
    }
    let unidentified = line(r#"{"type":"error","reason":"unidentified_client","message":"who"}"#);
    assert!(matches!(handshake(&unidentified), Handshake::Lost(_)));
}

#[test]
fn any_other_event_before_hello_is_ignored() {
    let mut session = handshaking();
    assert_eq!(session.hello(&server(24)), Handshake::Ignore);
    assert_eq!(session.phase(), &Phase::Handshaking);
}

#[test]
fn the_backoff_doubles_to_thirty_seconds_and_stays_there() {
    let waits: Vec<u64> = (0..8).map(|n| delay(n).as_secs()).collect();
    assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(delay(u32::MAX), Duration::from_secs(30));
    assert_eq!(HANDSHAKE_TIMEOUT, Duration::from_secs(3));
}

#[test]
fn a_handshake_resets_the_backoff() {
    let mut session = handshaking();
    session.lost(Lost::Broken("eof".into()));
    session.start();
    session.connected();
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        Some(Duration::from_secs(2))
    );
    session.start();
    session.connected();
    session.hello(&server(0));
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        Some(Duration::from_secs(1))
    );
}

#[test]
fn the_page_says_why_it_is_not_connected() {
    let mut session = Session::default();
    assert_eq!(session.shown(), Shown::Idle);
    session.start();
    assert_eq!(session.shown(), Shown::Connecting);
    session.lost(Lost::NoSocket);
    assert_eq!(session.shown(), Shown::NoChat);
    session.start();
    assert_eq!(
        session.shown(),
        Shown::NoChat,
        "a retry keeps the last reason"
    );
    session.lost(Lost::Broken("eof".into()));
    assert!(matches!(session.shown(), Shown::Disconnected(_)));
}

#[test]
fn the_closing_errors_are_the_contracts() {
    for reason in [
        "invalid_json",
        "unknown_event",
        "invalid_field",
        "line_too_large",
        "max_clients_reached",
        "unsupported_protocol_version",
    ] {
        assert!(closes(reason), "{reason}");
    }
    for reason in [
        "client_message_conflict",
        "unsupported_profile",
        "request_backlog_full",
        "request_failed",
    ] {
        assert!(!closes(reason), "{reason}");
    }
}

#[test]
fn a_second_loss_while_waiting_counts_nothing() {
    let mut session = handshaking();
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        Some(Duration::from_secs(1))
    );
    assert_eq!(session.lost(Lost::Broken("eof".into())), None);
    session.start();
    session.connected();
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        Some(Duration::from_secs(2))
    );
}

#[test]
fn a_connect_that_lands_after_its_attempt_ended_is_refused() {
    let mut session = Session::default();
    session.start();
    session.lost(Lost::Broken("deadline".into()));
    assert!(
        !session.connected(),
        "the late socket is the caller's to close"
    );
    assert_eq!(session.phase(), &Phase::Waiting);
    session.start();
    assert!(session.connected());
}

#[test]
fn an_unreachable_server_is_said_in_its_sentence_and_tried_again() {
    let sentence =
        "The server fermix-vm is unreachable. Fermix keeps your message until it is back.";
    let mut session = handshaking();
    let Handshake::Lost(lost) = session.hello(&line(UNREACHABLE)) else {
        panic!("an unreachable server loses the attempt");
    };
    assert_eq!(lost, Lost::Said(sentence.into()));
    assert_eq!(session.lost(lost), Some(Duration::from_secs(1)));
    assert_eq!(session.shown(), Shown::Disconnected(sentence.into()));
    assert!(session.start(), "the next attempt runs after the backoff");
    assert_eq!(
        session.shown(),
        Shown::Disconnected(sentence.into()),
        "a retry keeps the sentence"
    );
    assert_eq!(handshake(&server(0)), Handshake::Ok);
}

#[test]
fn a_live_connection_ended_for_the_server_is_a_loss_not_a_refused_request() {
    let sentence =
        "The server fermix-vm is unreachable. Fermix keeps your message until it is back.";
    assert_eq!(
        ends_with(&line(UNREACHABLE)),
        Some(Lost::Said(sentence.into()))
    );
    let Some(Lost::Said(unattended)) = ends_with(&line(UNATTENDED)) else {
        panic!("an unattended client is told why");
    };
    assert!(
        unattended.starts_with("Only the Fermix app chats"),
        "{unattended}"
    );
    let failed = line(
        r#"{"type":"error","reason":"request_failed","message":"No.","client_msg_id":"linux-1"}"#,
    );
    assert_eq!(ends_with(&failed), None, "a refused request ends nothing");
    let unknown = line(r#"{"type":"error","reason":"unknown_event","event":"x"}"#);
    assert_eq!(ends_with(&unknown), None);
    for reason in ["primary_unreachable", "unattended_client"] {
        assert!(closes(reason), "{reason}");
    }
}

/// A message typed while the server is unreachable waits in the outbox and goes out after the
/// next handshake, as after any dropped connection.
#[test]
fn the_outbox_waits_out_an_unreachable_server() {
    let mut session = handshaking();
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    let Handshake::Lost(lost) = session.hello(&line(UNREACHABLE)) else {
        panic!("lost");
    };
    timeline::disconnected(&mut chat);
    assert!(session.lost(lost).is_some());
    assert!(timeline::send(&mut chat, "still there?", "linux-1".into()).is_empty());
    assert_eq!(chat.pending.len(), 1, "kept while away");
    assert!(session.start());
    assert!(session.connected());
    assert_eq!(session.hello(&server(0)), Handshake::Ok);
    let sent = timeline::negotiated(&mut chat, &BTreeSet::new());
    let lines: Vec<String> = sent.iter().map(|e| e.line().expect("a line")).collect();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("still there?") && l.contains("linux-1")),
        "{lines:?}"
    );
}

/// A message typed while the home moves is answered with the daemon's sentence that it is kept,
/// and it is: the refusal names it, yet it stays in the outbox and goes out again after the next
/// handshake (M75 review, edges F3).
#[test]
fn a_message_typed_while_the_home_moves_is_kept_and_sent_again() {
    let sentence =
        "Fermix is moving this home to north-box; your message is kept until it is done.";
    let mut session = handshaking();
    assert_eq!(session.hello(&server(0)), Handshake::Ok);
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    let sent = timeline::send(&mut chat, "did the move work?", "linux-1".into());
    assert_eq!(sent.len(), 1, "sent while connected");
    let Some(lost) = ends_with(&line(MOVING)) else {
        panic!("the refusal ends the connection rather than the message");
    };
    assert_eq!(lost, Lost::Said(sentence.into()));
    timeline::disconnected(&mut chat);
    assert!(session.lost(lost).is_some());
    assert_eq!(session.shown(), Shown::Disconnected(sentence.into()));
    assert_eq!(chat.pending.len(), 1, "kept, as the sentence says");
    assert!(session.start() && session.connected());
    assert_eq!(session.hello(&server(0)), Handshake::Ok);
    let again = timeline::negotiated(&mut chat, &BTreeSet::new());
    let lines: Vec<String> = again.iter().map(|e| e.line().expect("a line")).collect();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("did the move work?") && l.contains("linux-1")),
        "{lines:?}"
    );
}

/// A revoked desktop: its sentence is shown, chat stops trying, and nothing more is queued, since
/// nothing typed here can reach the server (M75 cleanup, item 5). An unreachable server keeps the
/// outbox as before.
#[test]
fn a_revoked_desktop_says_so_and_queues_nothing() {
    let sentence = "The server fermix-vm does not know this computer: it was removed from the \
        server's devices, or the server was restored from a snapshot taken before this computer \
        joined it. Fermix stopped reaching it.";
    assert!(closes("desktop_revoked"));
    let mut session = handshaking();
    assert_eq!(
        session.hello(&line(REVOKED)),
        Handshake::Refused(sentence.into())
    );
    assert_eq!(
        session.lost(Lost::Broken("closed".into())),
        None,
        "no retry"
    );
    assert_eq!(session.shown(), Shown::Refused(sentence.into()));
    assert!(!session.queues());

    let mut live = handshaking();
    assert_eq!(live.hello(&server(0)), Handshake::Ok);
    let lost = ends_with(&line(REVOKED)).expect("it ends the connection");
    assert_eq!(lost, Lost::Revoked(sentence.into()));
    assert_eq!(live.lost(lost), None, "no backoff, no retry");
    assert_eq!(live.shown(), Shown::Refused(sentence.into()));
    assert!(!live.queues());
    assert!(live.start(), "asked for again, chat tries once more");

    let mut away = handshaking();
    assert_eq!(away.hello(&server(0)), Handshake::Ok);
    let lost = ends_with(&line(UNREACHABLE)).expect("a loss");
    assert!(
        away.lost(lost).is_some(),
        "an unreachable server is retried"
    );
    assert!(away.queues(), "and what is typed waits for it");
}
