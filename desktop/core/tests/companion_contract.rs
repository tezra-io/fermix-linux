//! The vendored companion chat contract: its bytes are the engine's, pinned by checksum, and every
//! golden line in it decodes (server) or round-trips (client) through the app's own types. A drift
//! in the engine fails here rather than in the chat.

use fermix_client::chat_wire::protocol::{
    decode_line, Block, ClientEvent, Cursor, Direction, Grant, Reply, ServerEvent, TaskRef,
    ToolPhase, MAX_LINE_BYTES, PROTOCOL_VERSION,
};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const CHECKSUMS: &str = include_str!("../contracts/companion/CHECKSUMS.txt");
const SCHEMA: &str = include_str!("../contracts/companion/protocol.schema.json");
const CLIENT: &str = include_str!("../contracts/companion/fixtures/client_events.jsonl");
const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");
const VENDORED: [&str; 4] = [
    "PROTOCOL.md",
    "protocol.schema.json",
    "fixtures/client_events.jsonl",
    "fixtures/server_events.jsonl",
];

fn contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/companion")
}

fn schema() -> Value {
    serde_json::from_str(SCHEMA).expect("the vendored schema is JSON")
}

fn schema_types(side: &str) -> Vec<String> {
    let names = &schema()["$defs"][side]["properties"]["type"]["enum"];
    let mut types: Vec<String> = names
        .as_array()
        .unwrap_or_else(|| panic!("the schema lists the {side} types"))
        .iter()
        .map(|name| name.as_str().expect("a type name").to_owned())
        .collect();
    types.sort();
    types
}

fn fixture_types(lines: &str) -> Vec<String> {
    let mut types: Vec<String> = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .map(|value| value["type"].as_str().expect("a type").to_owned())
        .collect();
    types.sort();
    types.dedup();
    types
}

fn server(index: usize) -> ServerEvent {
    let line = SERVER
        .lines()
        .nth(index)
        .expect("the fixture has this line");
    decode_line(line.as_bytes()).expect("a golden server line decodes")
}

#[test]
fn the_checksums_name_exactly_the_four_vendored_files() {
    let listed: Vec<&str> = CHECKSUMS
        .lines()
        .map(|line| line.split_once("  ").expect("sha256sum format").1)
        .collect();
    assert_eq!(listed, VENDORED);
}

/// `sha256sum` is coreutils, on every Linux host and in the GNOME SDK, and the tool `SOURCE.md`
/// tells a re-vendor to run.
#[test]
fn every_vendored_file_matches_its_recorded_checksum() {
    let out = Command::new("sha256sum")
        .args(["--check", "--strict", "CHECKSUMS.txt"])
        .current_dir(contract_dir())
        .output()
        .expect("sha256sum (coreutils) runs");
    assert!(
        out.status.success(),
        "a vendored file no longer matches CHECKSUMS.txt:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_app_speaks_the_version_the_schema_publishes_inside_its_window() {
    let schema = schema();
    assert_eq!(schema["x-protocol-version"], PROTOCOL_VERSION);
    let range = &schema["x-supported-version-range"];
    let (min, max) = (
        range["min"].as_u64().unwrap(),
        range["max"].as_u64().unwrap(),
    );
    assert!((min..=max).contains(&PROTOCOL_VERSION));
    assert_eq!(schema["x-max-line-bytes"], MAX_LINE_BYTES as u64);
}

#[test]
fn the_fixtures_cover_every_event_type_the_schema_lists() {
    assert_eq!(fixture_types(SERVER), schema_types("serverEvent"));
    assert_eq!(fixture_types(CLIENT), schema_types("clientEvent"));
}

#[test]
fn every_golden_server_line_decodes_into_a_known_event() {
    for line in SERVER.lines() {
        let event = decode_line(line.as_bytes())
            .unwrap_or_else(|e| panic!("golden line failed to decode: {line}: {e:?}"));
        assert!(
            !matches!(event, ServerEvent::Unknown(_)),
            "golden line decoded as unknown: {line}"
        );
    }
}

#[test]
fn every_golden_client_line_round_trips_to_equal_json() {
    for line in CLIENT.lines() {
        let event: ClientEvent = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("golden line is not a client event: {line}: {e}"));
        let written = event.line().expect("a golden event fits a line");
        assert!(written.ends_with('\n'), "a line ends with a newline");
        assert_eq!(written.matches('\n').count(), 1, "one line per event");
        let back: Value = serde_json::from_str(&written).unwrap();
        let golden: Value = serde_json::from_str(line).unwrap();
        assert_eq!(back, golden, "{line}");
    }
}

#[test]
fn the_golden_client_lines_are_the_app_constructors() {
    let lines: Vec<&str> = CLIENT.lines().collect();
    let same = |event: ClientEvent, index: usize| {
        let golden: Value = serde_json::from_str(lines[index]).unwrap();
        let written: Value = serde_json::from_str(&event.line().unwrap()).unwrap();
        assert_eq!(written, golden, "line {index}");
    };
    same(ClientEvent::hello(), 0);
    same(
        ClientEvent::msg("mac-1", "What is on my calendar today?"),
        1,
    );
    same(
        ClientEvent::command("mac-2", "confirm", Some("opaque-token")),
        2,
    );
    same(ClientEvent::cancel("mac-3"), 3);
    same(ClientEvent::pull(Cursor::AfterSeq(0), 50), 5);
    same(ClientEvent::pull(Cursor::BeforeSeq(120), 50), 6);
    same(ClientEvent::read_state(13), 9);
    let fetch = "9879f6d3c9efefe8c4f4aabe9d3269762264be52794e96f62ab8ffa53b7fadec";
    same(ClientEvent::media_fetch(fetch), 10);
    same(ClientEvent::media_release(fetch), 11);
}

#[test]
fn the_task_ref_cancel_and_the_search_decode_though_the_app_never_sends_them() {
    let lines: Vec<&str> = CLIENT.lines().collect();
    let cancel: ClientEvent = serde_json::from_str(lines[4]).unwrap();
    let ClientEvent::Cancel { task_ref, .. } = cancel else {
        panic!("line 4 is a cancel");
    };
    assert_eq!(
        task_ref,
        Some(TaskRef {
            call_uuid: "3f2b8c1e-5a4d-4e6f-9b8a-7c6d5e4f3a2b".into(),
            task_id: "dg_01HA".into(),
            revision: 1,
        })
    );
    let search: ClientEvent = serde_json::from_str(lines[8]).unwrap();
    assert!(matches!(
        search,
        ClientEvent::HistorySearch {
            before_seq: Some(88),
            ..
        }
    ));
}

#[test]
fn the_golden_server_events_carry_their_fields() {
    assert_eq!(
        server(0),
        ServerEvent::ServerHello {
            min_version: 2,
            max_version: 3
        }
    );
    let ServerEvent::ToolEvent { phase, detail, .. } = server(7) else {
        panic!("line 7 is a tool event");
    };
    assert_eq!(phase, ToolPhase::Stop);
    assert_eq!(detail.as_deref(), Some("%{status: :ok}"));
    assert_eq!(
        server(12),
        ServerEvent::TurnDone {
            turn_id: "turn-mac-5".into()
        }
    );
    assert_eq!(server(20), ServerEvent::RowUpdated { server_seq: 21 });
}

#[test]
fn the_places_reply_carries_its_places_in_order() {
    let ServerEvent::TextDone(Reply {
        server_seq, blocks, ..
    }) = server(9)
    else {
        panic!("line 9 is a text_done");
    };
    assert_eq!(server_seq, 24);
    let [Block::Places(places)] = blocks.as_slice() else {
        panic!("one places block");
    };
    let names: Vec<&str> = places.iter().map(|place| place.name.as_str()).collect();
    assert_eq!(names, ["Example Coffee", "Quiet Grounds"]);
    let first = &places[0];
    assert_eq!(
        first.canonical_url.as_deref(),
        Some("https://example.coffee/")
    );
    assert_eq!(first.location.latitude, 40.7233);
    assert_eq!(first.categories, ["coffee_shop"]);
    assert_eq!(
        first
            .opening_hours
            .as_ref()
            .and_then(|h| h.current_day.as_deref()),
        Some("Thursday 07:00-19:00")
    );
}

#[test]
fn the_phone_rows_carry_their_media_and_previews() {
    let ServerEvent::Row(voice) = server(18) else {
        panic!("line 18 is the voice note's row");
    };
    assert_eq!(voice.server_seq, 21);
    assert_eq!(voice.media_refs.len(), 1);
    assert_eq!(voice.media_refs[0].mime, "audio/ogg");
    let ServerEvent::Row(lamp) = server(19) else {
        panic!("line 19 is the lamp's row");
    };
    assert_eq!(lamp.media_refs[0].filename.as_deref(), Some("lamp.jpg"));
    assert_eq!(
        lamp.link_previews[0].link.as_deref(),
        Some("https://shop.test/lamp")
    );
    assert!(lamp.link_previews[0].image_ref.is_some());
}

#[test]
fn a_history_message_reads_its_content_as_the_row_text() {
    let ServerEvent::HistoryPage(page) = server(25) else {
        panic!("line 25 is a page");
    };
    assert_eq!(page.history_head_seq, 13);
    assert_eq!(page.next_after_seq, Some(13));
    assert_eq!(page.skipped, 0);
    assert_eq!(page.messages[1].text, "You have one meeting, at 10.");
    assert_eq!(page.messages[1].in_reply_to.as_deref(), Some("mac-1"));
}

#[test]
fn the_media_answers_and_the_errors_decode() {
    let ServerEvent::MediaPath(Grant { path, sha256, .. }) = server(32) else {
        panic!("line 32 is a grant");
    };
    assert!(path.ends_with(".ogg"));
    assert_eq!(sha256.len(), 64);
    let ServerEvent::Error(refusal) = server(35) else {
        panic!("line 35 is a version refusal");
    };
    assert_eq!(refusal.direction, Some(Direction::ClientTooNew));
    assert_eq!(refusal.max_version, Some(3));
    let ServerEvent::Error(media) = server(40) else {
        panic!("line 40 is a media failure");
    };
    assert!(media.r#ref.is_some());
}
