//! Reading a job's state, and how long the app may keep polling it.

use crate::model::{JobStatus, JobView};

/// Polls allowed past the job's own budget, for the daemon to record the timeout.
const MARGIN_POLLS: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still going; the phase is display copy for the current step.
    Running(Option<String>),
    Completed,
    /// Failed or timed out, in the daemon's own words.
    Failed(String),
    Cancelled,
}

pub fn outcome(job: &JobView) -> Outcome {
    match job.status {
        JobStatus::Running => Outcome::Running(job.phase.clone()),
        JobStatus::Completed => Outcome::Completed,
        JobStatus::Cancelled => Outcome::Cancelled,
        JobStatus::Failed | JobStatus::TimedOut => {
            let failure = job
                .failure
                .as_ref()
                .expect("a failed or timed-out job carries its failure");
            Outcome::Failed(failure.sentence.clone())
        }
    }
}

/// The most polls a job with this budget can need; past it, the app gives up.
pub fn poll_cap(budget_ms: u64, interval_ms: u64) -> u64 {
    assert!(interval_ms > 0, "poll interval must be positive");
    budget_ms.div_ceil(interval_ms) + MARGIN_POLLS
}

/// The running sign-in a `busy` refusal points at: the one browser sign-in
/// (`auth`) still running that the app is not following already. Jobs do not
/// name their provider, so with two unknown ones there is nothing to adopt.
pub fn adoptable_sign_in(jobs: &[JobView], followed: &[String]) -> Option<JobView> {
    let mut unknown = jobs.iter().filter(|job| {
        job.kind == "auth" && job.status == JobStatus::Running && !followed.contains(&job.job_id)
    });
    let only = unknown.next()?;
    match unknown.next() {
        Some(_) => None,
        None => Some(only.clone()),
    }
}
