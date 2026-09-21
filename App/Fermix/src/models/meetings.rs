//! Meetings, as data.
//!
//! Two jobs and one detection. Turning the feature on for the first time
//! installs the notetaker and its browser, and the switch is only written once
//! the install has actually landed; signing in is the notetaker's own one-time
//! browser hop. Both re-probe `meetbot` when they reach any terminal outcome,
//! including a failure and a cancel, and the probe lands on the one settings
//! model so a pane that was closed in the meantime still sees the answer.
//!
//! Sign-in is state rather than a permanent button, and it is never inferred: a
//! missing `signed_in`, an absent notetaker and a refused probe are all "not
//! answered", which is a different row from "signed out".

use std::rc::Rc;

use crate::management::types::{
    CapabilitiesInstallParams, DetectResult, DetectTarget, JobStatus, JobView, SettingValue,
};
use crate::management::vocabulary::MEETBOT_TARGET;

use super::api::{accept, ask, WRITE_DEADLINE};
use super::jobs::JobRunner;
use super::settings_model::{Sentence, SettingsModel};
use super::Observers;

/// The section the daemon publishes the meeting rows under.
pub const SECTION: &str = "meetings";
/// The row that turns the feature on.
pub const ENABLED_KEY: &str = "meetings_enabled";

/// Where the notetaker's Google sign-in stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInState {
    /// Signed in, with the daemon's own sentence for the account.
    SignedIn(Option<String>),
    /// Present and signed out, with the daemon's own detail where it gave one.
    SignedOut(Option<String>),
    /// Nothing can be said: the notetaker is absent, the probe was refused, or
    /// the daemon reported no answer at all. Not the same row as signed out.
    Unanswered,
}

impl SignInState {
    /// One detection, read.
    pub fn of(detection: Option<&DetectResult>) -> Self {
        let Some(detection) = detection else {
            return SignInState::Unanswered;
        };
        if !detection.present {
            return SignInState::Unanswered;
        }

        match detection.signed_in {
            Some(true) => SignInState::SignedIn(detection.detail.clone()),
            Some(false) => SignInState::SignedOut(detection.detail.clone()),
            None => SignInState::Unanswered,
        }
    }

    /// Whether there is an account to act on at all.
    pub fn is_answered(&self) -> bool {
        !matches!(self, SignInState::Unanswered)
    }
}

/// Meetings, as one surface reads them.
pub struct MeetingsModel {
    settings: Rc<SettingsModel>,
    install: Rc<JobRunner>,
    sign_in: Rc<JobRunner>,
    observers: Observers,
}

impl MeetingsModel {
    /// A meetings model over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let api = settings.api();
        Rc::new(Self {
            settings,
            install: JobRunner::new(Rc::clone(&api)),
            sign_in: JobRunner::new(api),
            observers: Observers::default(),
        })
    }

    /// Tell me when something this surface draws moves.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// The install job.
    pub fn install_job(&self) -> Rc<JobRunner> {
        Rc::clone(&self.install)
    }

    /// The sign-in job.
    pub fn sign_in_job(&self) -> Rc<JobRunner> {
        Rc::clone(&self.sign_in)
    }

    /// Read this pane's section and the notetaker probe.
    pub async fn refresh(&self) {
        self.settings.refresh_section(SECTION).await;
        self.probe().await;
    }

    /// `setup.detect {targets: ["meetbot"]}`.
    ///
    /// Taken on pane entry and after either job reaches a terminal outcome. The
    /// answer lands on the shared model, so it is there whether or not this
    /// pane is still on screen.
    pub async fn probe(&self) {
        self.settings
            .refresh_detections(&[DetectTarget::Meetbot])
            .await;
        self.observers.notify();
    }

    /// Where the sign-in stands, as the last probe answered.
    pub fn sign_in_state(&self) -> SignInState {
        SignInState::of(self.settings.state().detection(DetectTarget::Meetbot))
    }

    /// Whether the feature is on, as the daemon's own row says.
    pub fn enabled(&self) -> bool {
        self.settings
            .state()
            .rows(SECTION)
            .iter()
            .find(|row| row.key == ENABLED_KEY)
            .map(|row| matches!(row.value, SettingValue::Toggle(true)))
            .unwrap_or(false)
    }

    /// Whether the notetaker is on this machine, as the last probe answered.
    pub fn present(&self) -> bool {
        self.settings
            .state()
            .detection(DetectTarget::Meetbot)
            .map(|detection| detection.present)
            .unwrap_or(false)
    }

    /// Turn the feature off, which is one ordinary write.
    pub async fn disable(&self) {
        self.settings
            .apply(SECTION, ENABLED_KEY, SettingValue::Toggle(false))
            .await;
        self.observers.notify();
    }

    /// Turn the feature on.
    ///
    /// The notetaker is installed first where it is absent, and the switch is
    /// written only once that install has landed: a feature enabled over an
    /// install that failed is a switch reading on with nothing behind it.
    pub async fn enable(&self) -> Result<(), Sentence> {
        if self.present() {
            self.settings
                .apply(SECTION, ENABLED_KEY, SettingValue::Toggle(true))
                .await;
            self.observers.notify();
            return Ok(());
        }

        self.start_install().await
    }

    /// `capabilities.install.start {target: "meetbot"}`.
    pub async fn start_install(&self) -> Result<(), Sentence> {
        let params = CapabilitiesInstallParams {
            target: MEETBOT_TARGET.to_string(),
        };
        let issued = ask::<_, JobView>(
            self.settings.api().as_ref(),
            "capabilities.install.start",
            &params,
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Ok(()),
            Some(Ok(job)) => {
                self.install.adopt(job);
                self.observers.notify();
                Ok(())
            }
            Some(Err(error)) => Err(Sentence::of(&error)),
        }
    }

    /// The install has reached a terminal outcome.
    ///
    /// A run that worked is followed by the write the switch asked for; one that
    /// did not writes nothing and keeps the daemon's own sentence. Either way
    /// the notetaker is probed again, because the install moved it.
    pub async fn install_finished(&self) -> Result<(), Sentence> {
        let job = self.install.job();
        let failed = job
            .as_ref()
            .map(|job| job.status != JobStatus::Completed)
            .unwrap_or(true);

        if failed {
            let sentence = job
                .and_then(|job| job.failure)
                .map(|failure| Sentence {
                    code: None,
                    text: failure.sentence,
                    reason: None,
                })
                .unwrap_or_else(|| Sentence {
                    code: None,
                    text: crate::copy::text(crate::copy::Key::JobCancelled),
                    reason: None,
                });
            self.probe().await;
            return Err(sentence);
        }

        self.settings
            .apply(SECTION, ENABLED_KEY, SettingValue::Toggle(true))
            .await;
        self.probe().await;
        Ok(())
    }

    /// `meetings.signin.start`: the notetaker's own one-time browser hop.
    pub async fn start_sign_in(&self) -> Result<(), Sentence> {
        let issued = ask::<_, JobView>(
            self.settings.api().as_ref(),
            "meetings.signin.start",
            &serde_json::json!({}),
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Ok(()),
            Some(Ok(job)) => {
                self.sign_in.adopt(job);
                self.observers.notify();
                Ok(())
            }
            Some(Err(error)) => Err(Sentence::of(&error)),
        }
    }

    /// The sign-in has reached a terminal outcome, whichever one.
    ///
    /// Job success is never taken for an account: the probe is what answers.
    pub async fn sign_in_finished(&self) {
        self.probe().await;
    }
}
