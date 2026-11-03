//! The chat connection's life: the handshake's four outcomes (PROTOCOL.md "Handshake state
//! machine", macOS `CompanionSession`), the bounded backoff, and what the page says.

use fermix_client::chat_wire::protocol::{read_line, Attachments, ServerEvent};
use fermix_client::chat_wire::session::{
    closes, delay, handshake, logged, Handshake, Lost, Phase, Session, Shown, HANDSHAKE_TIMEOUT,
    HISTORY_UNREAD, RECONNECTING,
};
use serde_json::{json, Value};
use std::time::Duration;

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");
const PROTOCOL: &str = include_str!("../contracts/companion/PROTOCOL.md");

fn server(index: usize) -> ServerEvent {
    line(SERVER.lines().nth(index).unwrap())
}

fn line(text: &str) -> ServerEvent {
    let event = read_line(text.as_bytes()).unwrap();
    assert!(
        !matches!(event, ServerEvent::Unreadable(_)),
        "a test event reads: {event:?}"
    );
    event
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
    let newer = line(r#"{"type":"server_hello","min_version":8,"max_version":9}"#);
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

/// PROTOCOL.md "Versioning": the engine serves 6..7, and there is no fallback below 7.
#[test]
fn this_build_speaks_7_on_the_6_to_7_window_and_a_window_ending_at_6_is_refused() {
    let current = line(r#"{"type":"server_hello","min_version":6,"max_version":7}"#);
    assert_eq!(handshake(&current), Handshake::Ok);
    let v0_17 = line(r#"{"type":"server_hello","min_version":3,"max_version":4}"#);
    let at_6 = line(r#"{"type":"server_hello","min_version":5,"max_version":6}"#);
    let Handshake::Refused(older) = handshake(&v0_17) else {
        panic!("a daemon whose window ends at 4 is refused");
    };
    let Handshake::Refused(sentence) = handshake(&at_6) else {
        panic!("a daemon whose window ends at 6 is refused");
    };
    assert_eq!(older, sentence);
    assert_eq!(
        sentence,
        "The Fermix engine is older than this app. Update the engine to chat."
    );
}

#[test]
fn a_version_refusal_stops_retrying_with_its_directional_sentence() {
    let mut session = handshaking();
    let Handshake::Refused(sentence) = session.hello(&server(59)) else {
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
    for index in [58, 64] {
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
    assert_eq!(session.hello(&server(29)), Handshake::Ignore);
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

/// A connection given up because its history could not be read keeps its backoff past the next
/// handshake, so a history that stays unreadable is tried less and less often, at most every 30 s;
/// the first loss of another kind after it waits that backoff once, and the next handshake resets
/// it.
#[test]
fn a_history_given_up_keeps_its_backoff_past_the_handshake() {
    let mut session = handshaking();
    session.hello(&server(0));
    assert_eq!(session.lost(Lost::History), Some(Duration::from_secs(1)));
    assert!(matches!(session.shown(), Shown::Disconnected(_)));
    for seconds in [2, 4, 8, 16, 30, 30] {
        session.start();
        session.connected();
        session.hello(&server(0));
        assert_eq!(
            session.lost(Lost::History),
            Some(Duration::from_secs(seconds))
        );
    }
    session.start();
    session.connected();
    session.hello(&server(0));
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        Some(Duration::from_secs(30))
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
    assert_eq!(session.shown(), Shown::Disconnected(RECONNECTING.into()));
    session.start();
    session.connected();
    session.hello(&server(0));
    session.lost(Lost::History);
    assert_eq!(
        session.shown(),
        Shown::Disconnected(HISTORY_UNREAD.into()),
        "Fermix answered: only its history could not be read"
    );
    assert_ne!(HISTORY_UNREAD, RECONNECTING);
}

/// The backoff kept for a history that could not be read ends with that episode: the first page
/// read on the next connection resets it, so a daemon whose history reads again is not waited
/// for 30 s at its next loss. A page read on a connection that kept no backoff changes nothing.
#[test]
fn a_page_read_after_a_history_given_up_ends_its_backoff() {
    let mut session = handshaking();
    session.hello(&server(0));
    for seconds in [1, 2, 4, 8, 16, 30] {
        assert_eq!(
            session.lost(Lost::History),
            Some(Duration::from_secs(seconds))
        );
        session.start();
        session.connected();
        session.hello(&server(0));
    }
    session.history_read();
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        Some(Duration::from_secs(1))
    );
    session.start();
    session.connected();
    session.history_read();
    assert_eq!(
        session.lost(Lost::Broken("eof".into())),
        Some(Duration::from_secs(2)),
        "a page read only ends a backoff kept on a connection"
    );
}

/// The log names a handshake event by its type, and an error by its reason and direction as code
/// words (`loggable_id`): never the daemon's sentence, nor a word outside the contract's
/// alphabet. A lost handshake carries the same into the log.
#[test]
fn a_handshake_event_is_logged_by_its_codes_alone() {
    let plain = line(
        r#"{"type":"error","reason":"unsupported_protocol_version","direction":"client_too_old","message":"Update this app."}"#,
    );
    assert_eq!(
        logged(&plain),
        "error unsupported_protocol_version, direction client_too_old"
    );
    let odd = line(
        r#"{"type":"error","reason":"a password: hunter2\nforged","direction":"sideways now","message":"secret"}"#,
    );
    let text = logged(&odd);
    for word in ["hunter2", "forged", "sideways", "secret", "\n"] {
        assert!(!text.contains(word), "{text}");
    }
    assert_eq!(logged(&server(0)), "server_hello");
    let lost = line(r#"{"type":"error","reason":"a password: hunter2","message":"secret"}"#);
    let Handshake::Lost(detail) = handshake(&lost) else {
        panic!("an error with no direction loses the handshake");
    };
    assert!(
        !detail.contains("hunter2") && !detail.contains("secret"),
        "{detail}"
    );
}

/// PROTOCOL.md "Errors": each reason in the table closes the connection exactly when its row says
/// `closed`, so a contract taken again that adds a closing reason fails here until this build knows
/// it.
#[test]
fn every_reason_in_the_contracts_errors_table_closes_as_its_row_says() {
    let section = PROTOCOL
        .split("\n## Errors\n")
        .nth(1)
        .expect("PROTOCOL.md has an Errors section");
    let table = section.trim_start().split("\n\n").next().unwrap();
    let rows: Vec<&str> = table.lines().skip(2).collect();
    assert!(rows.len() > 10, "the table was read: {table}");
    for row in rows {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let closed = cells[3].starts_with("closed");
        assert!(closed || cells[3] == "open", "closed or open: {row}");
        for reason in cells[1].split(", ") {
            assert_eq!(closes(reason.trim_matches('`')), closed, "{reason}");
        }
    }
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
        "stale_primary",
        "primary_epoch_unavailable",
        "unattended_client",
        "primary_unreachable",
        "desktop_revoked",
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

/// What a hello offers that this build does not use yet costs only itself: a command that does
/// not fit is skipped and counted, commands that are no list are all skipped, an attachments
/// offer that does not fit offers nothing, and the hello connects.
#[test]
fn a_hello_whose_offers_do_not_fit_still_connects() {
    let golden: Value = serde_json::from_str(SERVER.lines().next().unwrap()).unwrap();
    let offered = golden["commands"].as_array().expect("commands").len();
    let mut hello = golden.clone();
    let commands = hello["commands"].as_array_mut().unwrap();
    commands.push(json!({"name": "broken"}));
    hello["attachments"] = json!({"dir": 5});
    let event = line(&hello.to_string());
    let ServerEvent::ServerHello {
        commands,
        attachments,
        ..
    } = &event
    else {
        panic!("a hello: {event:?}");
    };
    assert_eq!((commands.offered.len(), commands.skipped), (offered, 1));
    assert_eq!(*attachments, Attachments::Unreadable);
    assert_eq!(handshake(&event), Handshake::Ok);
    hello["commands"] = json!("none");
    let event = line(&hello.to_string());
    let ServerEvent::ServerHello { commands, .. } = &event else {
        panic!("a hello: {event:?}");
    };
    assert_eq!((commands.offered.len(), commands.skipped), (0, 1));
    assert_eq!(handshake(&event), Handshake::Ok);
}
