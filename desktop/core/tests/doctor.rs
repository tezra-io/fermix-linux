//! Doctor: sessions decoded from the engine's own fixtures, and the pure rules
//! that order the rows, name them, and say which remediations the app can act on.

use fermix_client::doctor::{
    check_title, detail, grouped, headline, poll_cap, progress, summary_sentence, ActionKind,
    Check, CheckStatus, Fix, Group, Scope, Session, SessionStatus, Severity, Summary, Tone,
    MAX_POLLS, POLL_MS,
};
use fermix_client::frame::{read_frame, write_frame};
use fermix_client::management::{decode_response, CallError, Management};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::os::unix::net::UnixListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const SUCCESS: &str = include_str!("fixtures/management/success.jsonl");
const ERRORS: &str = include_str!("fixtures/management/errors.jsonl");

fn fixture(name: &str) -> Value {
    let found: Vec<Value> = SUCCESS
        .lines()
        .chain(ERRORS.lines())
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .filter(|v| v["name"] == name)
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one fixture named {name}");
    found.into_iter().next().unwrap()
}

fn answer(name: &str) -> Result<Value, CallError> {
    let response = &fixture(name)["response"];
    let id = response["request_id"].as_str().expect("fixture has an id");
    decode_response(&serde_json::to_vec(response).unwrap(), id)
}

fn decode<T: DeserializeOwned>(name: &str) -> T {
    let result = answer(name).expect("fixture is a success");
    serde_json::from_value(result).unwrap_or_else(|e| panic!("{name} does not decode: {e}"))
}

fn check(id: &str, status: &str, severity: &str) -> Check {
    serde_json::from_value(json!({
        "id": id, "category": "runtime", "severity": severity, "applicability": "always",
        "origin": "engine", "status": status, "summary": format!("{id} says so"),
        "evidence": {}, "remediation_code": null, "remediation": null,
        "duration_ms": 0, "finished_at": null
    }))
    .unwrap()
}

fn remedied(kind: &str, target: Value) -> Check {
    serde_json::from_value(json!({
        "id": "some_check", "severity": "warning", "status": "warning",
        "summary": "something to fix", "remediation_code": "some_check.warning",
        "remediation": {"title": "Do the thing", "body": "Why and how.",
                        "action": {"kind": kind, "target": target}}
    }))
    .unwrap()
}

fn session(status: &str, summary: Value) -> Session {
    serde_json::from_value(json!({
        "session_id": "doctor:AAAAAAAAAAAA", "scope": "local", "status": status,
        "budget_ms": 10000, "duration_ms": 5, "started_at": "2026-09-25T03:53:04Z",
        "finished_at": null, "total": 39, "completed_count": 12,
        "summary": summary, "checks": []
    }))
    .unwrap()
}

fn counts(failed: u32, warning: u32) -> Value {
    json!({"passed": 30, "warning": warning, "failed": failed, "not_applicable": 0,
           "unavailable": 0, "skipped": 0, "cancelled": 0, "timed_out": 0})
}

#[test]
fn a_started_session_is_running_with_no_rows_yet() {
    let started: Session = decode("doctor_start");
    assert_eq!(started.session_id, "doctor:9Fj2mQ7bT1xK");
    assert_eq!(started.status, SessionStatus::Running);
    assert_eq!(started.budget_ms, 10_000);
    assert!(started.checks.is_empty());
    assert_eq!(progress(&started), Some((0, 34)));
    assert_eq!(headline(&started), "Running checks");
    assert_eq!(detail(&started), "0 of 34 checks done");
}

#[test]
fn a_session_in_progress_carries_the_rows_that_landed() {
    let running: Session = decode("doctor_get_in_progress");
    assert_eq!(running.status, SessionStatus::Running);
    assert_eq!(progress(&running), Some((3, 34)));
    let ids: Vec<&str> = running.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["readiness", "daemon_socket", "restart_pending"]);
    let socket = &running.checks[1];
    assert_eq!(socket.status, CheckStatus::Warning);
    assert_eq!(socket.severity, Severity::Critical);
    assert_eq!(socket.summary, "not running (start with `fermix start`)");
    assert!(socket.remediation.is_none(), "a code without a remediation");
    assert_eq!(socket.fix(), None);
    let restart = &running.checks[2];
    assert_eq!(
        restart.remediation.as_ref().unwrap().action.kind,
        ActionKind::Restart
    );
    assert_eq!(restart.fix(), Some(Fix::Restart));
}

#[test]
fn a_cancelled_session_is_terminal_and_says_so() {
    let cancelled: Session = decode("doctor_cancel_terminal");
    assert_eq!(cancelled.status, SessionStatus::Cancelled);
    assert_eq!(cancelled.completed_count, cancelled.total);
    assert_eq!(cancelled.summary.cancelled, 3);
    assert_eq!(progress(&cancelled), None, "progress only while running");
    assert_eq!(headline(&cancelled), "Checks cancelled");
    let statuses: Vec<CheckStatus> = cancelled.checks.iter().map(|c| c.status).collect();
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == CheckStatus::Cancelled)
            .count(),
        3
    );
}

#[test]
fn busy_is_a_refusal_carrying_the_daemons_sentence() {
    let Err(CallError::Refused(busy)) = answer("busy_doctor") else {
        panic!("busy_doctor is a refusal");
    };
    assert_eq!(busy.code, "busy");
    assert_eq!(
        busy.sentence,
        "Another management operation of this kind is already running."
    );
}

#[test]
fn a_newer_daemons_statuses_kinds_and_fields_still_decode() {
    let row: Check = serde_json::from_value(json!({
        "id": "future_check", "severity": "urgent", "status": "deferred", "summary": "later",
        "sparkle": true, "remediation": {"title": "T", "body": "B",
        "action": {"kind": "teleport", "target": "moon", "extra": 1}}
    }))
    .unwrap();
    assert_eq!(row.status, CheckStatus::Unknown);
    assert_eq!(row.severity, Severity::Unknown);
    assert_eq!(
        row.remediation.as_ref().unwrap().action.kind,
        ActionKind::Other
    );
    assert_eq!(row.fix(), None);
    assert_eq!(row.status.verdict(), "Unknown");
    let paused = session("paused", counts(0, 0));
    assert_eq!(paused.status, SessionStatus::Unknown);
}

#[test]
fn the_title_is_the_id_with_its_underscores_opened_and_first_letter_raised() {
    assert_eq!(check_title("auth_token_expiry"), "Auth token expiry");
    assert_eq!(check_title("readiness"), "Readiness");
    assert_eq!(check_title("acp"), "Acp");
    assert_eq!(check_title(""), "");
}

#[test]
fn problems_come_first_worst_first_then_passed_then_not_run() {
    let rows = [
        check("a", "passed", "info"),
        check("b", "warning", "info"),
        check("c", "failed", "warning"),
        check("d", "cancelled", "critical"),
        check("e", "warning", "critical"),
        check("f", "unavailable", "info"),
        check("g", "not_applicable", "info"),
        check("h", "failed", "critical"),
        check("i", "passed", "critical"),
        check("j", "skipped", "info"),
        check("k", "timed_out", "info"),
    ];
    let groups: Vec<(Group, Vec<&str>)> = grouped(&rows)
        .into_iter()
        .map(|(g, checks)| (g, checks.iter().map(|c| c.id.as_str()).collect()))
        .collect();
    assert_eq!(
        groups,
        [
            (Group::Attention, vec!["h", "c", "f", "e", "b"]),
            (Group::Passed, vec!["a", "i"]),
            (Group::NotRun, vec!["d", "g", "j", "k"]),
        ]
    );
    assert_eq!(Group::Attention.title(), "Needs attention");
    assert_eq!(Group::Passed.title(), "Passed");
    assert_eq!(Group::NotRun.title(), "Not run");
}

#[test]
fn empty_groups_are_left_out() {
    let rows = [check("a", "passed", "info"), check("b", "passed", "info")];
    let groups: Vec<Group> = grouped(&rows).into_iter().map(|(g, _)| g).collect();
    assert_eq!(groups, [Group::Passed]);
    assert!(grouped(&[]).is_empty());
}

#[test]
fn every_status_has_a_verdict_and_a_tone() {
    let expect = [
        ("passed", "Passed", Tone::Good),
        ("warning", "Warning", Tone::Warn),
        ("failed", "Failed", Tone::Bad),
        ("unavailable", "Unavailable", Tone::Bad),
        ("not_applicable", "Not applicable", Tone::Quiet),
        ("skipped", "Skipped", Tone::Quiet),
        ("cancelled", "Cancelled", Tone::Quiet),
        ("timed_out", "Timed out", Tone::Quiet),
        ("deferred", "Unknown", Tone::Warn),
    ];
    for (wire, word, tone) in expect {
        let status = check("x", wire, "info").status;
        assert_eq!((status.verdict(), status.tone()), (word, tone), "{wire}");
    }
}

#[test]
fn the_banner_comes_from_the_summary_never_from_the_rows() {
    let say = |failed, warning| {
        let summary: Summary = serde_json::from_value(counts(failed, warning)).unwrap();
        summary_sentence(&summary)
    };
    assert_eq!(say(1, 3), "One check failed");
    assert_eq!(say(2, 0), "2 checks failed");
    assert_eq!(say(0, 1), "Healthy, with one thing to look at");
    assert_eq!(say(0, 4), "Healthy, with 4 things to look at");
    assert_eq!(say(0, 0), "Everything checks out");
}

#[test]
fn a_summary_missing_keys_counts_them_as_zero() {
    let summary: Summary = serde_json::from_value(json!({"failed": 2})).unwrap();
    assert_eq!(summary.failed, 2);
    assert_eq!(summary.warning, 0);
}

#[test]
fn each_ending_has_its_own_headline() {
    assert_eq!(
        headline(&session("completed", counts(0, 2))),
        "Healthy, with 2 things to look at"
    );
    assert_eq!(
        headline(&session("cancelled", counts(0, 0))),
        "Checks cancelled"
    );
    assert_eq!(
        headline(&session("timed_out", counts(0, 0))),
        "The checks ran out of time"
    );
    assert_eq!(
        headline(&session("failed", counts(0, 0))),
        "The checks stopped early"
    );
    assert_eq!(
        headline(&session("paused", counts(1, 0))),
        "One check failed"
    );
}

#[test]
fn the_detail_line_says_where_answers_come_from_or_what_a_cut_run_found() {
    let done = session("completed", counts(0, 0));
    assert_eq!(
        detail(&done),
        "Answers come from the running daemon, not from this app."
    );
    assert_eq!(
        detail(&session("cancelled", counts(1, 0))),
        "One check failed"
    );
    assert_eq!(
        detail(&session("timed_out", counts(0, 0))),
        "Nothing that ran needs attention."
    );
}

#[test]
fn a_settings_pane_remediation_opens_that_pane_and_only_a_known_one() {
    let providers = remedied("settings_pane", json!("providers"));
    let fix = providers.fix().unwrap();
    assert_eq!(
        fix,
        Fix::OpenPane {
            slug: "providers",
            title: "Providers"
        }
    );
    assert_eq!(fix.label(), "Open Providers");
    assert_eq!(remedied("settings_pane", json!("wormholes")).fix(), None);
    assert_eq!(remedied("settings_pane", Value::Null).fix(), None);
}

#[test]
fn remediation_kinds_map_to_what_the_app_can_do() {
    assert_eq!(remedied("restart", Value::Null).fix(), Some(Fix::Restart));
    assert_eq!(remedied("reload", Value::Null).fix(), Some(Fix::Reload));
    let legacy = remedied("instructions", json!("legacy_service_unit.removal"));
    assert_eq!(legacy.fix(), Some(Fix::ServiceSteps));
    // Linux has no Recovery screen yet; the row's own title and body say what to do.
    let recovery = remedied("instructions", json!("external_config_change.recovery"));
    assert_eq!(recovery.fix(), None);
    assert_eq!(remedied("instructions", json!("anything_else")).fix(), None);
    assert_eq!(remedied("system_settings", json!("privacy")).fix(), None);
    assert_eq!(remedied("job", json!("some_job")).fix(), None);
    assert_eq!(remedied("none", Value::Null).fix(), None);
    assert_eq!(Fix::Restart.label(), "Restart Fermix…");
    assert_eq!(Fix::Reload.label(), "Reload settings");
    assert_eq!(Fix::ServiceSteps.label(), "Show how");
}

#[test]
fn the_live_local_example_decodes_and_its_sign_in_row_opens_providers() {
    let live: Session = serde_json::from_value(json!({
        "session_id":"doctor:gH5iTtlCAxcf","scope":"local","status":"completed","budget_ms":10000,
        "duration_ms":122,"started_at":"2026-09-25T03:53:04.670064Z",
        "finished_at":"2026-09-25T03:53:04.792281Z","total":39,"completed_count":39,
        "summary":{"passed":37,"warning":2,"failed":0,"not_applicable":0,"unavailable":0,
                   "skipped":0,"cancelled":0,"timed_out":0},
        "checks":[
          {"id":"auth_token_expiry","category":"security","severity":"warning",
           "applicability":"always","origin":"engine","status":"warning",
           "summary":"stale, re-auth may be needed: anthropic_oauth",
           "evidence":{"source_name":"auth tokens","source_status":"warn"},
           "remediation_code":"auth_token_expiry.warning",
           "remediation":{"title":"Sign in again",
                          "body":"A saved sign-in is close to expiring, so turns may start failing soon.",
                          "action":{"kind":"settings_pane","target":"providers"}},
           "duration_ms":0,"finished_at":"2026-09-25T03:53:04Z"},
          {"id":"plugins","category":"capability","severity":"warning","applicability":"always",
           "origin":"engine","status":"warning","summary":"gmail: needs_client_config",
           "evidence":{"source_name":"plugins","source_status":"warn"},
           "remediation_code":"plugins.warning","remediation":null,"duration_ms":3,
           "finished_at":"2026-09-25T03:53:04Z"}]}))
    .unwrap();
    assert_eq!(headline(&live), "Healthy, with 2 things to look at");
    assert_eq!(live.checks[0].fix().unwrap().label(), "Open Providers");
    assert_eq!(live.checks[1].fix(), None);
}

#[test]
fn polling_is_bounded_by_the_budget_and_a_hard_cap() {
    assert_eq!(POLL_MS, 500);
    let local = poll_cap(10_000);
    assert!((10_000 / POLL_MS..=MAX_POLLS).contains(&local), "{local}");
    let network = poll_cap(30_000);
    assert!(
        (30_000 / POLL_MS..=MAX_POLLS).contains(&network),
        "{network}"
    );
    assert_eq!(poll_cap(3_600_000), MAX_POLLS);
    assert_eq!(MAX_POLLS, 80);
}

/// A daemon that answers exactly one call with `result`, and hands back the request.
fn one_exchange(result: Value) -> (Management, JoinHandle<Value>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request: Value = serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
        let reply = json!({"request_id": request["request_id"], "result": result});
        write_frame(&mut stream, &serde_json::to_vec(&reply).unwrap()).unwrap();
        request
    });
    let client = Management::new(socket, Duration::from_millis(500));
    (client, handle, dir)
}

#[test]
fn start_get_and_cancel_send_the_scope_and_session_id() {
    let view = fixture("doctor_start")["response"]["result"].clone();
    let (client, request, _dir) = one_exchange(view.clone());
    let started = client.doctor_start(Scope::Local).unwrap();
    assert_eq!(started.status, SessionStatus::Running);
    let sent = request.join().unwrap();
    assert_eq!(sent["method"], "doctor.start");
    assert_eq!(sent["params"], json!({"scope": "local"}));

    let (client, request, _dir) = one_exchange(view.clone());
    client.doctor_get("doctor:9Fj2mQ7bT1xK").unwrap();
    let sent = request.join().unwrap();
    assert_eq!(sent["method"], "doctor.get");
    assert_eq!(sent["params"], json!({"session_id": "doctor:9Fj2mQ7bT1xK"}));

    let (client, request, _dir) = one_exchange(view);
    client.doctor_cancel("doctor:9Fj2mQ7bT1xK").unwrap();
    let sent = request.join().unwrap();
    assert_eq!(sent["method"], "doctor.cancel");
    assert_eq!(Scope::Network.wire(), "network");
}

#[test]
fn a_session_of_the_wrong_shape_is_a_protocol_error() {
    let (client, request, _dir) = one_exchange(json!({"session_id": 5}));
    let answer = client.doctor_get("doctor:9Fj2mQ7bT1xK");
    request.join().unwrap();
    assert!(matches!(answer, Err(CallError::Protocol(_))), "{answer:?}");
}
