//! The vendored companion chat contract: its bytes are the engine's, pinned by checksum, and every
//! golden line in it decodes (server) or round-trips (client) through the app's own types. A drift
//! in the engine fails here rather than in the chat.

use fermix_client::chat_wire::protocol::{
    read_line, Ask, Attachments, Block, ClientEvent, Cursor, Decision, Direction, Grant, Inbox,
    ListOp, ProposalState, Reply, ServerEvent, TaskRef, ToolPhase, MAX_LINE_BYTES,
    PROTOCOL_VERSION,
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
    let event = read_line(line.as_bytes()).expect("a golden server line decodes");
    assert!(
        !matches!(event, ServerEvent::Unreadable(_)),
        "a golden server line reads: {event:?}"
    );
    event
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
        let event = read_line(line.as_bytes())
            .unwrap_or_else(|e| panic!("golden line failed to decode: {line}: {e:?}"));
        assert!(
            !matches!(event, ServerEvent::Unknown(_) | ServerEvent::Unreadable(_)),
            "golden line decoded as unknown or unreadable: {line}"
        );
        let golden: Value = serde_json::from_str(line).unwrap();
        assert_eq!(Some(event.kind()), golden["type"].as_str(), "{line}");
        assert_eq!(parts_skipped(&event), 0, "golden line lost a part: {line}");
    }
}

/// The parts a hello, a row, a reply or a page dropped in decoding: none of a golden line's may
/// be. A hello drops the commands it cannot read, and an offer of attachments it cannot read.
fn parts_skipped(event: &ServerEvent) -> usize {
    match event {
        ServerEvent::ServerHello {
            commands,
            attachments,
            ..
        } => commands.skipped + usize::from(*attachments == Attachments::Unreadable),
        ServerEvent::Row(row) => row.skipped,
        ServerEvent::TextDone(reply) => reply.skipped,
        ServerEvent::HistoryPage(page) => {
            page.skipped + page.messages.iter().map(|row| row.skipped).sum::<usize>()
        }
        _ => 0,
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
        2,
    );
    same(
        ClientEvent::command("mac-2", "confirm", Some("opaque-token")),
        3,
    );
    same(ClientEvent::cancel("mac-3"), 4);
    same(ClientEvent::pull(Cursor::AfterSeq(0), 50), 6);
    same(ClientEvent::pull(Cursor::BeforeSeq(120), 50), 7);
    same(ClientEvent::read_state(13), 10);
    let fetch = "9879f6d3c9efefe8c4f4aabe9d3269762264be52794e96f62ab8ffa53b7fadec";
    same(ClientEvent::media_fetch(fetch), 11);
    same(ClientEvent::media_release(fetch), 12);
    let proposal = "cpr_4f1c2a9b7e3d5a60c8b1f2e3";
    same(
        ClientEvent::proposal_answer("mac-10", proposal, Decision::Confirm),
        13,
    );
    let other = "cpr_9a0b1c2d3e4f5a6b7c8d9e0f";
    same(
        ClientEvent::proposal_answer("mac-11", other, Decision::Deny),
        14,
    );
    let follow_up = "cft_5d2e8a1c9b4f7e3a6c0d1b2e";
    same(
        ClientEvent::proposal_answer("mac-12", follow_up, Decision::AlwaysAllow),
        36,
    );
}

#[test]
fn the_client_events_the_app_does_not_send_yet_decode() {
    let lines: Vec<&str> = CLIENT.lines().collect();
    let client = |index: usize| -> ClientEvent { serde_json::from_str(lines[index]).unwrap() };
    let ClientEvent::Cancel { task_ref, .. } = client(5) else {
        panic!("line 5 is a cancel");
    };
    assert_eq!(
        task_ref,
        Some(TaskRef {
            call_uuid: "3f2b8c1e-5a4d-4e6f-9b8a-7c6d5e4f3a2b".into(),
            task_id: "dg_01HA".into(),
            revision: 1,
        })
    );
    assert!(matches!(
        client(9),
        ClientEvent::HistorySearch {
            before_seq: Some(88),
            ..
        }
    ));
    assert!(matches!(
        client(1),
        ClientEvent::ClientHello {
            primary_epoch: Some(2),
            ..
        }
    ));
    assert_eq!(
        client(15),
        ClientEvent::ChecklistsPull {
            profile_id: "main".into()
        }
    );
    assert!(matches!(
        client(37),
        ClientEvent::AttachFile { name: Some(_), .. }
    ));
    let ClientEvent::Msg {
        text, attach_ids, ..
    } = client(40)
    else {
        panic!("line 40 is a msg");
    };
    assert_eq!((text.as_str(), attach_ids.len()), ("", 1));
}

#[test]
fn every_list_edit_carries_its_op_and_keeps_an_empty_note_or_folder() {
    let lines: Vec<&str> = CLIENT.lines().collect();
    let ops: Vec<ListOp> = (18..=35)
        .map(|index| match serde_json::from_str(lines[index]).unwrap() {
            ClientEvent::ChecklistEdit { op, .. } => op,
            other => panic!("line {index} is a list edit: {other:?}"),
        })
        .collect();
    assert!(matches!(&ops[0], ListOp::Create { folder: Some(f), .. } if f == "Trips"));
    assert!(matches!(&ops[5], ListOp::Edit { note: Some(n), .. } if n.is_empty()));
    assert!(matches!(&ops[11], ListOp::Folder { folder } if folder.is_empty()));
    assert_eq!(ops[13], ListOp::ClearDone);
    assert_eq!(
        ops[15],
        ListOp::Restore {
            row_id: None,
            title: None
        }
    );
    assert_eq!(ops[17], ListOp::RemoveList);
}

#[test]
fn the_golden_server_events_carry_their_fields() {
    let ServerEvent::ServerHello {
        min_version,
        max_version,
        primary_epoch,
        role,
        commands,
        attachments,
    } = server(0)
    else {
        panic!("line 0 is a hello");
    };
    assert_eq!((min_version, max_version), (6, 7));
    assert_eq!((primary_epoch, role.as_deref()), (Some(2), Some("primary")));
    assert_eq!(commands.offered[0].name, "new");
    assert_eq!(commands.offered[0].aliases, ["clear"]);
    assert_eq!(commands.skipped, 0);
    assert_eq!(attachments, Attachments::Absent);
    let ServerEvent::ServerHello { attachments, .. } = server(1) else {
        panic!("line 1 is a hello");
    };
    assert_eq!(
        attachments,
        Attachments::Offered(Inbox {
            dir: "/Users/me/.fermix/companion/inbox".into(),
            max_bytes: 20_971_520
        })
    );
    let ServerEvent::ToolEvent { activity, .. } = server(8) else {
        panic!("line 8 is a tool event");
    };
    assert_eq!(activity.as_deref(), Some("Searching the web"));
    let ServerEvent::ToolEvent { phase, detail, .. } = server(9) else {
        panic!("line 9 is a tool event");
    };
    assert_eq!(phase, ToolPhase::Stop);
    assert_eq!(detail.as_deref(), Some("%{status: :ok}"));
    assert!(matches!(server(7), ServerEvent::Reasoning { .. }));
    assert_eq!(
        server(15),
        ServerEvent::TurnDone {
            turn_id: "turn-mac-5".into()
        }
    );
    assert_eq!(server(25), ServerEvent::RowUpdated { server_seq: 21 });
    assert_eq!(server(49), ServerEvent::HomeStatusChanged);
    assert_eq!(
        server(74),
        ServerEvent::AttachStatus {
            attach_id: "mac-att-7f3a9c".into(),
            status: "present".into()
        }
    );
}

#[test]
fn the_places_reply_carries_its_places_in_order() {
    let ServerEvent::TextDone(Reply {
        server_seq, blocks, ..
    }) = server(11)
    else {
        panic!("line 11 is a text_done");
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
    let ServerEvent::Row(voice) = server(21) else {
        panic!("line 21 is the voice note's row");
    };
    assert_eq!(voice.server_seq, 21);
    assert_eq!(voice.media_refs.len(), 1);
    assert_eq!(voice.media_refs[0].mime, "audio/ogg");
    let ServerEvent::Row(lamp) = server(22) else {
        panic!("line 22 is the lamp's row");
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
    let ServerEvent::HistoryPage(page) = server(30) else {
        panic!("line 30 is a page");
    };
    assert_eq!(page.history_head_seq, 13);
    assert_eq!(page.next_after_seq, Some(13));
    assert_eq!(page.skipped, 0);
    assert_eq!(page.messages[1].text, "You have one meeting, at 10.");
    assert_eq!(page.messages[1].in_reply_to.as_deref(), Some("mac-1"));
}

#[test]
fn the_media_answers_and_the_errors_decode() {
    let ServerEvent::MediaPath(Grant { path, sha256, .. }) = server(38) else {
        panic!("line 38 is a grant");
    };
    assert!(path.ends_with(".ogg"));
    assert_eq!(sha256.len(), 64);
    let ServerEvent::Error(refusal) = server(59) else {
        panic!("line 59 is a version refusal");
    };
    assert_eq!(refusal.direction, Some(Direction::ClientTooNew));
    assert_eq!(refusal.max_version, Some(7));
    let ServerEvent::Error(media) = server(65) else {
        panic!("line 65 is a media failure");
    };
    assert!(media.r#ref.is_some());
    let reasons: Vec<String> = [60, 71, 72, 73]
        .into_iter()
        .map(|index| match server(index) {
            ServerEvent::Error(error) => error.reason,
            other => panic!("line {index} is an error: {other:?}"),
        })
        .collect();
    let moved = [
        "stale_primary",
        "primary_unreachable",
        "desktop_revoked",
        "unattended_client",
    ];
    assert_eq!(reasons, moved);
    let ServerEvent::Error(file) = server(76) else {
        panic!("line 76 is a refused file");
    };
    assert_eq!(file.attach_id.as_deref(), Some("mac-att-9d04e2"));
}

#[test]
fn the_golden_proposals_carry_their_record_and_their_outside_text() {
    let ServerEvent::Proposal { proposal: full } = server(40) else {
        panic!("line 40 is a proposal");
    };
    assert_eq!(full.proposal_id, "cpr_4f1c2a9b7e3d5a60c8b1f2e3");
    assert_eq!(full.state, ProposalState::Waiting);
    let Ask::Call {
        mandate,
        tool,
        args_text,
    } = &full.ask
    else {
        panic!("line 40 asks for a call");
    };
    assert_eq!(
        (mandate.as_str(), tool.as_str()),
        ("release-issues", "github_create_issue")
    );
    assert!(args_text.contains("\"title\": \"Phoenix 2.0\""));
    assert_eq!(full.task.name.as_deref(), Some("release issues"));
    assert_eq!(full.watcher.as_deref(), Some("release-watch"));
    assert_eq!(full.source_ref.as_deref(), Some("eng-blogs"));
    assert_eq!(full.outside.args[3].name, "title");
    assert!(full.outside.reason.is_some());
    let cue = full.outside.cue.expect("the first card keeps its cue");
    assert_eq!(cue.r#type.as_deref(), Some("release"));
    assert_eq!(cue.occurred_at.as_deref(), Some("2026-10-10T11:58:30Z"));
    let ServerEvent::Proposal { proposal: bare } = server(41) else {
        panic!("line 41 is a proposal");
    };
    assert_eq!(bare.task.name, None);
    assert_eq!(
        (bare.watcher, bare.source_ref, bare.received_at),
        (None, None, None)
    );
    assert_eq!(bare.outside.cue, None);
}

#[test]
fn the_golden_follow_up_card_reads_as_a_follow_up() {
    let ServerEvent::Proposal { proposal } = server(42) else {
        panic!("line 42 is a proposal");
    };
    assert_eq!(proposal.proposal_id, "cft_5d2e8a1c9b4f7e3a6c0d1b2e");
    let Ask::FollowUp(follow_up) = proposal.ask else {
        panic!("line 42 asks for a follow-up");
    };
    assert_eq!(follow_up.asks, "run");
    assert!(follow_up.offers_always_allow);
    assert_eq!(follow_up.name, "Codex review");
    assert!(!follow_up.tools.is_empty());
    assert!(proposal.outside.args.is_empty());
}

#[test]
fn the_golden_resolutions_and_refused_answers_decode() {
    let ServerEvent::ProposalResolved(confirmed) = server(44) else {
        panic!("line 44 is a resolution");
    };
    assert_eq!(confirmed.state, ProposalState::Confirmed);
    assert_eq!(confirmed.result_status.as_deref(), Some("ok"));
    assert_eq!(confirmed.result_text.as_deref(), Some("{\"number\":42}"));
    let states: Vec<ProposalState> = (45..=48)
        .map(|index| match server(index) {
            ServerEvent::ProposalResolved(resolution) => resolution.state,
            other => panic!("line {index} is a resolution: {other:?}"),
        })
        .collect();
    let ended = [
        ProposalState::Denied,
        ProposalState::Refused,
        ProposalState::Expired,
        ProposalState::Confirmed,
    ];
    assert_eq!(states, ended);
    let ServerEvent::Error(refused) = server(66) else {
        panic!("line 66 is a refused answer");
    };
    assert_eq!(refused.reason, "request_failed");
    assert_eq!(refused.client_msg_id.as_deref(), Some("mac-10"));
}

#[test]
fn the_golden_lists_carry_their_rows_what_was_made_and_what_was_removed() {
    let ServerEvent::Checklists { lists } = server(50) else {
        panic!("line 50 is the index");
    };
    assert_eq!(lists[0].folder.as_deref(), Some("Projects"));
    assert_eq!((lists[1].open, lists[1].done, lists[1].rev), (3, 1, 7));
    let ServerEvent::Checklist { list, made } = server(53) else {
        panic!("line 53 is a list");
    };
    assert_eq!(list.rows[1].kind, "section");
    assert_eq!(list.rows[2].note.as_deref(), Some("The unwaxed ones"));
    assert_eq!(made["mac-21"], "ci_0r2y8e6u3g");
    let ServerEvent::Checklist { list, .. } = server(54) else {
        panic!("line 54 is a list");
    };
    assert_eq!(list.removed[0].row.text, "Basil");
    assert_eq!(list.removed[0].removed_at, "2031-06-02T14:05:00.000000Z");
    let ServerEvent::Checklist { list, made } = server(55) else {
        panic!("line 55 is a list");
    };
    assert_eq!(
        (list.rows.len(), made["mac-20"].as_str()),
        (0, "ck_2b7n5r8c3j")
    );
    let ServerEvent::Checklist { list, .. } = server(56) else {
        panic!("line 56 is a list");
    };
    assert_eq!(
        list.removed_at.as_deref(),
        Some("2031-05-20T08:00:00.000000Z")
    );
}
