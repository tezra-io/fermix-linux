//! The vendored management contract: its bytes are the engine's, pinned by checksum, and every
//! phone golden in it decodes through the app's own types. Each phone method is sent by the
//! app's own client to a fake daemon answering with the golden, so the request it writes is held
//! to the contract's request fixture too. A drift in the engine fails here rather than on a call.

use fermix_client::always_on::{
    passphrase, unconfirmed, Candidate, ItemStatus, MoveFrom, Role, Snapshots,
};
use fermix_client::frame::{read_frame, write_frame};
use fermix_client::management::{CallError, Management, LONG_CALL_DEADLINE};
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
fn the_paired_phones_and_a_forget_decode() {
    exchange("mobile_devices_list", "mobile_devices_list", |m| {
        let list = m.mobile_devices_list().expect("decodes");
        assert_eq!(list.devices.len(), 2, "a phone and a desktop a move paired");
        let phones = list.phones();
        assert_eq!(
            phones.devices.len(),
            1,
            "the phone list leaves the desktop out"
        );
        let phone = &phones.devices[0];
        assert_eq!(phone.kind, DeviceKind::Phone);
        assert_eq!(phone.executor, None);
        assert_eq!(phone.name, "Sam's phone");
        assert_eq!(phone.model, "Google Pixel 9 Pro");
        assert_eq!(phone.last_seen.as_deref(), Some("2026-09-26T12:04:40Z"));
    });
    let device = "3f4a1a55-69a0-4f8a-9132-17d6ac728f84";
    exchange("mobile_devices_revoke", "mobile_devices_revoke", |m| {
        let revoked = m.mobile_devices_revoke(device).expect("decodes");
        assert_eq!(revoked.device_id, device);
        assert!(revoked.revoked);
    });
}

/// A desktop's executor switch (M75 §3): the row names its kind and switch, and the write sends
/// exactly the request the contract names and decodes the row it answers.
#[test]
fn a_desktops_executor_switch_decodes_and_is_written_as_the_contract_says() {
    exchange("mobile_devices_list", "mobile_devices_list", |m| {
        let list = m.mobile_devices_list().expect("decodes");
        let desktop = &list.devices[1];
        assert_eq!(desktop.kind, DeviceKind::Desktop);
        assert_eq!(desktop.executor, Some(true));
    });
    let desktop = "c41e9d27-5a8b-4f63-9e20-7d1b3c5a8f94";
    exchange(
        "mobile_devices_set_executor",
        "mobile_devices_set_executor",
        |m| {
            let row = m
                .mobile_devices_set_executor(desktop, false)
                .expect("decodes");
            assert_eq!(row.device_id, desktop);
            assert_eq!(row.kind, DeviceKind::Desktop);
            assert_eq!(row.executor, Some(false));
        },
    );
}

/// The home goldens the Always On pane reads decode through the client, which sends what the
/// request fixtures name.
#[test]
fn the_home_goldens_decode_through_the_client() {
    exchange("home_status", "home_status", |m| {
        let status = m.home_status().expect("decodes");
        assert_eq!(status.role, Role::Primary);
        assert_eq!(status.epoch, Some(4));
        assert!(status.primary.is_none() && status.desktops.is_empty());
        assert!(status.snapshots.configured);
        let target = &status.snapshots.targets[0];
        assert_eq!(target.label, "S3");
        let verified = target.last_verified.as_ref().expect("a verified point");
        assert_eq!(verified.name.as_deref(), Some("20261027T093000Z-e4.fxsnap"));
        assert_eq!(
            status.snapshots.sentence,
            "The last snapshot was stored at 2026-10-27 09:30 UTC."
        );
        assert_eq!(status.secrets_missing, ["anthropic_api_key"]);
        assert_eq!(verified.key_id.as_deref(), Some("08c15ca36dd935c4"));
        assert!(status.primary_home.is_none() && status.primary_missing.is_none());
    });
    exchange("home_snapshots_run", "home_snapshots_run", |m| {
        let run = m.home_snapshots_run().expect("decodes");
        assert!(run.started && run.snapshots.running);
        assert_eq!(run.snapshots.sentence, "A snapshot is being written now.");
    });
}

/// After a move, a forwarded call the server cannot answer is refused in the daemon's words,
/// and the app shows that sentence rather than calling the daemon broken (M75 §6).
#[test]
fn an_unreachable_server_and_a_device_class_refusal_carry_their_sentences() {
    let response = named(ERRORS, "primary_unreachable")["response"].clone();
    let sentence =
        "The server fermix-vm is unreachable. Fermix keeps your message until it is back.";
    serve(response, |m| {
        let refused = m
            .settings_get("snapshots")
            .expect_err("the call is refused");
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
    serve(response, |m| match m.settings_get("engine") {
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

/// The code comes back once and no print of it spells it; the move goes out with exactly the
/// candidate and the passphrase the request fixture names, and its answer decodes.
#[test]
fn the_enrollment_code_and_the_move_go_as_the_contract_says() {
    exchange("home_enroll_open", "home_enroll_open", |m| {
        let code = m.home_enroll_open().expect("decodes");
        assert!(code.text().starts_with("fxenroll1."));
        assert!(!format!("{code:?}").contains(code.text()));
        assert_eq!(code.expires_at, "2026-10-27T09:40:00Z");
    });
    let params = &named(REQUESTS, "home_move_start")["frame"]["params"];
    let candidate = params["candidate_id"]
        .as_str()
        .expect("a candidate")
        .to_owned();
    let typed = params["recovery_passphrase"]
        .as_str()
        .expect("a passphrase")
        .to_owned();
    exchange("home_move_start", "home_move_start", |m| {
        let held = passphrase(&typed, &typed).expect("the two entries agree");
        let started = m
            .home_move_start(&candidate, &held, MoveFrom::ThisComputer)
            .expect("decodes");
        assert_eq!(started.candidate.candidate_id, candidate);
        assert_eq!(started.candidate.name, "north-box");
        assert!(started.keep_as_desktop);
        assert_eq!(
            started.sentence,
            "Fermix restarts in the device role and moves this home to north-box; keep the \
             recovery passphrase somewhere safe."
        );
    });
}

/// Replacing the server from a desktop is the same move, sent with `keep_as_desktop: false`, so
/// the server being replaced keeps no desktop key, row or link (M75 fix plan, decision 4).
#[test]
fn replacing_the_server_sends_the_move_that_keeps_no_desktop() {
    let params = &named(REQUESTS, "home_move_start_replacing_the_server")["frame"]["params"];
    assert_eq!(params["keep_as_desktop"], Value::Bool(false));
    let candidate = params["candidate_id"]
        .as_str()
        .expect("a candidate")
        .to_owned();
    let typed = params["recovery_passphrase"]
        .as_str()
        .expect("a passphrase")
        .to_owned();
    exchange(
        "home_move_start_replacing_the_server",
        "home_move_start",
        |m| {
            let held = passphrase(&typed, &typed).expect("the two entries agree");
            let started = m
                .home_move_start(&candidate, &held, MoveFrom::TheServer)
                .expect("decodes");
            assert_eq!(started.candidate.name, "north-box");
        },
    );
}

/// The readiness report goes out for the candidate the request fixture names, and every item
/// decodes with its status and the daemon's sentence.
#[test]
fn the_readiness_report_goes_as_the_contract_says() {
    let params = &named(REQUESTS, "home_move_readiness")["frame"]["params"];
    let candidate = params["candidate_id"]
        .as_str()
        .expect("a candidate")
        .to_owned();
    exchange("home_move_readiness", "home_move_readiness", |m| {
        let report = m.home_move_readiness(&candidate).expect("decodes");
        assert!(report.ready);
        let ids: Vec<&str> = report.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "keychain_secrets",
                "engine_version",
                "disk",
                "s3_size",
                "plugins",
                "stays_here",
                "cutover_gap",
                "serves_nothing"
            ]
        );
        assert!(report
            .items
            .iter()
            .all(|i| matches!(i.status, ItemStatus::Ok | ItemStatus::Warn)));
        assert!(report.items.iter().all(|i| i.sentence.ends_with('.')));
    });
}

/// A primary that stays one sets its recovery passphrase from the app (M75 review, edges F8):
/// the passphrase goes out as typed and the answer carries the daemon's sentence.
#[test]
fn the_recovery_passphrase_is_set_through_the_client() {
    let params = &named(REQUESTS, "home_snapshots_passphrase")["frame"]["params"];
    let typed = params["recovery_passphrase"]
        .as_str()
        .expect("a passphrase")
        .to_owned();
    exchange(
        "home_snapshots_passphrase",
        "home_snapshots_passphrase",
        |m| {
            let held = passphrase(&typed, &typed).expect("the two entries agree");
            let set = m.home_snapshots_passphrase(&held).expect("decodes");
            assert_eq!(set.key_id, "08c15ca36dd935c4");
            assert!(set.snapshots.recipient_set);
            assert!(set
                .sentence
                .starts_with("Snapshots from now on open with this passphrase"));
        },
    );
}

/// On a linked desktop `settings.sections` lists this computer's own sections, and says in
/// `primary_missing` why the server's are missing (M75 review, edges F13).
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
        .filter(|s| s["id"] == "engine" || s["id"] == "sandbox")
        .cloned()
        .collect();
    composed["sections"] = Value::Array(own);
    composed["primary_missing"] = Value::from(
        "The server fermix-vm is unreachable. Fermix keeps your message until it is back.",
    );
    let sections: Sections = serde_json::from_value(composed).expect("decodes");
    assert_eq!(sections.sections.len(), 2);
    assert!(sections
        .primary_missing
        .is_some_and(|s| s.starts_with("The server fermix-vm")));
}

/// Production Fermix is managed through the app, so no settings row the app draws (Always On's
/// sections among them) tells the owner to run a `fermix` command: not a label, a footer or an
/// info (M75 review, edges F7 and F8; fix plan, decision 13).
#[test]
fn no_settings_row_prescribes_a_fermix_command() {
    let goldens: Vec<Value> = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter(|v| {
            v["name"]
                .as_str()
                .is_some_and(|n| n.starts_with("settings_get"))
        })
        .collect();
    let names: Vec<&str> = goldens.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(names.contains(&"settings_get_engine") && names.contains(&"settings_get_snapshots"));
    for golden in &goldens {
        let rows = golden["response"]["result"]["rows"]
            .as_array()
            .expect("rows");
        for row in rows {
            for field in ["label", "footer", "info"] {
                let words = row[field].as_str().unwrap_or_default();
                assert!(
                    !names_a_command(words),
                    "{} {} {field}: {words}",
                    golden["name"],
                    row["key"]
                );
            }
        }
    }
}

/// Whether `words` name a `fermix` command: the word, a space, and a verb.
fn names_a_command(words: &str) -> bool {
    words.match_indices("fermix ").any(|(at, found)| {
        words[at + found.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
    })
}

/// Every home golden is decoded here or by a test above, so a golden the engine adds fails here
/// until the app reads it. The app reads candidates and snapshots through `home.status`, so
/// those goldens are decoded into the same types.
#[test]
fn no_home_golden_goes_unread() {
    let candidates =
        named(SUCCESS, "home_candidates_list")["response"]["result"]["candidates"].clone();
    let candidates: Vec<Candidate> = serde_json::from_value(candidates).expect("decodes");
    assert_eq!(candidates[0].name, "north-box");
    for (golden, pointer) in [
        ("home_snapshots_status", ""),
        ("home_snapshots_passphrase", "/snapshots"),
    ] {
        let result = &named(SUCCESS, golden)["response"]["result"];
        let value = result.pointer(pointer).expect("the snapshots").clone();
        let snapshots: Snapshots = serde_json::from_value(value).expect("decodes");
        assert!(snapshots.configured, "{golden}");
    }
    let read: BTreeSet<&str> = [
        "home_status",
        "home_snapshots_status",
        "home_snapshots_run",
        "home_snapshots_passphrase",
        "home_enroll_open",
        "home_candidates_list",
        "home_move_readiness",
        "home_move_start",
    ]
    .into_iter()
    .collect();
    let published: Vec<String> = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .filter(|name| name.starts_with("home_"))
        .collect();
    for name in &published {
        assert!(read.contains(name.as_str()), "{name} is not decoded");
    }
    assert_eq!(published.len(), read.len());
}

/// The recovery passphrase the request fixture carries goes out as written, and never in a
/// print of the value that holds it.
#[test]
fn the_passphrase_is_sent_as_typed_and_never_printed() {
    let frame = &named(REQUESTS, "home_snapshots_passphrase")["frame"];
    let typed = frame["params"]["recovery_passphrase"]
        .as_str()
        .expect("the fixture's passphrase");
    let held = passphrase(typed, typed).expect("the two entries agree");
    assert!(!format!("{held:?}").contains(typed));
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
        "mobile_devices_set_executor",
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

/// The two move calls walk the whole home before they answer, and a desktop forwarding one waits
/// 60 s for the server: the client gives them at least 65 s (PROTOCOL.md, "Forward deadlines");
/// every other call keeps its own.
#[test]
fn the_move_calls_wait_at_least_sixty_five_seconds() {
    let protocol = include_str!("../contracts/management/PROTOCOL.md");
    assert!(protocol.contains("| `home.move.start` | 60 s | at least 65 s |"));
    assert!(LONG_CALL_DEADLINE >= Duration::from_secs(65));
    let usual = Duration::from_secs(10);
    let client = Management::new(PathBuf::from("/nonexistent/daemon.sock"), usual);
    for method in ["home.move.readiness", "home.move.start"] {
        assert!(
            client.deadline_for(method) >= Duration::from_secs(65),
            "{method}"
        );
    }
    for method in [
        "home.status",
        "settings.get",
        "home.snapshots.run",
        "home.enroll.open",
    ] {
        assert_eq!(client.deadline_for(method), usual, "{method}");
    }
    let patient = Management::new(
        PathBuf::from("/nonexistent/daemon.sock"),
        Duration::from_secs(90),
    );
    assert_eq!(
        patient.deadline_for("home.move.start"),
        Duration::from_secs(90)
    );
}

/// A forwarded start whose answer was lost is `unconfirmed`, with the daemon's sentence: the pane
/// follows the move rather than calling it refused.
#[test]
fn an_unconfirmed_start_carries_its_sentence() {
    let response = named(ERRORS, "unconfirmed")["response"].clone();
    serve(response, |m| {
        let params = &named(REQUESTS, "home_move_start")["frame"]["params"];
        let candidate = params["candidate_id"].as_str().expect("a candidate");
        let typed = params["recovery_passphrase"]
            .as_str()
            .expect("a passphrase");
        let held = passphrase(typed, typed).expect("they agree");
        let refused = m
            .home_move_start(candidate, &held, MoveFrom::TheServer)
            .expect_err("unconfirmed");
        assert_eq!(
            unconfirmed(&refused),
            Some(
                "The server fermix-vm did not confirm in time whether the move started. Its \
                 state shows once the server answers again, and starting it again never starts \
                 a second move."
            )
        );
    });
    let other = named(ERRORS, "unavailable_home_owner_decision")["response"].clone();
    serve(other, |m| {
        let refused = m.home_enroll_open().expect_err("refused");
        assert_eq!(unconfirmed(&refused), None);
    });
}

/// The read grants: listed and removed through the client as the request fixtures name them, and
/// a removal by a process that is not the owner refused in the daemon's words.
#[test]
fn the_read_grants_go_as_the_contract_says() {
    exchange("device_grants_list", "device_grants_list", |m| {
        let listed = m.device_grants_list().expect("decodes");
        assert_eq!(listed.grants.len(), 2);
        assert_eq!(listed.grants[1].path, "/Users/ana/Taxes");
        assert!(listed.sentence.starts_with("The server may read 2 folders"));
    });
    let path = named(REQUESTS, "device_grants_remove")["frame"]["params"]["path"]
        .as_str()
        .expect("a path")
        .to_owned();
    exchange("device_grants_remove", "device_grants_remove", |m| {
        let removed = m.device_grants_remove(&path).expect("decodes");
        assert_eq!(removed.removed, path);
        assert_eq!(removed.grants.len(), 1);
        assert_eq!(
            removed.sentence,
            "The server can no longer read that folder on this computer."
        );
    });
    let refusal = named(ERRORS, "unavailable_device_owner_decision")["response"].clone();
    serve(refusal, |m| match m.device_grants_remove(&path) {
        Err(CallError::Refused(r)) => assert!(r.sentence.starts_with("Only the owner can change")),
        other => panic!("an owner decision, got {other:?}"),
    });
    let published: Vec<String> = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .filter(|name| name.starts_with("device_"))
        .collect();
    assert_eq!(
        published,
        ["device_grants_list", "device_grants_remove"],
        "each one read above"
    );
}
