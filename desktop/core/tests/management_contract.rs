//! The vendored management contract: its bytes are the engine's, pinned by checksum, and every
//! phone golden in it decodes through the app's own types. Each phone method is sent by the
//! app's own client to a fake daemon answering with the golden, so the request it writes is held
//! to the contract's request fixture too. A drift in the engine fails here rather than on a call.

use fermix_client::frame::{read_frame, write_frame};
use fermix_client::management::{CallError, Management};
use fermix_client::mobile::{DeviceKind, OutcomeReason, SessionState};
use fermix_client::phone::switch_starts_in_place;
use fermix_client::settings::{switch_key, SectionRows, Sections};
use fermix_client::view::{daemon_problem, DaemonProblem};
use serde_json::Value;
use std::collections::BTreeSet;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

const CHECKSUMS: &str = include_str!("../contracts/management/CHECKSUMS.txt");
const REQUESTS: &str = include_str!("../contracts/management/fixtures/requests.jsonl");
const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const ERRORS: &str = include_str!("../contracts/management/fixtures/errors.jsonl");
const VENDORED: [&str; 6] = [
    "PROTOCOL.md",
    "protocol.schema.json",
    "fixtures/requests.jsonl",
    "fixtures/success.jsonl",
    "fixtures/errors.jsonl",
    "fixtures/compatibility.jsonl",
];
const SESSION: &str = "5b0c7d2e-8f41-4a6b-9c3d-2e7f1a8b4c60";

fn contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/management")
}

/// The one line of `lines` named `name`.
fn named(lines: &str, name: &str) -> Value {
    let found: Vec<Value> = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .filter(|v| v["name"] == name)
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one fixture named {name}");
    found.into_iter().next().expect("one")
}

/// A fake daemon that reads one request and answers with `response`, the golden's envelope with
/// the request's own id; it hands the request back.
fn serve(response: Value, call: impl FnOnce(&Management)) -> Value {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).expect("the fake daemon binds");
    let daemon = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the client connects");
        let request: Value =
            serde_json::from_slice(&read_frame(&mut stream).expect("a request frame"))
                .expect("the request is JSON");
        let mut answer = response;
        answer["request_id"] = request["request_id"].clone();
        let bytes = serde_json::to_vec(&answer).expect("the answer serializes");
        write_frame(&mut stream, &bytes).expect("the answer is written");
        request
    });
    call(&Management::new(socket, Duration::from_secs(2)));
    daemon.join().expect("the fake daemon ends")
}

/// Sends one phone call against the golden `answer` and holds what the client wrote to the
/// request fixture `request`: the same method and the same params.
fn exchange(request: &str, answer: &str, call: impl FnOnce(&Management)) {
    let golden = named(SUCCESS, answer)["response"].clone();
    let sent = serve(golden, call);
    let expected = &named(REQUESTS, request)["frame"];
    assert_eq!(sent["method"], expected["method"], "{request}");
    assert_eq!(sent["params"], expected["params"], "{request}");
    assert_eq!(
        sent["protocol_version"], expected["protocol_version"],
        "{request}"
    );
}

#[test]
fn the_checksums_name_exactly_the_six_vendored_files() {
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
        "a vendored file was edited:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn the_status_decodes_with_the_open_window_it_names() {
    let mut status = None;
    exchange("mobile_status", "mobile_status", |m| {
        status = Some(m.mobile_status().expect("the golden decodes"));
    });
    let status = status.expect("answered");
    assert!(status.enabled && status.started && !status.refused);
    assert_eq!(status.paired_devices, 1);
    let pairing = status.pairing.expect("a window is named");
    assert_eq!(pairing.session_id, SESSION);
    assert_eq!(pairing.state, SessionState::AwaitingDecision);
}

#[test]
fn both_starts_decode_and_only_the_opened_one_carries_a_window() {
    let mut opened = None;
    exchange("mobile_pair_start", "mobile_pair_start", |m| {
        opened = Some(m.mobile_pair_start().expect("the golden decodes"));
    });
    let opened = opened.expect("answered");
    assert_eq!(opened.session.session_id.as_deref(), Some(SESSION));
    assert_eq!(opened.session.state, SessionState::AwaitingScan);
    assert!(opened.has_uri(), "the start hands the link back once");

    let mut refused = None;
    exchange("mobile_pair_start", "mobile_pair_start_channel_off", |m| {
        refused = Some(m.mobile_pair_start().expect("the golden decodes"));
    });
    let refused = refused.expect("answered");
    assert_eq!(refused.session.state, SessionState::Failed);
    assert_eq!(refused.session.session_id, None);
    assert!(!refused.has_uri());
    let failure = refused.session.failure.expect("a failed start says why");
    assert_eq!(failure.sentence, "The mobile channel is turned off.");
}

#[test]
fn every_session_read_decodes_with_the_fields_its_state_carries() {
    let cases = [
        ("mobile_pair_get_awaiting_scan", SessionState::AwaitingScan),
        (
            "mobile_pair_get_awaiting_decision",
            SessionState::AwaitingDecision,
        ),
        ("mobile_pair_get_approved", SessionState::Approved),
        ("mobile_pair_get_denied", SessionState::Denied),
        ("mobile_pair_get_expired", SessionState::Expired),
        ("mobile_pair_get_cancelled", SessionState::Cancelled),
        ("mobile_pair_get_failed", SessionState::Failed),
        ("mobile_pair_get_refused", SessionState::Failed),
    ];
    for (golden, state) in cases {
        let mut session = None;
        exchange("mobile_pair_get", golden, |m| {
            session = Some(m.mobile_pair_get(SESSION).expect("the golden decodes"));
        });
        let session = session.expect("answered");
        assert_eq!(session.state, state, "{golden}");
        assert_eq!(session.ttl_ms.is_some(), !state.is_terminal(), "{golden}");
    }
}

/// A refused secure-hardware proof never reaches the owner: the window fails with no request.
#[test]
fn a_refused_proof_fails_the_window_with_no_phone_named() {
    let mut refused = None;
    exchange("mobile_pair_get", "mobile_pair_get_refused", |m| {
        refused = Some(m.mobile_pair_get(SESSION).expect("the golden decodes"));
    });
    let refused = refused.expect("answered");
    assert_eq!(refused.state, SessionState::Failed);
    assert!(refused.request.is_none(), "no phone reaches the owner");
    let failure = refused.failure.expect("the daemon says why");
    assert_eq!(failure.code, "refused");
}

#[test]
fn a_decision_and_a_cancel_answer_the_terminal_view() {
    let mut approved = None;
    exchange("mobile_pair_decide", "mobile_pair_decide", |m| {
        approved = Some(m.mobile_pair_decide(SESSION, true).expect("decodes"));
    });
    let approved = approved.expect("answered");
    assert_eq!(approved.state, SessionState::Approved);
    let request = approved.request.expect("the approved phone is named");
    assert_eq!(request.device_name, "Sam's phone");
    assert_eq!(request.model, "Google Pixel 9 Pro");
    assert_eq!(request.sas, "481062");
    assert_eq!(
        request.attestation.sentence,
        "Secure hardware, locked bootloader, official Fermix build."
    );

    // Denying sends the same method with approved false; the golden answer is not read.
    exchange("mobile_pair_decide_deny", "mobile_pair_get_denied", |m| {
        let denied = m.mobile_pair_decide(SESSION, false).expect("decodes");
        let reason = denied.outcome.and_then(|o| o.reason);
        assert_eq!(reason, Some(OutcomeReason::Denied));
    });

    exchange("mobile_pair_cancel", "mobile_pair_cancel", |m| {
        let cancelled = m.mobile_pair_cancel(SESSION).expect("decodes");
        assert_eq!(cancelled.state, SessionState::Cancelled);
    });
}

#[test]
fn the_paired_devices_and_a_forget_decode() {
    exchange("mobile_devices_list", "mobile_devices_list", |m| {
        let list = m.mobile_devices_list().expect("decodes");
        assert_eq!(list.devices.len(), 2);
        let phone = &list.devices[0];
        assert_eq!(phone.name, "Sam's phone");
        assert_eq!(phone.model, "Google Pixel 9 Pro");
        assert_eq!(phone.kind, DeviceKind::Phone);
        assert_eq!(phone.last_seen.as_deref(), Some("2026-09-26T12:04:40Z"));
        let desktop = &list.devices[1];
        assert_eq!(desktop.name, "Studio Mac");
        assert_eq!(desktop.kind, DeviceKind::Desktop, "a desktop a move paired");
    });
    let device = "3f4a1a55-69a0-4f8a-9132-17d6ac728f84";
    exchange("mobile_devices_revoke", "mobile_devices_revoke", |m| {
        let revoked = m.mobile_devices_revoke(device).expect("decodes");
        assert_eq!(revoked.device_id, device);
        assert!(revoked.revoked);
    });
}

/// A desktop is never offered Forget; were one sent anyway, the daemon says why in its words.
#[test]
fn a_desktops_forget_is_refused_in_the_daemons_words() {
    let response = named(ERRORS, "unavailable_mobile_desktop_revoke")["response"].clone();
    serve(response, |m| {
        match m.mobile_devices_revoke("c41e9d27-5a8b-4f63-9e20-7d1b3c5a8f94") {
            Err(CallError::Refused(refusal)) => {
                assert_eq!(refusal.code, "unavailable");
                assert!(refusal
                    .sentence
                    .starts_with("This desktop cannot be removed"));
            }
            other => panic!("a desktop's forget is refused, got {other:?}"),
        }
    });
}

/// Every phone golden is decoded by one of the tests above, so a golden the engine adds fails
/// here until the app reads it.
#[test]
fn no_phone_golden_goes_unread() {
    let read: BTreeSet<&str> = [
        "mobile_status",
        "mobile_pair_start",
        "mobile_pair_start_channel_off",
        "mobile_pair_get_awaiting_scan",
        "mobile_pair_get_awaiting_decision",
        "mobile_pair_get_approved",
        "mobile_pair_get_denied",
        "mobile_pair_get_expired",
        "mobile_pair_get_cancelled",
        "mobile_pair_get_failed",
        "mobile_pair_get_refused",
        "mobile_pair_decide",
        "mobile_pair_cancel",
        "mobile_devices_list",
        "mobile_devices_revoke",
    ]
    .into_iter()
    .collect();
    let published: Vec<String> = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .filter(|name| name.starts_with("mobile_"))
        .collect();
    for name in &published {
        assert!(read.contains(name.as_str()), "{name} is not decoded");
    }
    assert_eq!(published.len(), read.len());
}

#[test]
fn the_phone_refusals_carry_their_codes_and_the_daemons_words() {
    for (golden, code, sentence) in [
        (
            "busy_mobile_pair",
            "busy",
            "Another management operation of this kind is already running.",
        ),
        (
            "unknown_pairing_session",
            "unknown_pairing_session",
            "The pairing session is not retained by this daemon.",
        ),
        (
            "unavailable_owner_decision",
            "unavailable",
            "Only the owner can pair or forget a phone; run this from your own terminal.",
        ),
    ] {
        let response = named(ERRORS, golden)["response"].clone();
        serve(response, |m| match m.mobile_pair_start() {
            Err(CallError::Refused(refusal)) => {
                assert_eq!(refusal.code, code, "{golden}");
                assert_eq!(refusal.sentence, sentence, "{golden}");
            }
            other => panic!("{golden} should refuse, got {other:?}"),
        });
    }
}

/// After a move, a request the server cannot be reached for says so in the daemon's words, and the
/// window says the server is unreachable rather than that Fermix here is not responding.
#[test]
fn an_unreachable_server_and_a_device_class_refusal_carry_their_sentences() {
    let response = named(ERRORS, "primary_unreachable")["response"].clone();
    let sentence =
        "The server fermix-vm is unreachable. Fermix keeps your message until it is back.";
    serve(response, |m| {
        let refused = m.setup_state().expect_err("the call is refused");
        let problem = daemon_problem(&refused);
        let CallError::Refused(refusal) = refused else {
            panic!("primary_unreachable should refuse, got {refused:?}");
        };
        assert_eq!(refusal.code, "primary_unreachable");
        assert_eq!(refusal.sentence, sentence);
        assert_eq!(problem, DaemonProblem::PrimaryUnreachable(sentence.into()));
        assert_eq!(problem.status_word(), "Server unreachable");
        assert_eq!(problem.sentence(), sentence);
    });
    let response = named(ERRORS, "device_class_method")["response"].clone();
    serve(response, |m| match m.settings_get("sandbox") {
        Err(CallError::Refused(refusal)) => {
            assert_eq!(refusal.code, "device_class_method");
            assert_eq!(
                refusal.sentence,
                "This request configures the computer it came from, so the server does not \
                 answer it."
            );
        }
        other => panic!("device_class_method should refuse, got {other:?}"),
    });
}

/// On a desktop whose server is unreachable, `settings.sections` lists this computer's own
/// sections and says in `primary_missing` why the server's are missing.
#[test]
fn the_sections_say_when_the_servers_are_missing() {
    let listed = named(SUCCESS, "settings_sections")["response"]["result"].clone();
    let sections: Sections = serde_json::from_value(listed.clone()).expect("decodes");
    assert_eq!(sections.primary_missing, None);
    let mut composed = listed;
    let own: Vec<Value> = composed["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .filter(|s| s["id"] == "sandbox" || s["id"] == "computer_use")
        .cloned()
        .collect();
    composed["sections"] = Value::Array(own);
    composed["primary_missing"] = Value::from("The server fermix-vm is unreachable.");
    let sections: Sections = serde_json::from_value(composed).expect("decodes");
    assert_eq!(sections.sections.len(), 2);
    assert_eq!(
        sections.primary_missing.as_deref(),
        Some("The server fermix-vm is unreachable.")
    );
}

/// The section's first row is the channel's switch; reach from anywhere's switch follows it, and
/// the network rows, the local-network announcement among them, are advanced.
#[test]
fn the_phone_sections_switch_is_mobile_enabled() {
    let result = named(SUCCESS, "settings_get_channels_mobile")["response"]["result"].clone();
    let section: SectionRows = serde_json::from_value(result).expect("the section decodes");
    assert_eq!(section.title, "Phone");
    assert_eq!(switch_key("mobile", &section), Some("mobile_enabled"));
    assert!(
        switch_starts_in_place(Some(&section)),
        "the switch's row says it starts the channel at once"
    );
    let rows: Vec<(&str, bool)> = section
        .rows
        .iter()
        .map(|r| (r.key.as_str(), r.advanced))
        .collect();
    assert_eq!(
        rows,
        [
            ("mobile_enabled", false),
            ("mobile_iroh", false),
            ("mobile_port", true),
            ("mobile_bind", true),
            ("mobile_advertise_mdns", true)
        ],
        "the network rows are advanced, the two switches are not"
    );
    let reach = &section.rows[1];
    assert!(
        reach.info.is_some(),
        "reach from anywhere explains what off keeps"
    );
}
