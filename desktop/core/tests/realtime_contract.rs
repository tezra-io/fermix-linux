//! The vendored realtime contract: its bytes are the engine's, pinned by checksum, and every
//! golden line in it decodes (server) or round-trips (client) through the app's own types. A
//! drift in the engine fails here rather than on a call.

use fermix_client::realtime::protocol::{
    decode_line, CallReady, Caption, ClientEvent, Direction, ServerEvent, Speaker, Task,
    TaskStatus, ToolStatus, TurnState, PROTOCOL_VERSION,
};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const CHECKSUMS: &str = include_str!("../contracts/realtime/CHECKSUMS.txt");
const SCHEMA: &str = include_str!("../contracts/realtime/protocol.schema.json");
const CLIENT: &str = include_str!("../contracts/realtime/fixtures/client_events.jsonl");
const SERVER: &str = include_str!("../contracts/realtime/fixtures/server_events.jsonl");
const VENDORED: [&str; 4] = [
    "PROTOCOL.md",
    "protocol.schema.json",
    "fixtures/client_events.jsonl",
    "fixtures/server_events.jsonl",
];

fn contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/realtime")
}

fn schema() -> Value {
    serde_json::from_str(SCHEMA).expect("the vendored schema is JSON")
}

fn schema_types(side: &str) -> Vec<String> {
    let names = &schema()["$defs"][side]["properties"]["type"]["enum"];
    names
        .as_array()
        .unwrap_or_else(|| panic!("the schema lists the {side} types"))
        .iter()
        .map(|name| name.as_str().expect("a type name").to_owned())
        .collect()
}

fn fixture_types(lines: &str) -> Vec<String> {
    lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .map(|value| {
            value["type"]
                .as_str()
                .expect("fixture has a type")
                .to_owned()
        })
        .collect()
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

/// `sha256sum` is coreutils, on every Linux host and in the GNOME SDK. It is the same tool
/// `SOURCE.md` tells a re-vendor to run, so the check and the recipe cannot disagree.
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
    assert!((min..=max).contains(&u64::from(PROTOCOL_VERSION)));
}

#[test]
fn the_fixtures_cover_every_event_type_the_schema_lists() {
    let mut server = fixture_types(SERVER);
    server.sort();
    server.dedup();
    let mut listed = schema_types("serverEvent");
    listed.sort();
    assert_eq!(server, listed);

    let mut client = fixture_types(CLIENT);
    client.sort();
    client.dedup();
    let mut listed = schema_types("clientEvent");
    listed.sort();
    assert_eq!(client, listed);
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
fn the_golden_server_events_carry_their_fields() {
    assert_eq!(
        server(0),
        ServerEvent::ServerHello {
            min_version: 1,
            max_version: 2
        }
    );
    assert_eq!(
        server(1),
        ServerEvent::State {
            state: TurnState::Listening
        }
    );
    assert_eq!(
        server(2),
        ServerEvent::AudioDelta {
            audio: vec![0, 0, 0]
        }
    );
    assert_eq!(
        server(5),
        ServerEvent::ToolEvent {
            status: ToolStatus::Completed,
            name: None,
            reason: None
        }
    );
    assert_eq!(
        server(7),
        ServerEvent::CallReady(CallReady {
            engine: "openai_live".into(),
            call_id: "voice_live:17".into(),
            provider_session_id: Some("sess_live_01H9".into()),
            expires_at: Some(1_788_000_000),
            captions: true,
        })
    );
    assert_eq!(
        server(8),
        ServerEvent::Caption(Caption {
            speaker: Speaker::User,
            delta: "what is ".into(),
            start_ms: 1200,
            end_ms: 1640,
        })
    );
    assert_eq!(
        server(9),
        ServerEvent::Task(Task {
            delegation_id: "dg_01H9".into(),
            revision: 1,
            status: TaskStatus::Running,
            summary: Some("checking the calendar".into()),
        })
    );
    assert_eq!(server(14), ServerEvent::PlaybackStop);
}

#[test]
fn the_golden_usage_and_error_lines_keep_what_the_app_shows() {
    let ServerEvent::Usage(live) = server(10) else {
        panic!("line 10 is usage")
    };
    assert_eq!(live.voice_cost_cents, Some(5.35));
    assert_eq!(live.backend_cost.as_deref(), Some("unknown"));
    assert_eq!(live.accounting.as_deref(), Some("running"));

    let ServerEvent::Error(refusal) = server(13) else {
        panic!("line 13 is an error")
    };
    assert_eq!(refusal.reason, "unsupported_protocol_version");
    assert_eq!(refusal.kind.as_deref(), Some("update_required"));
    assert_eq!(refusal.direction, Some(Direction::ClientTooOld));
    assert_eq!(refusal.min_version, Some(2));
    assert_eq!(refusal.required_for.as_deref(), Some("openai_live"));
}
