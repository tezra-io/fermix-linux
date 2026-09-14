//! The permission ledger.
//!
//! One ledger feeds Permissions, Voice and Computer, so the three cannot
//! disagree about the same right. Its content is M38 section 7.4's table: the
//! principal that holds each right, how a person takes it back, and where the
//! durable artifact actually lives. The table is rendered, never restated.
//!
//! Two rights have a live state and the daemon owns it:
//! `computer_use.permissions.get` answers for screen capture and input control,
//! it never prompts, and it is asked on pane open and on Refresh only. Two more
//! have a state this process knows first hand: whether the background service
//! is enabled, which the command line reports, and whether the autostart entry
//! exists, which is a file in the person's own home. The rest have no state at
//! all, which is the honest answer: nothing was granted, so there is nothing to
//! report.

use std::cell::RefCell;
use std::rc::Rc;

use crate::copy::Key;
use crate::management::types::ComputerUsePermissions;

use super::api::{accept, ask, READ_DEADLINE};
use super::settings_model::{Sentence, SettingsModel};
use super::Observers;

/// One right the product needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Right {
    Microphone,
    ScreenCapture,
    InputSynthesis,
    BackgroundService,
    Autostart,
    Privileged,
    BrowserAutomation,
}

/// Every right, in the order the table lists them.
pub const RIGHTS: &[Right] = &[
    Right::Microphone,
    Right::ScreenCapture,
    Right::InputSynthesis,
    Right::BackgroundService,
    Right::Autostart,
    Right::Privileged,
    Right::BrowserAutomation,
];

impl Right {
    /// What the right is called.
    pub fn title(self) -> Key {
        match self {
            Right::Microphone => Key::PermissionMicrophoneTitle,
            Right::ScreenCapture => Key::PermissionScreenCaptureTitle,
            Right::InputSynthesis => Key::PermissionInputSynthesisTitle,
            Right::BackgroundService => Key::PermissionBackgroundServiceTitle,
            Right::Autostart => Key::PermissionAutostartTitle,
            Right::Privileged => Key::PermissionPrivilegedTitle,
            Right::BrowserAutomation => Key::PermissionBrowserTitle,
        }
    }

    /// Who grants it.
    pub fn principal(self) -> Key {
        match self {
            Right::Microphone => Key::PermissionMicrophonePrincipal,
            Right::ScreenCapture => Key::PermissionScreenCapturePrincipal,
            Right::InputSynthesis => Key::PermissionInputSynthesisPrincipal,
            Right::BackgroundService => Key::PermissionBackgroundServicePrincipal,
            Right::Autostart => Key::PermissionAutostartPrincipal,
            Right::Privileged => Key::PermissionPrivilegedPrincipal,
            Right::BrowserAutomation => Key::PermissionBrowserPrincipal,
        }
    }

    /// How a person takes it back.
    pub fn revocation(self) -> Key {
        match self {
            Right::Microphone => Key::PermissionMicrophoneRevoke,
            Right::ScreenCapture => Key::PermissionScreenCaptureRevoke,
            Right::InputSynthesis => Key::PermissionInputSynthesisRevoke,
            Right::BackgroundService => Key::PermissionBackgroundServiceRevoke,
            Right::Autostart => Key::PermissionAutostartRevoke,
            Right::Privileged => Key::PermissionPrivilegedRevoke,
            Right::BrowserAutomation => Key::PermissionBrowserRevoke,
        }
    }

    /// Where the durable artifact lives.
    pub fn artifact(self) -> Key {
        match self {
            Right::Microphone => Key::PermissionMicrophoneArtifact,
            Right::ScreenCapture => Key::PermissionScreenCaptureArtifact,
            Right::InputSynthesis => Key::PermissionInputSynthesisArtifact,
            Right::BackgroundService => Key::PermissionBackgroundServiceArtifact,
            Right::Autostart => Key::PermissionAutostartArtifact,
            Right::Privileged => Key::PermissionPrivilegedArtifact,
            Right::BrowserAutomation => Key::PermissionBrowserArtifact,
        }
    }

    /// Whether the daemon is what answers for this right.
    pub fn answered_by_daemon(self) -> bool {
        matches!(self, Right::ScreenCapture | Right::InputSynthesis)
    }
}

/// One row of the ledger: the right, and what is known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    pub right: Right,
    /// The word for where it stands, where anything is known. Nothing is not
    /// "off": it is a right with no state to report.
    pub standing: Option<Key>,
}

/// The one ledger.
pub struct PermissionLedger {
    settings: Rc<SettingsModel>,
    permissions: RefCell<Option<ComputerUsePermissions>>,
    refusal: RefCell<Option<Sentence>>,
    observers: Observers,
}

impl PermissionLedger {
    /// A ledger over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        Rc::new(Self {
            settings,
            permissions: RefCell::new(None),
            refusal: RefCell::new(None),
            observers: Observers::default(),
        })
    }

    /// Tell me when the ledger moves.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// `computer_use.permissions.get`, which never prompts.
    ///
    /// Called on pane open and on Refresh only. A refused probe keeps the
    /// daemon's own sentence and claims nothing about the two rights.
    pub async fn refresh(&self) {
        let issued = ask::<_, ComputerUsePermissions>(
            self.settings.api().as_ref(),
            "computer_use.permissions.get",
            &serde_json::json!({}),
            READ_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => return,
            Some(Ok(permissions)) => {
                self.permissions.replace(Some(permissions));
                self.refusal.replace(None);
            }
            Some(Err(error)) => {
                self.permissions.replace(None);
                self.refusal.replace(Some(Sentence::of(&error)));
            }
        }

        self.observers.notify();
    }

    /// The probe, as it was last taken.
    pub fn permissions(&self) -> Option<ComputerUsePermissions> {
        self.permissions.borrow().clone()
    }

    /// The daemon's own sentence, where the probe was refused.
    pub fn refusal(&self) -> Option<Sentence> {
        self.refusal.borrow().clone()
    }

    /// When the probe was taken, as the daemon wrote it.
    pub fn probed_at(&self) -> Option<String> {
        self.permissions
            .borrow()
            .as_ref()
            .and_then(|permissions| permissions.probed_at.clone())
    }

    /// Every row, with what is known about each.
    pub fn rows(&self) -> Vec<LedgerRow> {
        RIGHTS
            .iter()
            .map(|right| LedgerRow {
                right: *right,
                standing: self.standing(*right),
            })
            .collect()
    }

    /// One row, by right.
    pub fn row(&self, right: Right) -> LedgerRow {
        LedgerRow {
            right,
            standing: self.standing(right),
        }
    }

    /// Whether one of the helper's two rights is held, as the probe answered.
    pub fn holds(&self, right: Right) -> Option<bool> {
        let permissions = self.permissions.borrow();
        let permissions = permissions.as_ref()?;
        match right {
            Right::ScreenCapture => Some(permissions.screen_capture),
            Right::InputSynthesis => Some(permissions.input_control),
            _ => None,
        }
    }

    fn standing(&self, right: Right) -> Option<Key> {
        match right {
            Right::ScreenCapture | Right::InputSynthesis => {
                Some(super::computer::verdict(self.holds(right)))
            }
            Right::BackgroundService => Some(if self.settings.state().background_enabled() {
                Key::StateOn
            } else {
                Key::StateOff
            }),
            Right::Autostart => Some(if self.settings.state().open_at_login {
                Key::StateOn
            } else {
                Key::StateOff
            }),
            // Nothing is kept, so there is nothing to report. The principal and
            // the artifact say the whole of it.
            Right::Microphone | Right::Privileged | Right::BrowserAutomation => None,
        }
    }
}
