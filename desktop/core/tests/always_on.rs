//! The Always On pane (M75 §14): what it draws for each shape of `home.status`, the one source it
//! reads, and the steps the app holds itself: the code shown once, the passphrase rule, the
//! readiness report before the move, the move followed across the engine's restart (and given up
//! on plainly once it stalls), and, on a desktop, the server's recovery and its replacement.
//!
//! The shapes start from the vendored goldens: `home_status` as published, or emptied of its
//! enrollment, candidates and move for a home that has done nothing yet, and the candidate of
//! `home_candidates_list`. How the client sends the calls is held in `management_contract.rs`.

use fermix_client::always_on::{
    expiry, grant_rows, moving_between, not_back, not_taken_over, passphrase, restarting, span,
    view, Checked, Code, DesktopStage, Destination, DeviceGrants, HomeStatus, ItemStatus, MoveFrom,
    Moving, Readiness, Replacing, Role, Setup, Stage, Steps, View, CANDIDATE, CODE_ELSEWHERE,
    CODE_EXPIRED, CONNECTED, GET_CODE, MOVE_BACK, NEW_CODE, NOTHING_VERIFIED, NOT_CONNECTED,
    PASSPHRASE_MISMATCH, POLLS_PER_VISIT, POLL_EVERY, RECONNECTING, REMOVED, REPLACE_GRACE,
    RESTART_GRACE, SECRETS_HERE, SECRETS_SERVER, SERVER_AWAY, THE_SERVER, UNLINKED,
};
use fermix_client::view::DaemonProblem;
use serde_json::{json, Value};
use std::time::Duration;

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
/// 2026-10-27T10:00:00Z: half an hour after the golden's verified snapshot.
const NOW: i64 = 1_793_095_200;
const CANDIDATE_ID: &str = "cand-5e2a9c1f7b3d4086";

fn golden(name: &str) -> Value {
    SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no golden named {name}"))["response"]["result"]
        .clone()
}

/// The golden `home.status` as published.
fn published() -> HomeStatus {
    serde_json::from_value(golden("home_status")).expect("home.status decodes")
}

/// The golden `home.status` of a home that has opened no window, holds no candidate and started
/// no move, changed by `change`.
fn status(change: impl FnOnce(&mut Value)) -> HomeStatus {
    let mut result = golden("home_status");
    result["enrollment"] = Value::Null;
    result["candidates"] = json!([]);
    result["move"] = Value::Null;
    change(&mut result);
    serde_json::from_value(result).expect("home.status decodes")
}

/// The candidate `home.candidates.list` publishes, which `home.status` holds in the same shape.
fn candidate() -> Value {
    golden("home_candidates_list")["candidates"][0].clone()
}

/// The code `home.enroll.open` publishes, expiring at `expires_at`.
fn code(expires_at: &str) -> Code {
    let mut opened = golden("home_enroll_open");
    opened["expires_at"] = json!(expires_at);
    serde_json::from_value(opened).expect("the code decodes")
}

fn code_text() -> String {
    golden("home_enroll_open")["code"]
        .as_str()
        .expect("a code")
        .to_owned()
}

fn drawn(status: &HomeStatus, steps: &Steps) -> View {
    view(Some(Ok(status)), steps, NOW)
}

fn setup_of(view: &View) -> &Setup {
    match &view.stage {
        Stage::Setup(setup) => setup,
        other => panic!("expected the setup, got {other:?}"),
    }
}

fn moving_of(view: &View) -> &Moving {
    match &view.stage {
        Stage::Moving(moving) => moving,
        other => panic!("expected a move, got {other:?}"),
    }
}

fn desktop_of(view: &View) -> &DesktopStage {
    match &view.stage {
        Stage::Desktop(desktop) => desktop,
        other => panic!("expected a desktop, got {other:?}"),
    }
}

/// The readiness report `home.move.readiness` publishes, changed by `change`.
fn readiness(change: impl FnOnce(&mut Value)) -> Readiness {
    let mut report = golden("home_move_readiness");
    change(&mut report);
    serde_json::from_value(report).expect("the readiness report decodes")
}

/// A desktop linked to the server north-box, whose own picture (`primary_home`) is `server`, or
/// missing with the daemon's sentence when `server` is None.
fn linked(server: Option<Value>) -> HomeStatus {
    status(|s| {
        s["role"] = json!("device");
        s["moved"] = json!({"to": "north-box", "moved_at": "2026-10-27T09:40:00Z", "epoch": 5});
        s["primary"] = json!({"name": "north-box", "connected": server.is_some(),
            "since": null, "epoch": 5, "sentence": "Connected to north-box."});
        s["snapshots"]["configured"] = json!(false);
        s["snapshots"]["recipient_set"] = json!(false);
        s["snapshots"]["next_run_at"] = json!(null);
        s["snapshots"]["targets"] = json!([]);
        s["snapshots"]["sentence"] = json!("Snapshots are the server's to take.");
        s["secrets_missing"] = json!([]);
        s["primary_missing"] = match &server {
            Some(_) => Value::Null,
            None => json!(UNREACHABLE),
        };
        s["primary_home"] = server.unwrap_or(Value::Null);
    })
}

/// The server's own `home.status`: the published golden, with its candidate, at epoch 5.
fn server_home(change: impl FnOnce(&mut Value)) -> Value {
    let mut server = golden("home_status");
    server["epoch"] = json!(5);
    server["move"] = Value::Null;
    server["enrollment"] = Value::Null;
    change(&mut server);
    server
}

const UNREACHABLE: &str =
    "The server north-box is unreachable. Fermix keeps your message until it is back.";

#[test]
fn a_primary_that_never_moved_explains_and_asks_where_to_run() {
    let fresh = status(|_| {});
    let steps = Steps::default();
    let view = drawn(&fresh, &steps);
    let setup = setup_of(&view);
    assert_eq!(setup.destination, None);
    assert_eq!(
        setup.get_code, None,
        "a code follows the choice of a server"
    );
    assert!(setup.code.is_none() && !setup.waiting && setup.candidates.is_empty());
    assert_eq!(setup.notice, None);
    let recovery = view.recovery.expect("recovery is drawn from the status");
    assert_eq!(
        recovery.sentence,
        "The last snapshot was stored at 2026-10-27 09:30 UTC."
    );
    assert_eq!(recovery.targets[0].title, "S3");
    assert_eq!(recovery.targets[0].verified, "Last verified 30 minutes ago");
    assert_eq!(recovery.next.as_deref(), Some("Next snapshot in 5 hours"));
    assert!(recovery.can_run, "configured and not running");
    assert!(view.desktops.is_empty());
}

/// The golden as published: a used window, the server that used it, and a move that was undone.
#[test]
fn the_published_status_lists_its_candidate_and_says_the_move_was_undone() {
    let status = published();
    assert_eq!(status.role, Role::Primary);
    let view = drawn(&status, &Steps::default());
    let setup = setup_of(&view);
    assert_eq!(
        setup.notice.as_deref(),
        Some("The move to north-box was undone; this home is still the primary.")
    );
    assert_eq!(setup.code_note, None, "a used window leaves nothing to say");
    assert_eq!(setup.candidates.len(), 1);
    assert_eq!(setup.candidates[0].id, CANDIDATE_ID);
}

#[test]
fn choosing_a_server_offers_a_code_and_the_code_is_shown_with_its_expiry() {
    let fresh = status(|_| {});
    let mut steps = Steps::default();
    steps.destination = Some(Destination::Aws);
    assert_eq!(setup_of(&drawn(&fresh, &steps)).get_code, Some(GET_CODE));
    steps.opening = true;
    assert_eq!(
        setup_of(&drawn(&fresh, &steps)).get_code,
        None,
        "one at a time"
    );
    steps.code_opened(code("2026-10-27T10:10:00Z"));
    let open = status(|s| {
        s["enrollment"] = json!({"state": "open", "expires_at": "2026-10-27T10:10:00Z"});
    });
    let view = drawn(&open, &steps);
    let setup = setup_of(&view);
    let shown = setup.code.as_ref().expect("the code is shown");
    assert_eq!(shown.text, code_text());
    assert_eq!(shown.expiry, "Use it within 10 minutes. It works once.");
    assert!(setup.waiting, "no server has enrolled yet");
    assert_eq!(setup.get_code, Some(NEW_CODE));
    assert_eq!(setup.code_note, None);
}

/// The daemon answers the code once and no status carries it, so the app holds it only while
/// its window can still be used, and no print of it spells it.
#[test]
fn the_code_is_shown_once_and_let_go_when_its_window_ends() {
    let mut steps = Steps::default();
    steps.destination = Some(Destination::ExistingHost);
    steps.code_opened(code("2026-10-27T10:10:00Z"));
    assert!(!format!("{steps:?}").contains(&code_text()), "{steps:?}");

    let used = status(|s| {
        s["enrollment"] = json!({"state": "used", "expires_at": "2026-10-27T10:10:00Z"});
        s["candidates"] = json!([candidate()]);
    });
    steps.settle(&used, steps.ticket(), NOW);
    assert!(steps.code().is_none(), "a used code is gone");
    let view = drawn(&used, &steps);
    let setup = setup_of(&view);
    assert!(setup.code.is_none() && !setup.waiting);
    assert_eq!(setup.code_note, None);
    assert_eq!(setup.candidates.len(), 1);

    steps.code_opened(code("2026-10-27T09:59:00Z"));
    let open = status(|s| {
        s["enrollment"] = json!({"state": "open", "expires_at": "2026-10-27T09:59:00Z"});
    });
    steps.settle(&open, steps.ticket(), NOW);
    assert!(steps.code().is_none(), "a code past its own expiry is gone");
    let view = drawn(&open, &steps);
    // The daemon still says a window is open, and the app no longer has its code.
    assert_eq!(setup_of(&view).code_note, Some(CODE_ELSEWHERE));
    assert_eq!(setup_of(&view).get_code, Some(NEW_CODE));

    // The daemon's word that the window expired is enough, whatever this computer's clock says.
    let expired = status(|s| {
        s["enrollment"] = json!({"state": "expired", "expires_at": "2026-10-27T10:05:00Z"});
    });
    steps.code_opened(code("2026-10-27T10:05:00Z"));
    steps.settle(&expired, steps.ticket(), NOW);
    assert!(steps.code().is_none());
    assert_eq!(
        setup_of(&drawn(&expired, &steps)).code_note,
        Some(CODE_EXPIRED)
    );
}

#[test]
fn a_status_without_a_window_keeps_the_code_until_it_expires() {
    let mut steps = Steps::default();
    steps.code_opened(code("2026-10-27T10:10:00Z"));
    steps.settle(&status(|_| {}), steps.ticket(), NOW);
    assert!(
        steps.code().is_some(),
        "a null enrollment says nothing about the code"
    );
}

#[test]
fn a_candidate_is_listed_and_reviewed_with_its_instance_facts_unverified() {
    let enrolled = status(|s| s["candidates"] = json!([candidate()]));
    let mut steps = Steps::default();
    steps.review(CANDIDATE_ID);
    steps.settle(&enrolled, steps.ticket(), NOW);
    let view = drawn(&enrolled, &steps);
    let setup = setup_of(&view);
    let row = &setup.candidates[0];
    assert_eq!(row.title, "north-box");
    assert_eq!(row.subtitle, "linux, aarch64, Fermix 0.9.0");
    let review = setup.review.as_ref().expect("under review");
    assert_eq!(review.name, "north-box");
    assert_eq!(
        review.host,
        [
            ("Name", "north-box".to_owned()),
            ("System", "linux, aarch64".to_owned()),
            ("Fermix", "0.9.0".to_owned()),
            ("Enrolled", "48 minutes ago".to_owned()),
            ("Snapshot bucket", "invented-fermix-snapshots".to_owned()),
        ]
    );
    assert_eq!(
        review.instance,
        [
            ("Provider", "aws".to_owned()),
            ("Instance ID", "i-0a1b2c3d4e5f60718".to_owned()),
            ("Region", "eu-central-1".to_owned()),
            ("Account", "111122223333".to_owned()),
        ]
    );

    let gone = status(|_| {});
    steps.settle(&gone, steps.ticket(), NOW);
    assert_eq!(
        steps.reviewing(),
        None,
        "a candidate no longer held is let go"
    );
}

#[test]
fn a_host_that_reported_no_instance_has_no_instance_facts() {
    let bare = status(|s| {
        let mut held = candidate();
        held["instance"] = Value::Null;
        held["snapshots"] = Value::Null;
        s["candidates"] = json!([held]);
    });
    let mut steps = Steps::default();
    steps.review(CANDIDATE_ID);
    let view = drawn(&bare, &steps);
    let review = setup_of(&view).review.clone().expect("under review");
    assert!(review.instance.is_empty());
    assert!(!review
        .host
        .iter()
        .any(|(label, _)| *label == "Snapshot bucket"));
}

#[test]
fn the_passphrase_is_ready_only_when_both_entries_agree() {
    assert_eq!(passphrase("", ""), Err(None));
    assert_eq!(passphrase("invented-lighthouse", ""), Err(None));
    assert_eq!(
        passphrase("invented-lighthouse", "invented-lighthousE"),
        Err(Some(PASSPHRASE_MISMATCH))
    );
    let held = passphrase("invented-lighthouse", "invented-lighthouse").expect("they agree");
    assert!(!format!("{held:?}").contains("lighthouse"));
}

/// After the move starts the engine restarts into it. A read that fails meanwhile is that
/// restart, not a stopped Fermix, until the engine that came back says the move ended.
#[test]
fn the_move_is_followed_across_the_engines_restart() {
    let mut steps = Steps::default();
    steps.move_started("north-box");
    let ticket = steps.ticket();
    let down = DaemonProblem::NotRunning;
    assert!(steps.unanswered(ticket, NOW));
    assert_eq!(
        view(Some(Err(&down)), &steps, NOW).stage,
        Stage::Moving(Moving {
            sentence: "Fermix is restarting to move this home to north-box.".into(),
            detail: None,
            waiting: true,
            stalled: false,
            problem: None,
        })
    );
    let streaming = status(|s| {
        s["role"] = json!("device");
        s["move"] = json!({"phase": "streaming", "candidate": "north-box",
            "sentence": "This home is being sent to north-box."});
    });
    steps.settle(&streaming, ticket, NOW);
    assert!(steps.started.is_some(), "the move runs on");
    assert_eq!(
        moving_of(&drawn(&streaming, &steps)).sentence,
        "This home is being sent to north-box."
    );
    let done = status(|s| {
        s["role"] = json!("device");
        s["moved"] = json!({"to": "north-box", "moved_at": "2026-10-27T09:40:00Z", "epoch": 5});
        s["primary"] = json!({"name": "north-box", "connected": true,
            "since": "2026-10-27T09:41:00Z", "epoch": 5,
            "sentence": "Connected to north-box."});
        s["move"] = json!({"phase": "done", "candidate": "north-box",
            "sentence": "This home moved to north-box; this computer is now a desktop of it."});
    });
    steps.settle(&done, ticket, NOW);
    assert_eq!(
        steps.started, None,
        "the engine that came back says it ended"
    );
    let shown = drawn(&done, &steps);
    let server = &desktop_of(&shown).server;
    assert_eq!(server.name, "north-box");
    assert_eq!(server.state, CONNECTED);
    assert_eq!(server.sentence, "Connected to north-box.");
    assert_eq!(
        server.moved.as_deref(),
        Some("Fermix moved to north-box 20 minutes ago.")
    );
    assert_eq!(
        view(Some(Err(&down)), &steps, NOW).stage,
        Stage::Down(DaemonProblem::NotRunning),
        "with no move of the app's own, a failed read is the daemon's state"
    );
}

/// `committed` and `undoing` are written just before the engine restarts: only the engine that
/// came back writes `done` or `aborted` (M75 review, edges F1). Until then the move is followed,
/// and the restart's failed reads are the restart.
#[test]
fn committed_and_undoing_are_not_endings() {
    let down = DaemonProblem::NotRunning;
    for (role, phase) in [("device", "committed"), ("device", "undoing")] {
        let mut steps = Steps::default();
        steps.move_started("north-box");
        let ticket = steps.ticket();
        let about_to_restart = status(|s| {
            s["role"] = json!(role);
            s["move"] = json!({"phase": phase, "candidate": "north-box",
                "sentence": "Fermix restarts to finish the move."});
        });
        steps.settle(&about_to_restart, ticket, NOW);
        assert_eq!(steps.started.as_deref(), Some("north-box"), "{phase}");
        assert_eq!(
            moving_of(&drawn(&about_to_restart, &steps)).sentence,
            "Fermix restarts to finish the move.",
            "{phase}"
        );
        steps.unanswered(ticket, NOW);
        assert_eq!(
            moving_of(&view(Some(Err(&down)), &steps, NOW)).sentence,
            restarting("north-box"),
            "{phase}: the restart that follows is not a stopped Fermix"
        );
    }
}

/// A read asked before the move started may land after it: what it says (here, the published
/// golden's earlier, undone move) is about an earlier move, and the pane drops it.
#[test]
fn a_read_asked_before_the_move_started_says_nothing_about_it() {
    let mut steps = Steps::default();
    let asked = steps.ticket();
    steps.move_started("north-box");
    assert!(!steps.settle(&published(), asked, NOW), "dropped");
    assert_eq!(steps.started.as_deref(), Some("north-box"));
    assert!(!steps.unanswered(asked, NOW));
    let down = DaemonProblem::NotRunning;
    assert!(
        !moving_of(&view(Some(Err(&down)), &steps, NOW + 600)).stalled,
        "a dropped read starts no wait"
    );
}

/// An engine that never comes back from the restart into a move is not a spinner for half an
/// hour: past `RESTART_GRACE` the pane says so and offers what the down page would (edges F14).
#[test]
fn an_engine_that_never_comes_back_is_said_after_its_grace() {
    let mut steps = Steps::default();
    steps.move_started("north-box");
    let ticket = steps.ticket();
    let grace = i64::try_from(RESTART_GRACE.as_secs()).unwrap();
    assert!(grace <= 300, "a short grace, not the visit's half hour");
    let down = DaemonProblem::NotRunning;
    assert!(steps.unanswered(ticket, NOW));
    assert!(
        steps.unanswered(ticket, NOW + 30),
        "the wait keeps its start"
    );
    let waiting = view(Some(Err(&down)), &steps, NOW + grace - 1);
    assert!(!moving_of(&waiting).stalled);
    let stalled = view(Some(Err(&down)), &steps, NOW + grace);
    let moving = moving_of(&stalled);
    assert!(moving.stalled);
    assert_eq!(moving.sentence, not_back("north-box"));
    assert_eq!(moving.problem, Some(DaemonProblem::NotRunning));
    let streaming = status(|s| {
        s["role"] = json!("device");
        s["move"] = json!({"phase": "streaming", "candidate": "north-box",
            "sentence": "This home is being sent to north-box."});
    });
    steps.settle(&streaming, ticket, NOW + grace + 5);
    steps.unanswered(ticket, NOW + grace + 10);
    assert!(
        !moving_of(&view(Some(Err(&down)), &steps, NOW + grace + 10)).stalled,
        "an answer starts the wait again"
    );
}

#[test]
fn an_undone_move_returns_to_the_setup_with_the_daemons_sentence() {
    let mut steps = Steps::default();
    steps.move_started("north-box");
    let undone = published();
    steps.settle(&undone, steps.ticket(), NOW);
    assert_eq!(steps.started, None);
    let view = drawn(&undone, &steps);
    assert_eq!(
        setup_of(&view).notice.as_deref(),
        Some("The move to north-box was undone; this home is still the primary.")
    );
}

/// Between the review and Move: the readiness report, read for the candidate under review. Move
/// waits for it and stays off while anything blocks (M75 fix plan, decision 2).
#[test]
fn the_readiness_report_stands_between_the_review_and_the_move() {
    let enrolled = status(|s| s["candidates"] = json!([candidate()]));
    let mut steps = Steps::default();
    steps.review(CANDIDATE_ID);
    assert!(steps.checking, "the review asks for its report");
    let review = |steps: &Steps| {
        setup_of(&drawn(&enrolled, steps))
            .review
            .clone()
            .expect("under review")
    };
    assert_eq!(review(&steps).readiness, Checked::Checking);
    assert!(!review(&steps).can_start, "not before the report");

    let blocked = readiness(|r| {
        r["ready"] = json!(false);
        r["items"][2] = json!({"id": "disk", "status": "block",
            "sentence": "The move needs about 4 GiB free here, and 1 GiB is free."});
    });
    steps.readiness_read(CANDIDATE_ID, Ok(blocked.clone()));
    assert!(!steps.checking);
    let shown = review(&steps);
    assert_eq!(shown.readiness, Checked::Answered(blocked.clone()));
    assert!(!shown.can_start, "an item blocks");
    assert_eq!(blocked.items[2].status.mark(), Some("Stops the move"));
    assert_eq!(blocked.items[5].status, ItemStatus::Warn);

    steps.review(CANDIDATE_ID);
    steps.readiness_read("cand-elsewhere", Ok(readiness(|_| {})));
    assert!(steps.checking, "still waiting for its own report");
    assert_eq!(
        review(&steps).readiness,
        Checked::Checking,
        "an answer for another candidate is dropped"
    );
    steps.readiness_read(CANDIDATE_ID, Ok(readiness(|_| {})));
    assert!(review(&steps).can_start, "nothing blocks");
    steps.starting = true;
    assert!(!review(&steps).can_start, "one start at a time");
    steps.starting = false;

    steps.review(CANDIDATE_ID);
    steps.readiness_read(
        CANDIDATE_ID,
        Err("This home holds no candidate by that id.".into()),
    );
    let shown = review(&steps);
    assert_eq!(
        shown.readiness,
        Checked::Unread("This home holds no candidate by that id.".into())
    );
    assert!(!shown.can_start);
}

#[test]
fn a_status_this_app_does_not_know_is_shown_without_a_mark() {
    let report = readiness(|r| r["items"][0]["status"] = json!("later"));
    assert_eq!(report.items[0].status, ItemStatus::Unknown);
    assert_eq!(report.items[0].status.mark(), None);
    assert_eq!(ItemStatus::Ok.mark(), Some("Checked"));
}

/// A primary names, in Recovery, each secret it could not resolve, and offers to set the recovery
/// passphrase while it has no recovery key (M75 review, edges F8).
#[test]
fn a_primary_names_its_missing_secrets_and_its_missing_recovery_key() {
    let published = published();
    let recovery = drawn(&published, &Steps::default()).recovery.unwrap();
    assert_eq!(recovery.secrets, ["anthropic_api_key"]);
    assert_eq!(recovery.secrets_note, SECRETS_HERE);
    assert!(!recovery.can_set_passphrase, "it has a recovery key");

    let keyless = status(|s| {
        s["snapshots"]["recipient_set"] = json!(false);
        s["snapshots"]["configured"] = json!(false);
    });
    let recovery = drawn(&keyless, &Steps::default()).recovery.unwrap();
    assert!(recovery.can_set_passphrase);
    assert!(!recovery.can_run, "nothing is configured to back up");

    let unlinked = status(|s| {
        s["role"] = json!("device");
        s["snapshots"]["recipient_set"] = json!(false);
    });
    let recovery = drawn(&unlinked, &Steps::default()).recovery.unwrap();
    assert!(!recovery.can_set_passphrase, "a device keeps no key");
}

/// On a desktop, Recovery and Back up now are the server's, from its own picture; the desktop's
/// retired status file is never shown (M75 fix plan, decision 3).
#[test]
fn a_linked_desktop_shows_the_servers_recovery() {
    let desktop = linked(Some(server_home(|_| {})));
    let recovery = drawn(&desktop, &Steps::default()).recovery.unwrap();
    assert_eq!(
        recovery.sentence,
        "The last snapshot was stored at 2026-10-27 09:30 UTC."
    );
    assert_eq!(recovery.targets[0].title, "S3");
    assert!(recovery.can_run, "Back up now reaches the server");
    assert_eq!(recovery.secrets, ["anthropic_api_key"]);
    assert_eq!(recovery.secrets_note, SECRETS_SERVER);
}

#[test]
fn a_linked_desktop_says_why_the_servers_recovery_is_missing() {
    let desktop = linked(None);
    let view = drawn(&desktop, &Steps::default());
    let recovery = view.recovery.clone().unwrap();
    assert_eq!(recovery.sentence, format!("{UNREACHABLE}\n{SERVER_AWAY}"));
    assert!(recovery.targets.is_empty() && recovery.next.is_none());
    assert!(!recovery.can_run && !recovery.can_set_passphrase);
    let shown = desktop_of(&view);
    assert_eq!(shown.server.state, NOT_CONNECTED);
    assert!(
        !shown.can_replace,
        "nothing to replace while the server cannot be read"
    );
    assert_eq!(
        desktop.move_from(),
        MoveFrom::TheServer,
        "a desktop never moves the server as if it were this computer"
    );
    assert_eq!(
        desktop.replacement_by(CANDIDATE_ID),
        None,
        "nothing to follow it by"
    );
    assert_eq!(
        published().move_from(),
        MoveFrom::ThisComputer,
        "a primary moves itself"
    );
    assert_eq!(published().replacement_by(CANDIDATE_ID), None);
}

/// Replacing the server from a desktop: a code and the candidate come from the server's own
/// picture, the move is the server's, and the desktop follows it to the server that takes over
/// (M75 fix plan, decision 4).
#[test]
fn a_desktop_replaces_the_server_and_follows_the_move_to_the_new_one() {
    let mut steps = Steps::default();
    let desktop = linked(Some(server_home(|_| {})));
    let shown = drawn(&desktop, &steps);
    assert!(desktop_of(&shown).can_replace);
    assert_eq!(desktop_of(&shown).replace, None, "closed until asked");
    assert_eq!(
        desktop_of(&shown).explain,
        "Fermix moves from north-box to a new server you choose, and north-box serves nothing \
         once the move is done. This computer stays a desktop, of the new server."
    );
    assert!(MOVE_BACK.starts_with("Moving Fermix back to this computer is not in this version."));

    steps.open_replace(true);
    steps.destination = Some(Destination::Aws);
    steps.code_opened(code("2026-10-27T10:10:00Z"));
    steps.review(CANDIDATE_ID);
    steps.readiness_read(CANDIDATE_ID, Ok(readiness(|_| {})));
    let shown = drawn(&desktop, &steps);
    let replace = desktop_of(&shown).replace.as_ref().expect("open");
    assert_eq!(replace.candidates[0].title, "north-box");
    assert!(replace.review.as_ref().expect("under review").can_start);

    assert_eq!(desktop.move_from(), MoveFrom::TheServer);
    assert_eq!(desktop.replacement_by("cand-elsewhere"), None);
    let replacing = desktop
        .replacement_by(CANDIDATE_ID)
        .expect("the server answers and holds it");
    assert_eq!(
        replacing,
        Replacing {
            to: "north-box".into(),
            from: "north-box".into(),
            epoch: Some(5),
        },
        "the golden's candidate shares the server's name"
    );
    steps.replace_started(Replacing {
        to: "south-box".into(),
        ..replacing
    });
    assert!(!steps.replace_open && steps.code().is_none() && steps.reviewing().is_none());
    let ticket = steps.ticket();
    let sending = linked(Some(server_home(|s| {
        s["move"] = json!({"phase": "streaming", "candidate": "south-box",
            "sentence": "This home is being sent to south-box."});
    })));
    steps.settle(&sending, ticket, NOW);
    assert_eq!(
        moving_of(&drawn(&sending, &steps)).sentence,
        "This home is being sent to south-box."
    );

    let away = linked(None);
    steps.settle(&away, ticket, NOW);
    let moving = drawn(&away, &steps);
    let moving = moving_of(&moving);
    assert_eq!(moving.sentence, moving_between("north-box", "south-box"));
    assert_eq!(moving.detail.as_deref(), Some(UNREACHABLE));
    assert!(!moving.stalled && moving.problem.is_none());
    let grace = i64::try_from(REPLACE_GRACE.as_secs()).unwrap();
    steps.settle(&away, ticket, NOW + grace);
    let stalled = view(Some(Ok(&away)), &steps, NOW + grace);
    assert!(moving_of(&stalled).stalled);
    assert_eq!(moving_of(&stalled).sentence, not_taken_over("south-box"));

    let taken_over = linked(Some(server_home(|s| s["epoch"] = json!(6))));
    steps.settle(&taken_over, ticket, NOW + grace + 3);
    assert_eq!(
        steps.replacing, None,
        "the server this computer reaches is the new one"
    );
    assert!(desktop_of(&drawn(&taken_over, &steps)).can_replace);
}

#[test]
fn an_undone_replacement_returns_to_the_desktop_with_the_servers_sentence() {
    let mut steps = Steps::default();
    steps.replace_started(Replacing {
        to: "south-box".into(),
        from: "north-box".into(),
        epoch: Some(5),
    });
    let undone = linked(Some(server_home(|s| {
        s["move"] = json!({"phase": "aborted", "candidate": "south-box",
            "sentence": "The move to south-box was undone; this home is still the primary."});
    })));
    steps.settle(&undone, steps.ticket(), NOW);
    assert_eq!(steps.replacing, None);
    steps.open_replace(true);
    let view = drawn(&undone, &steps);
    assert_eq!(
        desktop_of(&view)
            .replace
            .as_ref()
            .unwrap()
            .notice
            .as_deref(),
        Some("The move to south-box was undone; this home is still the primary.")
    );
}

#[test]
fn a_desktop_shows_its_server_unreachable_in_the_daemons_words() {
    let away = status(|s| {
        s["role"] = json!("device");
        s["primary"] = json!({"name": null, "connected": false, "since": null, "epoch": 5,
            "sentence": "The server is unreachable. Fermix keeps your message until it is back."});
        s["snapshots"]["configured"] = json!(false);
        s["snapshots"]["next_run_at"] = json!(null);
        s["snapshots"]["sentence"] = json!("Snapshots are the server's to take.");
    });
    let view = drawn(&away, &Steps::default());
    let server = &desktop_of(&view).server;
    assert_eq!(server.name, THE_SERVER);
    assert_eq!(server.state, NOT_CONNECTED);
    assert_eq!(
        server.sentence,
        "The server is unreachable. Fermix keeps your message until it is back."
    );
    let recovery = view.recovery.expect("recovery");
    assert!(!recovery.can_run, "a desktop takes no snapshot of its own");
    assert_eq!(recovery.next, None);
    assert_eq!(
        recovery.sentence, SERVER_AWAY,
        "no picture of the server and no sentence"
    );
}

#[test]
fn a_device_with_no_server_says_it_is_not_linked() {
    let unlinked = status(|s| s["role"] = json!("device"));
    assert_eq!(unlinked.role, Role::Device);
    assert_eq!(drawn(&unlinked, &Steps::default()).stage, Stage::Unlinked);
    assert!(UNLINKED.ends_with('.'));
}

#[test]
fn a_candidate_says_where_its_enrollment_stands_in_the_daemons_words() {
    let waiting = status(|s| {
        s["role"] = json!("candidate");
        s["move"] = json!({"phase": "enrolled", "candidate": null,
            "sentence": "Enrolled with studio-mac; waiting for its move until 2026-10-28T09:12:00Z."});
    });
    assert_eq!(
        drawn(&waiting, &Steps::default()).stage,
        Stage::Candidate(
            "Enrolled with studio-mac; waiting for its move until 2026-10-28T09:12:00Z.".into()
        )
    );
    let fresh = status(|s| s["role"] = json!("candidate"));
    assert_eq!(
        drawn(&fresh, &Steps::default()).stage,
        Stage::Candidate(CANDIDATE.into())
    );
}

#[test]
fn a_primary_lists_its_desktops_and_shows_a_switch_on_its_way() {
    let hub = status(|s| {
        s["desktops"] = json!([
            {"device_id": "c41e9d27", "name": "Studio Mac", "connected": true,
             "since": "2026-10-27T08:00:00Z", "executor": true},
            {"device_id": "77aa01bc", "name": "Tower", "connected": false,
             "since": null, "executor": false}
        ]);
    });
    let mut steps = Steps::default();
    let rows = drawn(&hub, &steps).desktops;
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].state, rows[0].executor), (CONNECTED, Some(true)));
    assert_eq!(
        (rows[1].state, rows[1].executor),
        (NOT_CONNECTED, Some(false))
    );
    assert!(!rows[0].switching);
    steps.executor = Some(("c41e9d27".into(), false));
    let rows = drawn(&hub, &steps).desktops;
    assert_eq!(
        rows[0].executor,
        Some(false),
        "the value asked for while it is on its way"
    );
    assert!(rows[0].switching);
    assert!(!rows[1].switching);
}

#[test]
fn a_target_with_nothing_verified_says_so_and_carries_its_error() {
    let failing = status(|s| {
        s["snapshots"]["targets"][0]["last_verified"] = json!(null);
        s["snapshots"]["targets"][0]["last_error"] = json!("S3 refused the upload: AccessDenied.");
        s["snapshots"]["running"] = json!(true);
    });
    let recovery = drawn(&failing, &Steps::default()).recovery.unwrap();
    assert_eq!(recovery.targets[0].verified, NOTHING_VERIFIED);
    assert_eq!(
        recovery.targets[0].error.as_deref(),
        Some("S3 refused the upload: AccessDenied.")
    );
    assert!(!recovery.can_run, "one is running");
    assert_eq!(recovery.next, None);
}

/// A desktop asleep is no failure: the target says the snapshot waits, in the daemon's words.
#[test]
fn a_waiting_target_says_so_without_an_error() {
    let waiting = status(|s| {
        s["snapshots"]["targets"][0]["last_result"] = json!("waiting");
        s["snapshots"]["targets"][0]["waiting"] =
            json!("Your Studio Mac is not connected; the snapshot waits for the next run.");
    });
    let recovery = drawn(&waiting, &Steps::default()).recovery.unwrap();
    assert_eq!(
        recovery.targets[0].waiting.as_deref(),
        Some("Your Studio Mac is not connected; the snapshot waits for the next run.")
    );
    assert_eq!(recovery.targets[0].error, None);
}

#[test]
fn nothing_read_yet_is_reading_and_a_down_daemon_is_its_problem() {
    let steps = Steps::default();
    assert_eq!(view(None, &steps, NOW).stage, Stage::Reading);
    let down = DaemonProblem::NotResponding;
    let shown = view(Some(Err(&down)), &steps, NOW);
    assert_eq!(shown.stage, Stage::Down(DaemonProblem::NotResponding));
    assert!(shown.recovery.is_none());
}

#[test]
fn times_read_in_their_largest_unit() {
    assert_eq!(span(30), "less than a minute");
    assert_eq!(span(60), "1 minute");
    assert_eq!(span(7_200), "2 hours");
    assert_eq!(span(90_000), "1 day");
    assert_eq!(
        expiry("2026-10-27T10:00:30Z", NOW),
        "Use it within a minute. It works once."
    );
    assert_eq!(expiry("2026-10-27T09:00:00Z", NOW), "It has expired.");
    assert_eq!(expiry("tomorrow", NOW), "It works once.");
}

/// The pane reads while it is on screen, and one visit stops after 30 minutes.
#[test]
fn polling_is_bounded() {
    let visit = POLL_EVERY * POLLS_PER_VISIT;
    assert_eq!(visit, Duration::from_secs(30 * 60));
}

/// The link's state in one word (`primary.status`): `renewing` is drawn quietly, as reconnecting,
/// and `revoked` and `stale` stand out with the daemon's sentence; an engine that sends no word is
/// read by `connected` (M75 cleanup, item 4).
#[test]
fn the_link_is_drawn_by_its_word() {
    let with = |status: Option<&str>, connected: bool| {
        let shown = drawn(
            &status_with_primary(status, connected, "The server north-box says so."),
            &Steps::default(),
        );
        desktop_of(&shown).server.clone()
    };
    let up = with(Some("connected"), true);
    assert_eq!((up.state, up.alarm), (CONNECTED, false));
    let renewing = with(Some("renewing"), false);
    assert_eq!(
        (renewing.state, renewing.alarm),
        (RECONNECTING, false),
        "quietly"
    );
    let away = with(Some("unreachable"), false);
    assert_eq!((away.state, away.alarm), (NOT_CONNECTED, false));
    let revoked = with(Some("revoked"), false);
    assert_eq!((revoked.state, revoked.alarm), (REMOVED, true));
    assert_eq!(revoked.sentence, "The server north-box says so.");
    let stale = with(Some("stale"), false);
    assert_eq!((stale.state, stale.alarm), (NOT_CONNECTED, true));
    assert_eq!(with(None, true).state, CONNECTED, "no word: by connected");
    assert_eq!(with(Some("later"), false).state, NOT_CONNECTED);
}

fn status_with_primary(word: Option<&str>, connected: bool, sentence: &str) -> HomeStatus {
    let mut primary = json!({"name": "north-box", "connected": connected, "since": null,
        "epoch": 5, "sentence": sentence});
    if let Some(word) = word {
        primary["status"] = json!(word);
    }
    status(|s| {
        s["role"] = json!("device");
        s["primary"] = primary;
    })
}

/// An engine that came back but could not record the ending keeps answering `committed` (or
/// `undoing`) with a sentence that says so; the pane shows that sentence without a spinner and
/// keeps following until the ending is written (M75 recheck, N5).
#[test]
fn a_move_whose_ending_was_not_recorded_says_so_without_waiting() {
    let mut steps = Steps::default();
    steps.move_started("north-box");
    let ticket = steps.ticket();
    let unrecorded = status(|s| {
        s["role"] = json!("device");
        s["move"] = json!({"phase": "committed", "candidate": "north-box",
            "sentence": "This home moved to north-box. Fermix could not record that here \
                (.move/phase.json: permission denied); it tries again each time this status is read."});
    });
    steps.settle(&unrecorded, ticket, NOW);
    assert_eq!(steps.started.as_deref(), Some("north-box"), "followed on");
    let shown = drawn(&unrecorded, &steps);
    let moving = moving_of(&shown);
    assert!(moving.sentence.contains("could not record that here"));
    assert!(!moving.waiting, "nothing to wait on but the next read");
    let streaming = status(|s| {
        s["role"] = json!("device");
        s["move"] = json!({"phase": "streaming", "candidate": "north-box",
            "sentence": "This home is being sent to north-box."});
    });
    assert!(moving_of(&drawn(&streaming, &steps)).waiting);
}

/// A desktop's forwarded start whose answer was lost is `unconfirmed`: no failure. The pane
/// follows the replacement with the daemon's sentence beside it until the server answers, and
/// a server that answers with no move at all means none ran (M75 recheck, R2).
#[test]
fn an_unconfirmed_start_is_followed_not_failed() {
    let sentence = "The server north-box did not confirm in time whether the move started. Its \
        state shows once the server answers again, and starting it again never starts a second \
        move.";
    let mut steps = Steps::default();
    let plan = linked(Some(server_home(|_| {})))
        .replacement_by(CANDIDATE_ID)
        .expect("a plan");
    steps.start_unconfirmed(Some(plan), "north-box", sentence);
    let ticket = steps.ticket();
    let away = linked(None);
    steps.settle(&away, ticket, NOW);
    let shown = drawn(&away, &steps);
    assert_eq!(moving_of(&shown).detail.as_deref(), Some(sentence));
    let sending = linked(Some(server_home(|s| {
        s["move"] = json!({"phase": "streaming", "candidate": "north-box",
            "sentence": "This home is being sent to north-box."});
    })));
    steps.settle(&sending, ticket, NOW);
    steps.settle(&away, ticket, NOW + 5);
    assert_eq!(
        moving_of(&drawn(&away, &steps)).detail.as_deref(),
        Some(UNREACHABLE),
        "once the server answered, its own word for why it is missing"
    );
    let mut steps = Steps::default();
    let plan = linked(Some(server_home(|_| {})))
        .replacement_by(CANDIDATE_ID)
        .expect("a plan");
    steps.start_unconfirmed(Some(plan), "north-box", sentence);
    let idle = linked(Some(server_home(|_| {})));
    steps.settle(&idle, steps.ticket(), NOW);
    assert_eq!(
        steps.replacing, None,
        "the server answers with no move: none ran"
    );

    let mut steps = Steps::default();
    steps.start_unconfirmed(None, "north-box", sentence);
    assert_eq!(steps.started.as_deref(), Some("north-box"));
    steps.unanswered(steps.ticket(), NOW);
    let down = DaemonProblem::NotRunning;
    assert_eq!(
        moving_of(&view(Some(Err(&down)), &steps, NOW))
            .detail
            .as_deref(),
        Some(sentence)
    );
}

/// The folders the server may read on this computer, each with when it was allowed.
#[test]
fn the_folders_the_server_may_read_say_when_they_were_allowed() {
    let grants: DeviceGrants = serde_json::from_value(golden("device_grants_list")).unwrap();
    let rows = grant_rows(&grants, NOW);
    assert_eq!(rows[0].path, "/Users/ana/Downloads");
    assert_eq!(rows[0].granted, "Allowed 30 minutes ago");
    assert_eq!(rows[1].granted, "Allowed less than a minute ago");
}
