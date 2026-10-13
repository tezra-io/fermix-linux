//! Meetings and Computer flows. Turning either on installs its helper first and
//! applies the switch only if the install completed (spec §6.1, M38 §5.7);
//! turning it off is a plain write that uninstalls nothing. Detection and the
//! probe are read when the pane opens, on Refresh, and after a job ends.

use crate::app::App;
use crate::capability_panes::{Running, COMPUTER_ENABLED, MEETINGS_ENABLED, MEETINGS_SIGN_IN};
use fermix_client::capabilities::{COMPUTER_SIDECAR, MEETBOT};
use fermix_client::job::{outcome, poll_cap, Outcome};
use fermix_client::management::CallError;
use fermix_client::model::JobView;
use gtk::glib;
use serde_json::Value;
use std::rc::Rc;
use std::time::Duration;

const POLL_MS: u64 = 1_000;
const NO_ANSWER: &str = "Fermix did not answer, so nothing changed.";

/// The settings row a capability's switch writes.
fn switch_row(target: &str) -> (&'static str, &'static str) {
    match target {
        MEETBOT => ("meetings", MEETINGS_ENABLED),
        COMPUTER_SIDECAR => ("computer_use", COMPUTER_ENABLED),
        other => panic!("no capability switch for {other}"),
    }
}

fn sentence(e: &CallError) -> String {
    match e {
        CallError::Refused(r) => r.sentence.clone(),
        _ => NO_ANSWER.to_owned(),
    }
}

impl App {
    pub async fn capability_switch(self: Rc<Self>, target: String, on: bool) {
        let (section, key) = switch_row(&target);
        self.settings_data.borrow_mut().job_failures.remove(&target);
        if !on {
            return self
                .apply_setting(section.into(), key.into(), Value::Bool(false))
                .await;
        }
        let name = target.clone();
        let started = self.daemon.call(move |m| m.capability_install(&name)).await;
        let ended = match started {
            Ok(job) => self.follow_job(&target, job).await,
            Err(e) => Outcome::Failed(sentence(&e)),
        };
        if target == MEETBOT {
            self.detect_meetbot().await;
        }
        match ended {
            Outcome::Completed => {
                self.apply_setting(section.into(), key.into(), Value::Bool(true))
                    .await
            }
            Outcome::Failed(why) => self.job_failed(&target, why),
            Outcome::Cancelled | Outcome::Running(_) => self.render(),
        }
    }

    pub async fn meetings_sign_in(self: Rc<Self>) {
        let started = self.daemon.call(|m| m.meetings_sign_in()).await;
        let ended = match started {
            Ok(job) => self.follow_job(MEETINGS_SIGN_IN, job).await,
            Err(e) => Outcome::Failed(sentence(&e)),
        };
        if let Outcome::Failed(why) = ended {
            self.shell.toast(&why);
        }
        self.detect_meetbot().await;
    }

    /// Polls a job until it ends, showing its phase on the pane. Bounded by
    /// the job's own budget; past it the job counts as not finished.
    async fn follow_job(&self, name: &str, started: JobView) -> Outcome {
        let cap = poll_cap(started.budget_ms, POLL_MS);
        let mut view = started;
        for _ in 0..cap {
            let phase = match outcome(&view) {
                Outcome::Running(phase) => phase,
                ended => return self.job_ended(name, ended),
            };
            self.settings_data.borrow_mut().jobs.insert(
                name.to_owned(),
                Running {
                    job_id: view.job_id.clone(),
                    phase,
                },
            );
            self.render();
            glib::timeout_future(Duration::from_millis(POLL_MS)).await;
            let id = view.job_id.clone();
            view = match self.daemon.call(move |m| m.job_get(&id)).await {
                Ok(next) => next,
                Err(e) => return self.job_ended(name, Outcome::Failed(sentence(&e))),
            };
        }
        self.job_ended(name, Outcome::Failed("It did not finish in time.".into()))
    }

    fn job_ended(&self, name: &str, ended: Outcome) -> Outcome {
        self.settings_data.borrow_mut().jobs.remove(name);
        ended
    }

    fn job_failed(&self, target: &str, why: String) {
        glib::g_warning!("fermix", "{target} did not install: {why}");
        self.settings_data
            .borrow_mut()
            .job_failures
            .insert(target.to_owned(), why);
        self.render();
    }

    pub async fn cancel_job(&self, job_id: String) {
        let id = job_id.clone();
        if let Err(e) = self.daemon.call(move |m| m.job_cancel(&id)).await {
            glib::g_warning!("fermix", "job {job_id} could not be cancelled: {e:?}");
            self.shell.toast(&sentence(&e));
        }
    }

    /// Reads the notetaker's detection. A refused probe is unanswered, never signed out.
    pub async fn detect_meetbot(&self) {
        let answer = self.daemon.call(|m| m.detect(&[MEETBOT])).await;
        let row = match answer {
            Ok(found) => found.results.into_iter().find(|r| r.target == MEETBOT),
            Err(e) => {
                glib::g_warning!("fermix", "meetbot detection failed: {e:?}");
                None
            }
        };
        self.settings_data.borrow_mut().meetbot = row;
        self.render();
    }

    pub async fn probe_computer(&self) {
        let answer = self.daemon.call(|m| m.computer_permissions()).await;
        let probe = answer.map_err(|e| {
            glib::g_warning!("fermix", "computer_use.permissions.get failed: {e:?}");
            sentence(&e)
        });
        self.settings_data.borrow_mut().probe = Some(probe);
        self.render();
    }
}
