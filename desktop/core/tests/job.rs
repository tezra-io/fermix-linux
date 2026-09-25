//! A job is polled until it ends, and never longer than its own budget allows.

use fermix_client::job::{adoptable_sign_in, outcome, poll_cap, Outcome};
use fermix_client::model::JobView;
use serde_json::json;

fn job(status: &str, phase: Option<&str>, failure: Option<(&str, &str)>) -> JobView {
    serde_json::from_value(json!({
        "job_id": "job:1", "kind": "auth", "status": status, "phase": phase, "progress": null,
        "budget_ms": 300000, "started_at": "2026-09-24T00:00:00Z", "finished_at": null,
        "result": null,
        "failure": failure.map(|(code, sentence)| json!({"code": code, "sentence": sentence}))
    }))
    .unwrap()
}

#[test]
fn a_running_job_reports_its_phase() {
    assert_eq!(
        outcome(&job("running", Some("awaiting_browser"), None)),
        Outcome::Running(Some("awaiting_browser".into()))
    );
}

#[test]
fn a_completed_job_is_done() {
    assert_eq!(outcome(&job("completed", None, None)), Outcome::Completed);
}

#[test]
fn a_failed_job_carries_the_daemons_sentence() {
    let failed = job(
        "failed",
        Some("verifying"),
        Some(("refused", "The sign-in was declined.")),
    );
    assert_eq!(
        outcome(&failed),
        Outcome::Failed("The sign-in was declined.".into())
    );
}

#[test]
fn a_timed_out_job_carries_its_sentence_too() {
    let timed_out = job(
        "timed_out",
        Some("awaiting_browser"),
        Some(("timed_out", "No sign-in arrived in time.")),
    );
    assert_eq!(
        outcome(&timed_out),
        Outcome::Failed("No sign-in arrived in time.".into())
    );
}

#[test]
fn a_cancelled_job_is_cancelled() {
    assert_eq!(outcome(&job("cancelled", None, None)), Outcome::Cancelled);
}

#[test]
fn the_poll_cap_covers_the_budget_plus_a_margin_and_no_more() {
    // 300 s at 1 s per poll, plus 10 polls of margin for the daemon to record the timeout.
    assert_eq!(poll_cap(300_000, 1_000), 310);
    assert_eq!(poll_cap(15_000, 500), 40);
}

#[test]
#[should_panic(expected = "poll interval")]
fn a_zero_poll_interval_is_refused() {
    poll_cap(300_000, 0);
}

fn listed(id: &str, kind: &str, status: &str) -> JobView {
    let mut view = job(status, Some("awaiting_browser"), None);
    view.job_id = id.into();
    view.kind = kind.into();
    view
}

#[test]
fn a_busy_sign_in_adopts_the_one_running_sign_in_nobody_follows() {
    let jobs = [
        listed("job:1", "auth", "cancelled"),
        listed("job:2", "auth", "running"),
        listed("job:3", "auth_import", "running"),
    ];
    let adopted = adoptable_sign_in(&jobs, &[]).expect("one running sign-in");
    assert_eq!(adopted.job_id, "job:2");
}

#[test]
fn a_sign_in_the_app_already_follows_is_never_adopted_twice() {
    let jobs = [listed("job:2", "auth", "running")];
    assert_eq!(adoptable_sign_in(&jobs, &["job:2".into()]), None);
}

#[test]
fn two_unknown_sign_ins_cannot_be_told_apart() {
    let jobs = [
        listed("job:2", "auth", "running"),
        listed("job:4", "auth", "running"),
    ];
    assert_eq!(adoptable_sign_in(&jobs, &[]), None);
}
