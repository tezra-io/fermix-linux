//! Computer use, as data.
//!
//! The verdict is the daemon's: `computer_use.permissions.get` says whether the
//! helper is installed and which of the two rights it holds, and it never
//! prompts. It is read on pane open and on the explicit Refresh, and nowhere
//! else.
//!
//! What this session knows about itself is used for one thing only: choosing
//! which finished sentence explains a machine the helper cannot be installed on
//! at all. The protocol this build speaks publishes `installed`, the two grants
//! and the time they were read, and no platform or display server, so the three
//! statements of the design are keyed on facts this process holds first hand —
//! the architecture it was compiled for and the session the desktop put it in —
//! and the session is named in the sentence so a person can check it. Nothing
//! here decides that a right is held: only the probe does that.

use std::cell::RefCell;
use std::rc::Rc;

use crate::copy::Key;
use crate::management::types::{CapabilitiesInstallParams, ComputerUsePermissions, JobView};
use crate::management::vocabulary::COMPUTER_USE_SIDECAR_TARGET;
use crate::session::DesktopFacts;

use super::api::{accept, ask, WRITE_DEADLINE};
use super::jobs::JobRunner;
use super::ledger::PermissionLedger;
use super::settings_model::{Sentence, SettingsModel};
use super::Observers;

/// The section the daemon publishes the computer-use rows under.
pub const SECTION: &str = "computer_use";
/// The row that turns the feature on.
pub const ENABLED_KEY: &str = "computer_use_enabled";

/// The architecture this build was compiled for. A fact of this binary.
const ARCH: &str = std::env::consts::ARCH;
/// The architecture the helper has no build for.
const ARM64: &str = "aarch64";
/// The session type the helper cannot drive.
const WAYLAND: &str = "wayland";

/// What the pane says about this machine, in the order the design resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// The helper has no build for this architecture, so there is nothing to
    /// install.
    NoBuildForArchitecture,
    /// There is no display server running at all.
    NoGraphicalSession,
    /// A session the helper cannot drive, named so the sentence is checkable.
    UnsupportedSession(String),
    /// The helper can be installed here and is not installed.
    Installable,
    /// The helper is installed, and the two rights are the probe's answer.
    Installed,
    /// Nothing has been read yet, so nothing is said.
    Unread,
}

impl Standing {
    /// The statement for a machine the helper cannot run on.
    pub fn statement(&self) -> Option<Key> {
        match self {
            Standing::NoBuildForArchitecture => Some(Key::ComputerArm64),
            Standing::NoGraphicalSession => Some(Key::ComputerNoGraphicalSession),
            Standing::UnsupportedSession(_) => Some(Key::ComputerWaylandSession),
            Standing::Installable => Some(Key::ComputerInstallStatement),
            Standing::Installed | Standing::Unread => None,
        }
    }

    /// Whether the pane offers to install anything.
    pub fn offers_install(&self) -> bool {
        matches!(self, Standing::Installable)
    }
}

/// Computer use, as one surface reads it.
pub struct ComputerModel {
    settings: Rc<SettingsModel>,
    ledger: Rc<PermissionLedger>,
    facts: RefCell<DesktopFacts>,
    install: Rc<JobRunner>,
    refusal: RefCell<Option<Sentence>>,
    observers: Observers,
}

impl ComputerModel {
    /// A computer model over the one settings model and the one ledger.
    pub fn new(settings: Rc<SettingsModel>, ledger: Rc<PermissionLedger>) -> Rc<Self> {
        let api = settings.api();
        Rc::new(Self {
            settings,
            ledger,
            facts: RefCell::new(DesktopFacts::default()),
            install: JobRunner::new(api),
            refusal: RefCell::new(None),
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

    /// The one ledger, which this pane and Permissions both read.
    pub fn ledger(&self) -> Rc<PermissionLedger> {
        Rc::clone(&self.ledger)
    }

    /// The daemon's own sentence for the last refused action.
    pub fn refusal(&self) -> Option<Sentence> {
        self.refusal.borrow().clone()
    }

    /// Read this pane's section, this session's own facts and the probe.
    pub async fn refresh(&self) {
        self.settings.refresh_section(SECTION).await;
        self.facts
            .replace(crate::session::DesktopSession::observe().await);
        self.ledger.refresh().await;
        self.observers.notify();
    }

    /// The probe, as it was last taken.
    pub fn permissions(&self) -> Option<ComputerUsePermissions> {
        self.ledger.permissions()
    }

    /// Where this machine stands.
    pub fn standing(&self) -> Standing {
        let Some(permissions) = self.permissions() else {
            return Standing::Unread;
        };
        if permissions.installed {
            return Standing::Installed;
        }

        // Nothing below decides a capability: the helper is not installed, and
        // these are the reasons it cannot be.
        if ARCH == ARM64 {
            return Standing::NoBuildForArchitecture;
        }

        let facts = self.facts.borrow();
        if !facts.display_present {
            return Standing::NoGraphicalSession;
        }
        match facts.session_type.as_deref() {
            Some(session) if session.eq_ignore_ascii_case(WAYLAND) => {
                Standing::UnsupportedSession(session.to_string())
            }
            _ => Standing::Installable,
        }
    }

    /// Whether the feature is on, as the daemon's own row says.
    pub fn enabled(&self) -> bool {
        self.settings
            .state()
            .rows(SECTION)
            .iter()
            .find(|row| row.key == ENABLED_KEY)
            .map(|row| {
                matches!(
                    row.value,
                    crate::management::types::SettingValue::Toggle(true)
                )
            })
            .unwrap_or(false)
    }

    /// Turn the feature off.
    pub async fn disable(&self) {
        self.settings
            .apply(
                SECTION,
                ENABLED_KEY,
                crate::management::types::SettingValue::Toggle(false),
            )
            .await;
        self.observers.notify();
    }

    /// Turn the feature on, installing the helper first where it is absent.
    pub async fn enable(&self) -> Result<(), Sentence> {
        if matches!(self.standing(), Standing::Installed) {
            self.settings
                .apply(
                    SECTION,
                    ENABLED_KEY,
                    crate::management::types::SettingValue::Toggle(true),
                )
                .await;
            self.observers.notify();
            return Ok(());
        }

        self.start_install().await
    }

    /// `capabilities.install.start {target: "computer_use_sidecar"}`.
    pub async fn start_install(&self) -> Result<(), Sentence> {
        let params = CapabilitiesInstallParams {
            target: COMPUTER_USE_SIDECAR_TARGET.to_string(),
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
            Some(Err(error)) => {
                let sentence = Sentence::of(&error);
                self.refusal.replace(Some(sentence.clone()));
                self.observers.notify();
                Err(sentence)
            }
        }
    }

    /// The install has reached a terminal outcome.
    pub async fn install_finished(&self) -> Result<(), Sentence> {
        let job = self.install.job();
        let failure = job.and_then(|job| job.failure).map(|failure| Sentence {
            code: None,
            text: failure.sentence,
        });

        self.ledger.refresh().await;
        self.observers.notify();

        match failure {
            Some(sentence) => {
                self.refusal.replace(Some(sentence.clone()));
                Err(sentence)
            }
            None => {
                self.settings
                    .apply(
                        SECTION,
                        ENABLED_KEY,
                        crate::management::types::SettingValue::Toggle(true),
                    )
                    .await;
                Ok(())
            }
        }
    }

    /// Take the probe again, which is what the Refresh button does.
    pub async fn reprobe(&self) {
        self.ledger.refresh().await;
        self.observers.notify();
    }

    /// This session's own facts, which name the session in the statement.
    pub fn facts(&self) -> DesktopFacts {
        self.facts.borrow().clone()
    }
}

/// One probe result, read as the word a person sees.
pub fn verdict(held: Option<bool>) -> Key {
    match held {
        Some(true) => Key::ComputerVerdictAvailable,
        Some(false) => Key::ComputerVerdictUnavailable,
        None => Key::ComputerProbeUnread,
    }
}
