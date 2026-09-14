//! One job, polled.
//!
//! One runner per job identifier, and one poller inside it. Dismissing an
//! ordinary view detaches the polling without cancelling the work; reopening
//! finds the run again through `job.list` rather than starting a second one.
//! Explicit cancel asks the daemon to stop. The poll is bounded twice: by the
//! daemon's own budget and by a hard ceiling of two thousand.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use crate::copy::Key;
use crate::management::types::{JobKind, JobListResult, JobParams, JobStatus, JobView};

use super::api::{accept, ask, ManagementApi, READ_DEADLINE};
use super::{spawn, Observers, Poller};

/// How often a job is re-read.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// The hard ceiling on polls, whatever the daemon's budget says.
pub const POLL_CAP: u32 = 2_000;
/// The polls added on top of the daemon's budget, so a job that finishes at its
/// deadline is still read once after it does.
const POLL_SLACK: u32 = 5;

/// The ten phase words, which the wire publishes as atoms.
pub fn phase_word(phase: &str) -> Option<Key> {
    Some(match phase {
        "calling" => Key::JobPhaseCalling,
        "binding" => Key::JobPhaseBinding,
        "awaiting_browser" => Key::JobPhaseAwaitingBrowser,
        "verifying" => Key::JobPhaseVerifying,
        "reading_keychain" => Key::JobPhaseReadingKeychain,
        "downloading" => Key::JobPhaseDownloading,
        "probing" => Key::JobPhaseProbing,
        "listing" => Key::JobPhaseListing,
        "sidecar_downloading" => Key::JobPhaseSidecarDownloading,
        "awaiting_sign_in" => Key::JobPhaseAwaitingSignIn,
        // A phase a newer daemon mints has no word here, so the row shows what
        // it was already showing rather than a stranger's.
        _ => return None,
    })
}

/// One job, and the poll that follows it.
pub struct JobRunner {
    api: Rc<dyn ManagementApi>,
    job: RefCell<Option<JobView>>,
    poller: Poller,
    detached: Cell<bool>,
    observers: Observers,
}

impl JobRunner {
    /// A runner with nothing to follow yet.
    pub fn new(api: Rc<dyn ManagementApi>) -> Rc<Self> {
        Rc::new(Self {
            api,
            job: RefCell::new(None),
            poller: Poller::new(),
            detached: Cell::new(false),
            observers: Observers::default(),
        })
    }

    /// Tell me when the job moves.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// The job as it was last read.
    pub fn job(&self) -> Option<JobView> {
        self.job.borrow().clone()
    }

    /// Whether this runner is polling.
    pub fn is_polling(&self) -> bool {
        self.poller.is_running()
    }

    /// How many polls it has performed.
    pub fn polls(&self) -> u32 {
        self.poller.ticks()
    }

    /// Whether the job has stopped, whichever way it stopped.
    pub fn is_terminal(&self) -> bool {
        self.job
            .borrow()
            .as_ref()
            .map(|job| job.status != JobStatus::Running)
            .unwrap_or(false)
    }

    /// Follow one job the daemon just answered with.
    pub fn adopt(self: &Rc<Self>, job: JobView) {
        let budget = job.budget_ms;
        let running = job.status == JobStatus::Running;
        self.job.replace(Some(job));
        self.detached.set(false);
        self.observers.notify();

        if !running {
            return;
        }

        let runner = Rc::clone(self);
        self.poller.start(POLL_INTERVAL, poll_cap(budget), move || {
            if runner.is_terminal() {
                return false;
            }
            let runner = Rc::clone(&runner);
            spawn(async move {
                runner.read_once().await;
            });
            true
        });
    }

    /// One `job.get`.
    pub async fn read_once(&self) {
        let Some(job_id) = self.job.borrow().as_ref().map(|job| job.job_id.clone()) else {
            return;
        };

        let issued = ask::<_, JobView>(
            self.api.as_ref(),
            "job.get",
            &JobParams { job_id },
            READ_DEADLINE,
        )
        .await;

        if let Some(Ok(job)) = accept(self.api.as_ref(), issued) {
            self.job.replace(Some(job));
            self.observers.notify();
        }
    }

    /// Stop polling without asking the daemon to stop the work. What dismissing
    /// an ordinary view does.
    pub fn detach(&self) {
        self.poller.stop();
        self.detached.set(true);
    }

    /// Whether this runner was detached from its job.
    pub fn is_detached(&self) -> bool {
        self.detached.get()
    }

    /// Find a job of one kind that this daemon is still running, and follow it.
    ///
    /// What reopening a surface does: the run a person started is found again
    /// rather than started a second time.
    pub async fn reattach(self: &Rc<Self>, kind: JobKind) -> bool {
        let issued = ask::<_, JobListResult>(
            self.api.as_ref(),
            "job.list",
            &serde_json::json!({}),
            READ_DEADLINE,
        )
        .await;

        let Some(Ok(list)) = accept(self.api.as_ref(), issued) else {
            return false;
        };

        // The newest run of that kind, which is the one a reopened surface is
        // looking for: `job.list` answers oldest first.
        let found = list.jobs.into_iter().rev().find(|job| job.kind == kind);

        match found {
            Some(job) => {
                self.adopt(job);
                true
            }
            None => false,
        }
    }

    /// Ask the daemon to stop the work. Cancelling a job that has already
    /// finished is not an error, and answers its terminal view.
    pub async fn cancel(&self) {
        self.poller.stop();

        let Some(job_id) = self.job.borrow().as_ref().map(|job| job.job_id.clone()) else {
            return;
        };

        let issued = ask::<_, JobView>(
            self.api.as_ref(),
            "job.cancel",
            &JobParams { job_id },
            READ_DEADLINE,
        )
        .await;

        if let Some(Ok(job)) = accept(self.api.as_ref(), issued) {
            self.job.replace(Some(job));
        }

        self.observers.notify();
    }
}

/// The cap for one job: the daemon's own budget in ticks, plus a little, and
/// never more than the published ceiling.
pub fn poll_cap(budget_ms: u64) -> u32 {
    let ticks = (budget_ms / POLL_INTERVAL.as_millis() as u64) as u32;
    ticks.saturating_add(POLL_SLACK).min(POLL_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_the_budget_and_never_more_than_the_ceiling() {
        assert_eq!(poll_cap(15_000), 20);
        assert_eq!(poll_cap(900_000), 905);
        assert_eq!(poll_cap(u64::MAX), POLL_CAP);
    }

    #[test]
    fn every_published_phase_has_a_word_and_a_newer_one_has_none() {
        for phase in [
            "calling",
            "binding",
            "awaiting_browser",
            "verifying",
            "reading_keychain",
            "downloading",
            "probing",
            "listing",
            "sidecar_downloading",
            "awaiting_sign_in",
        ] {
            let key = phase_word(phase).unwrap_or_else(|| panic!("{phase} has no word"));
            assert!(!crate::copy::text(key).is_empty());
        }

        assert_eq!(phase_word("a_phase_from_the_future"), None);
    }
}
