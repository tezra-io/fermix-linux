//! The chat connection's life: the handshake's four outcomes (PROTOCOL.md "Handshake state
//! machine", macOS `CompanionSession`), the bounded backoff, and what the page says.

use fermix_client::chat_wire::protocol::{decode_line, ServerEvent};
use fermix_client::chat_wire::session::{
    closes, delay, handshake, Handshake, Lost, Phase, Session, Shown, HANDSHAKE_TIMEOUT,
};
use std::time::Duration;

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
