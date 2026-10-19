//! The count of what reads each phone-side file, and the `media_fetch` and `media_release` it asks
//! for (macOS `MediaGrants`): a file is held only while something reads it.

use fermix_client::chat_wire::grants::{verify, Answer, Grants, Outcome};
use fermix_client::chat_wire::protocol::{decode_line, ClientEvent, Grant, ServerEvent};
use serde_json::json;
use std::path::{Path, PathBuf};

const NOTE: &str = "9879f6d3c9efefe8c4f4aabe9d3269762264be52794e96f62ab8ffa53b7fadec";
const LAMP: &str = "fe07114ad9867f226ff7c3f455645de3a1bde0a29509b3c5631768faba215a70";
const MEDIA: &str = "/Users/me/.fermix/companion/media";

fn online() -> Grants {
    let mut grants = Grants::new(PathBuf::from(MEDIA));
    assert!(grants.negotiated().is_empty(), "nothing is counted yet");
    grants
}

fn event(value: serde_json::Value) -> ServerEvent {
    decode_line(value.to_string().as_bytes()).expect("an event")
}

fn granted(r: &str) -> ServerEvent {
    event(
        json!({"type": "media_path", "ref": r, "path": format!("{MEDIA}/{r}.ogg"),
        "kind": "audio", "mime": "audio/ogg", "size_bytes": 1, "sha256": r}),
    )
}

/// A grant of `r` outside the media directory, which `verify` refuses.
fn refused_grant(r: &str) -> ServerEvent {
    event(
        json!({"type": "media_path", "ref": r, "path": format!("/etc/{r}.ogg"),
        "kind": "audio", "mime": "audio/ogg", "size_bytes": 1, "sha256": r}),
    )
}

/// A way a grant is broken, by name.
type Break<'a> = (&'a str, &'a dyn Fn(&mut Grant));

fn path_of(r: &str) -> PathBuf {
    Path::new(MEDIA).join(format!("{r}.ogg"))
}

#[test]
fn a_file_is_fetched_by_its_first_read_and_released_by_its_last() {
    let mut grants = online();
    let (first, answer, sent) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Later);
    assert_eq!(sent, [ClientEvent::media_fetch(NOTE)]);
    let (second, answer, sent) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Later);
    assert!(sent.is_empty(), "one fetch however many read it");
    let answered = grants.answered(&granted(NOTE)).expect("an answer");
    assert_eq!(answered.r#ref, NOTE);
    assert_eq!(answered.outcome, Outcome::Granted(path_of(NOTE)));
    assert_eq!(answered.tickets, [first, second]);
    assert!(answered.events.is_empty());
    assert!(answered.asked);
    let (_, answer, sent) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Now(Outcome::Granted(path_of(NOTE))));
    assert!(sent.is_empty());
    assert_eq!(grants.count(NOTE), 3);
    assert!(grants.release(NOTE).is_empty());
    assert!(grants.release(NOTE).is_empty());
    assert_eq!(grants.release(NOTE), [ClientEvent::media_release(NOTE)]);
    assert_eq!(grants.count(NOTE), 0);
    assert!(
        grants.release(NOTE).is_empty(),
        "a release of a ref not held is nothing"
    );
}

/// A read that ends before its answer sends nothing then; the grant that arrives is let go at
/// once, since nothing reads it (M5).
#[test]
fn a_grant_that_arrives_after_its_last_reader_left_is_released_at_once() {
    let mut grants = online();
    let (ticket, _, _) = grants.hold(NOTE);
    grants.forget(ticket, NOTE);
    assert!(grants.release(NOTE).is_empty(), "nothing is held yet");
    let answered = grants.answered(&granted(NOTE)).expect("an answer");
    assert!(answered.tickets.is_empty());
    assert_eq!(answered.events, [ClientEvent::media_release(NOTE)]);
    let (_, answer, sent) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Later, "a new read asks again");
    assert_eq!(sent, [ClientEvent::media_fetch(NOTE)]);
}

/// The daemon holds a file it granted for this connection, asked or not: let it go.
#[test]
fn a_grant_never_asked_for_is_released_and_still_reported() {
    let mut grants = online();
    let answered = grants.answered(&granted(LAMP)).expect("an answer");
    assert!(!answered.asked);
    assert_eq!(answered.events, [ClientEvent::media_release(LAMP)]);
    let missing = event(json!({"type": "media_missing", "ref": LAMP}));
    assert!(grants
        .answered(&missing)
        .expect("an answer")
        .events
        .is_empty());
}

/// The daemon holds what it granted until a release or the close, whether or not the grant is
/// one this app takes (PROTOCOL rules 2 and 3): a refused grant is let go at once, read or not,
/// and once only.
#[test]
fn a_refused_grant_is_still_released_while_read() {
    let mut grants = online();
    let (ticket, _, _) = grants.hold(NOTE);
    let answered = grants.answered(&refused_grant(NOTE)).expect("an answer");
    assert!(
        matches!(&answered.outcome, Outcome::Failed(why) if why.starts_with("broken grant")),
        "{:?}",
        answered.outcome
    );
    assert_eq!(answered.tickets, [ticket]);
    assert_eq!(answered.events, [ClientEvent::media_release(NOTE)]);
    let (_, answer, sent) = grants.hold(NOTE);
    assert!(matches!(answer, Answer::Now(Outcome::Failed(_))));
    assert!(sent.is_empty());
    assert!(grants.release(NOTE).is_empty());
    assert!(grants.release(NOTE).is_empty(), "released once, not again");
}

#[test]
fn a_refused_grant_nobody_reads_is_released_at_once() {
    let mut grants = online();
    let (ticket, _, _) = grants.hold(NOTE);
    grants.forget(ticket, NOTE);
    assert!(grants.release(NOTE).is_empty(), "nothing is held yet");
    let answered = grants.answered(&refused_grant(NOTE)).expect("an answer");
    assert!(matches!(answered.outcome, Outcome::Failed(_)));
    assert_eq!(answered.events, [ClientEvent::media_release(NOTE)]);
    let unasked = grants.answered(&refused_grant(LAMP)).expect("an answer");
    assert_eq!(unasked.events, [ClientEvent::media_release(LAMP)]);
}

/// `verify` compares a grant's folder with the media directory, so a relative one would refuse
/// every grant without a word.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "absolute")]
fn the_media_directory_is_absolute() {
    Grants::new(PathBuf::from("companion/media"));
}

#[test]
fn a_missing_file_answers_every_reader_and_every_later_one() {
    let mut grants = online();
    let (first, _, _) = grants.hold(NOTE);
    let (second, _, _) = grants.hold(NOTE);
    grants.forget(first, NOTE);
    let answered = grants
        .answered(&event(json!({"type": "media_missing", "ref": NOTE})))
        .expect("an answer");
    assert_eq!(answered.outcome, Outcome::Missing);
    assert_eq!(
        answered.tickets,
        [second],
        "a forgotten reader is not answered"
    );
    let (_, answer, sent) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Now(Outcome::Missing));
    assert!(sent.is_empty());
    assert!(grants.release(NOTE).is_empty());
    assert!(grants.release(NOTE).is_empty());
    assert!(
        grants.release(NOTE).is_empty(),
        "nothing was granted to release"
    );
    assert_eq!(grants.count(NOTE), 0);
}

/// A daemon that could not make the file says why, with the ref; an error without a ref is a
/// turn's, not a file's.
#[test]
fn a_failed_copy_answers_its_readers_and_other_errors_are_not_media() {
    let mut grants = online();
    let (ticket, _, _) = grants.hold(LAMP);
    let failed = event(json!({"type": "error", "reason": "request_failed",
        "message": "{:media_copy_failed, :enospc}", "ref": LAMP}));
    let answered = grants.answered(&failed).expect("an answer");
    assert_eq!(
        answered.outcome,
        Outcome::Failed("{:media_copy_failed, :enospc}".into())
    );
    assert_eq!(answered.tickets, [ticket]);
    let turn = event(json!({"type": "error", "reason": "request_failed", "message": "x"}));
    assert_eq!(grants.answered(&turn), None);
    let other = event(json!({"type": "error", "reason": "invalid_field", "ref": LAMP}));
    assert_eq!(grants.answered(&other), None);
    assert_eq!(
        grants.answered(&event(json!({"type": "turn_done", "turn_id": "t"}))),
        None
    );
}

/// A connection's grants end with it: what is still read is asked again after the next handshake,
/// and a read while there is no connection waits for it.
#[test]
fn every_ref_still_read_is_fetched_again_after_a_reconnect() {
    let mut grants = Grants::new(PathBuf::from(MEDIA));
    let (_, answer, sent) = grants.hold(NOTE);
    assert_eq!(answer, Answer::Later);
    assert!(sent.is_empty(), "no connection to ask on");
    assert_eq!(grants.negotiated(), [ClientEvent::media_fetch(NOTE)]);
    grants.answered(&granted(NOTE)).expect("an answer");
    grants.hold(LAMP);
    grants.disconnected();
    let (_, answer, sent) = grants.hold(NOTE);
    assert_eq!(
        answer,
        Answer::Later,
        "the path may be gone with the connection"
    );
    assert!(sent.is_empty());
    assert_eq!(
        grants.negotiated(),
        [
            ClientEvent::media_fetch(NOTE),
            ClientEvent::media_fetch(LAMP)
        ]
    );
    assert!(grants.negotiated().is_empty(), "each asked once");
    let answered = grants.answered(&granted(NOTE)).expect("an answer");
    assert_eq!(
        answered.tickets.len(),
        1,
        "the read that waited is answered"
    );
}

#[test]
fn a_ref_that_is_not_one_is_missing_and_asks_nothing() {
    let mut grants = online();
    for not_a_ref in ["lamp.jpg", &NOTE[1..], &NOTE.to_uppercase(), ""] {
        let (_, answer, sent) = grants.hold(not_a_ref);
        assert_eq!(answer, Answer::Now(Outcome::Missing), "{not_a_ref}");
        assert!(sent.is_empty());
        assert_eq!(grants.count(not_a_ref), 0);
        assert!(grants.release(not_a_ref).is_empty());
    }
}

/// A grant is a file of this home's media directory named by its ref, whose digest is its ref:
/// anything else is refused, never followed.
#[test]
fn a_grant_is_a_file_named_by_its_ref_in_the_media_directory() {
    const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");
    let fixture = SERVER
        .lines()
        .find_map(|line| match decode_line(line.as_bytes()) {
            Ok(ServerEvent::MediaPath(grant)) => Some(grant),
            _ => None,
        })
        .expect("a media_path fixture");
    let media = Path::new(MEDIA);
    assert_eq!(verify(&fixture, media), Ok(path_of(NOTE)));
    assert_eq!(
        verify(&fixture, Path::new(&format!("{MEDIA}/"))),
        Ok(path_of(NOTE))
    );
    let broken = |change: &dyn Fn(&mut Grant)| {
        let mut grant = fixture.clone();
        change(&mut grant);
        verify(&grant, media)
    };
    let refused: [Break; 9] = [
        ("digest", &|g: &mut Grant| g.sha256 = LAMP.into()),
        ("uppercase", &|g: &mut Grant| {
            g.r#ref = NOTE.to_uppercase();
            g.sha256 = NOTE.to_uppercase();
        }),
        ("relative", &|g: &mut Grant| {
            g.path = format!("companion/media/{NOTE}.ogg")
        }),
        ("dot-dot", &|g: &mut Grant| {
            g.path = format!("{MEDIA}/../media/{NOTE}.ogg")
        }),
        ("dot", &|g: &mut Grant| {
            g.path = format!("{MEDIA}/./{NOTE}.ogg")
        }),
        ("deeper", &|g: &mut Grant| {
            g.path = format!("{MEDIA}/x/{NOTE}.ogg")
        }),
        ("stem", &|g: &mut Grant| {
            g.path = format!("{MEDIA}/{LAMP}.ogg")
        }),
        ("up", &|g: &mut Grant| {
            g.path = format!("/Users/me/.fermix/companion/{NOTE}.ogg")
        }),
        ("elsewhere", &|g: &mut Grant| {
            g.path = format!("/tmp/{NOTE}.ogg")
        }),
    ];
    for (why, change) in refused {
        assert!(broken(change).is_err(), "{why}");
    }
    let mut grants = online();
    grants.hold(NOTE);
    let mut lying = fixture.clone();
    lying.path = format!("/etc/{NOTE}.ogg");
    let answered = grants
        .answered(&ServerEvent::MediaPath(lying))
        .expect("an answer");
    assert!(
        matches!(&answered.outcome, Outcome::Failed(why) if why.starts_with("broken grant")),
        "{:?}",
        answered.outcome
    );
}
