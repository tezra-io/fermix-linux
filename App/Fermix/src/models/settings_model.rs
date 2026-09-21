//! The one settings model.
//!
//! Built once, in the application composition, and handed to every surface.
//! It holds the setup snapshot, the per-section descriptor cache, the restart
//! state, the configuration state, the service's own view of the unit, the
//! drafts a person has not committed and the values waiting for the daemon to
//! confirm them. Views read it and never copy it.
//!
//! Three rules run through every method here:
//!
//! 1. **Every answer is checked against the epoch it was issued under.** A
//!    result from a connection that has since gone cannot change what is on
//!    screen.
//! 2. **Every write is optimistic in the control and confirmed by the daemon.**
//!    The typed value is held until the accepted re-read lands; a refusal puts
//!    the daemon's value back and keeps the daemon's own sentence under the row.
//! 3. **A refresh never disturbs an edit.** A draft outlives navigation and a
//!    poll, and only the person who typed it can end it.

use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gtk4::gio;

use crate::management::types::{
    ConfigState, DetectResult, DetectTarget, HelloResult, JobView, OverviewResult, RestartState,
    SecretClearParams, SecretMigrateParams, SecretMigrateResult, SecretResult, SecretSetParams,
    SettingValue, SettingsApplyParams, SettingsApplyResult, SettingsGetParams, SettingsGetResult,
    SettingsPane, SettingsReloadResult, SettingsRow, SettingsSection, SettingsSectionsResult,
    SetupDetectParams, SetupDetectResult, SetupStateResult,
};
use crate::management::vocabulary::{ConfigCondition, Refusal};
use crate::management::{ManagementError, TransportError};
use crate::service::runner::{ServiceError, ServiceRunner};

/// How many times the re-read after a write asks the gate for a slot.
///
/// Twenty tries a quarter-second apart is five seconds: far longer than four
/// reads take to finish, and still bounded.
const SETUP_REREAD_ATTEMPTS: u32 = 20;

/// How long that re-read waits between asking.
const SETUP_REREAD_PAUSE: std::time::Duration = std::time::Duration::from_millis(250);
use crate::service::types::{Restart, ServiceStatus};
use crate::session::autostart;

use super::api::{
    accept, ask, Gate, ManagementApi, READ_DEADLINE, UNLOCK_DEADLINE, WRITE_DEADLINE,
};
use super::secret_store::StoreKind;

/// One row, by the section it belongs to and the key it writes.
pub type RowId = (String, String);

/// What changed. Views subscribe to the model and redraw the part they own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// The daemon's own snapshot, or whether it is answering at all.
    Daemon,
    /// Readiness, the restart state, the configuration state.
    Setup,
    /// The section inventory.
    Sections,
    /// One section's rows.
    Section(String),
    /// One row's draft, optimistic value, refusal or busy state.
    Row(RowId),
    /// The unit's own state, and the two background controls.
    Service,
    /// The jobs this daemon retains.
    Jobs,
    /// What this machine was found to already have.
    Detections,
    /// The selected settings pane.
    Pane,
}

/// A refusal a person is shown: the daemon's or the command line's own
/// sentence, never one composed here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sentence {
    /// The code, where the surface routes on one.
    pub code: Option<String>,
    /// The words.
    pub text: String,
    /// `details.reason`, where the code publishes one.
    ///
    /// A code alone cannot separate a locked keyring from an absent one: both
    /// arrive under the same store refusal, and only the reason says which. It
    /// was dropped here before any surface could read it, so the dialog that
    /// had to tell them apart never had the field that does.
    pub reason: Option<String>,
}

impl Sentence {
    /// One management refusal, as the sentence a person is shown.
    pub fn of(error: &ManagementError) -> Self {
        match error {
            ManagementError::Wire(wire) => Sentence {
                code: Some(wire.code.clone()),
                text: wire.rendered().to_string(),
                reason: wire.detail("reason").map(str::to_owned),
            },
            other => Sentence {
                code: None,
                text: other.to_string(),
                reason: None,
            },
        }
    }

    /// One command line refusal, as the sentence a person is shown.
    ///
    /// The typed command line publishes no reason, so this carries none rather
    /// than parsing one out of the words.
    pub fn of_service(error: &ServiceError) -> Self {
        match error {
            ServiceError::Refused { code, sentence } => Sentence {
                code: Some(code.clone()),
                text: sentence.clone(),
                reason: None,
            },
            other => Sentence {
                code: None,
                text: other.to_string(),
                reason: None,
            },
        }
    }
}

/// Everything the model holds.
#[derive(Default)]
pub struct State {
    /// The daemon's identity, from `hello`.
    pub hello: Option<HelloResult>,
    /// The last `overview.get`.
    pub overview: Option<OverviewResult>,
    /// The last `setup.state.get`.
    pub setup: Option<SetupStateResult>,
    /// The published section inventory.
    pub sections: Vec<SettingsSection>,
    /// One section's rows, as the daemon last served them.
    pub descriptors: BTreeMap<String, SettingsGetResult>,
    /// The restart state the daemon publishes. Never a list composed here.
    pub restart: RestartState,
    /// Whether writes land, refuse, or have nowhere to go.
    pub config: ConfigCondition,
    /// The parser's own sentence, while the settings file cannot be read.
    pub unreadable: Option<Sentence>,
    /// The command line's view of the unit.
    pub service: Option<ServiceStatus>,
    /// The command line's own refusal, where it refused.
    pub service_refusal: Option<Sentence>,
    /// Whether the daemon answered at all. `None` until it has been asked.
    pub reachable: Option<bool>,
    /// The jobs the daemon retains.
    pub jobs: Vec<JobView>,
    /// What this machine was found to already have, by target. One owner, so
    /// the provider verbs and the meeting sign-in state cannot disagree about
    /// the same probe.
    pub detections: Vec<DetectResult>,
    /// The pane Settings is showing.
    pub pane: SettingsPane,
    /// What a person typed and has not committed. It survives navigation.
    pub drafts: BTreeMap<RowId, SettingValue>,
    /// What has been sent and not yet confirmed by an accepted re-read.
    pub in_flight: BTreeMap<RowId, SettingValue>,
    /// The daemon's own sentence under one row.
    pub refusals: BTreeMap<RowId, Sentence>,
    /// The rows with a write outstanding.
    pub busy: BTreeSet<RowId>,
    /// Whether Fermix opens at login.
    pub open_at_login: bool,
    /// The background switch's optimistic position, while a mutation runs.
    pub background_pending: Option<bool>,
}

impl State {
    /// Whether the unit is enabled, as the switch shows it: the optimistic
    /// position while a mutation runs, and the command line's answer otherwise.
    pub fn background_enabled(&self) -> bool {
        self.background_pending
            .unwrap_or_else(|| self.service.as_ref().is_some_and(|status| status.enabled))
    }

    /// One section's rows, as they should be drawn: the daemon's rows with the
    /// draft and the in-flight value laid over them.
    pub fn rows(&self, section: &str) -> Vec<SettingsRow> {
        let Some(descriptor) = self.descriptors.get(section) else {
            return Vec::new();
        };

        descriptor
            .rows
            .iter()
            .map(|row| {
                let mut row = row.clone();
                if let Some(value) = self.value_of(&(section.to_string(), row.key.clone())) {
                    row.value = value;
                }
                row
            })
            .collect()
    }

    /// The value one row should show: a draft first, then an unconfirmed write,
    /// then the daemon's own.
    pub fn value_of(&self, id: &RowId) -> Option<SettingValue> {
        self.drafts
            .get(id)
            .or_else(|| self.in_flight.get(id))
            .cloned()
    }

    /// The daemon's own value for one row, which is what Escape restores.
    pub fn daemon_value(&self, section: &str, key: &str) -> Option<SettingValue> {
        self.descriptors
            .get(section)?
            .rows
            .iter()
            .find(|row| row.key == key)
            .map(|row| row.value.clone())
    }

    /// The sentence under one row, where there is one.
    pub fn refusal(&self, id: &RowId) -> Option<&Sentence> {
        self.refusals.get(id)
    }

    /// Whether one row has a write outstanding.
    pub fn is_busy(&self, id: &RowId) -> bool {
        self.busy.contains(id)
    }

    /// One detection, where it has been taken.
    ///
    /// Absent is not false: a target nobody has asked about and a target that
    /// answered no are different answers, and the surfaces that read this one
    /// render them differently.
    pub fn detection(&self, target: DetectTarget) -> Option<&DetectResult> {
        self.detections
            .iter()
            .find(|result| result.target == target)
    }

    /// The sections that render under one pane, in published order.
    pub fn sections_of(&self, pane: SettingsPane) -> Vec<&SettingsSection> {
        self.sections
            .iter()
            .filter(|section| section.pane == pane)
            .collect()
    }
}

type Observer = Rc<dyn Fn(&Change)>;

/// The one model.
pub struct SettingsModel {
    api: Rc<dyn ManagementApi>,
    service: Rc<ServiceRunner>,
    gate: Gate,
    state: RefCell<State>,
    observers: RefCell<Vec<Observer>>,
}

impl SettingsModel {
    /// The one instance. Constructed in the application composition and shared;
    /// `tests/structure.rs` counts the constructions.
    pub fn new(api: Rc<dyn ManagementApi>, service: Rc<ServiceRunner>) -> Rc<Self> {
        Rc::new(Self {
            api,
            service,
            gate: Gate::default(),
            state: RefCell::new(State {
                pane: super::pane::first(),
                open_at_login: autostart::is_enabled(),
                ..State::default()
            }),
            observers: RefCell::new(Vec::new()),
        })
    }

    /// What the model holds. A view reads through this and never keeps a copy;
    /// the borrow is released before anything awaits.
    pub fn state(&self) -> Ref<'_, State> {
        self.state.borrow()
    }

    /// The daemon this model speaks to. The feature models call through it.
    pub fn api(&self) -> Rc<dyn ManagementApi> {
        Rc::clone(&self.api)
    }

    /// The command line this model runs.
    pub fn service(&self) -> Rc<ServiceRunner> {
        Rc::clone(&self.service)
    }

    /// Subscribe to what changes. The callback runs on the main context, and it
    /// must not borrow the model's state across an await.
    pub fn observe(&self, observer: impl Fn(&Change) + 'static) {
        self.observers.borrow_mut().push(Rc::new(observer));
    }

    /// Tell every observer.
    ///
    /// The list is copied and every borrow released before an observer runs, so
    /// an observer that writes back into the model cannot meet a borrow this
    /// loop is holding.
    fn notify(&self, change: Change) {
        let observers: Vec<Observer> = self.observers.borrow().clone();
        for observer in observers {
            observer(&change);
        }
    }

    // ---- Reads ----------------------------------------------------------

    /// Everything a window needs before it draws: the identity, the overview,
    /// the setup state, the inventory and the unit.
    pub async fn refresh_all(&self) {
        self.refresh_service().await;
        self.refresh_overview().await;
        self.refresh_setup().await;
        self.refresh_sections().await;
    }

    /// `overview.get`. Home's five-second poll, and the one read that says
    /// whether the daemon is answering at all.
    pub async fn refresh_overview(&self) {
        let Some(_permit) = self.gate.read() else {
            return;
        };

        let issued = ask::<_, OverviewResult>(
            self.api.as_ref(),
            "overview.get",
            &serde_json::json!({}),
            READ_DEADLINE,
        )
        .await;

        let Some(result) = accept(self.api.as_ref(), issued) else {
            return;
        };

        match result {
            Ok(overview) => {
                let mut state = self.state.borrow_mut();
                state.reachable = Some(true);
                state.hello = self.api.last_hello();
                state.overview = Some(overview);
            }
            Err(error) if is_stopped(&error) => {
                let mut state = self.state.borrow_mut();
                state.reachable = Some(false);
                state.overview = None;
            }
            // The daemon refused, or the exchange failed on its way. Neither is
            // a stopped daemon, and nothing is claimed about one.
            Err(_) => {}
        }

        self.notify(Change::Daemon);
    }

    /// `setup.state.get`: readiness, the restart state and the configuration
    /// state, all in one read.
    pub async fn refresh_setup(&self) {
        let _ = self.read_setup().await;
    }

    /// The same read, with what it answered.
    ///
    /// A surface that only redraws has nothing to do with a refusal and uses
    /// [`SettingsModel::refresh_setup`]; the Setup assistant is the one caller
    /// that has to say which step could not be taken, so it asks for the
    /// answer. One read, two entry points, and no second code path.
    pub async fn read_setup(&self) -> Result<(), ManagementError> {
        let Some(permit) = self.gate.read() else {
            return Ok(());
        };
        self.read_setup_holding(permit).await
    }

    /// The re-read a write owes the row, which the gate must not be able to
    /// lose.
    ///
    /// Reads are capped at four in flight and [`SettingsModel::read_setup`]
    /// answers a refused permit with `Ok(())`, which everywhere else is
    /// harmless: a surface that only redraws gets the next snapshot. After a
    /// write that moved where secrets live it is not harmless at all, because
    /// nothing else on that path reads again and the row would go on naming
    /// the store the owner just moved away from. So this waits for a slot
    /// rather than giving up on one, bounded, and says so at the cap instead
    /// of returning as though it had read.
    async fn reread_setup_after_write(&self) {
        for _ in 0..SETUP_REREAD_ATTEMPTS {
            if let Some(permit) = self.gate.read() {
                let _ = self.read_setup_holding(permit).await;
                return;
            }
            gtk4::glib::timeout_future(SETUP_REREAD_PAUSE).await;
        }

        // The cap is reached only if four reads stayed in flight for the whole
        // window, which means the application is wedged rather than busy. The
        // row is stale and this is the only place that knows it.
        eprintln!(
            "the setup re-read after a write never got a slot: the store row may name the wrong store"
        );
    }

    async fn read_setup_holding(
        &self,
        permit: crate::models::api::Permit,
    ) -> Result<(), ManagementError> {
        let _permit = permit;

        let issued = ask::<_, SetupStateResult>(
            self.api.as_ref(),
            "setup.state.get",
            &serde_json::json!({}),
            READ_DEADLINE,
        )
        .await;

        let setup = match accept(self.api.as_ref(), issued) {
            // The connection moved while the read was out. Nothing is claimed
            // about what it would have said.
            None => return Ok(()),
            Some(Ok(setup)) => setup,
            Some(Err(error)) => return Err(error),
        };

        {
            let mut state = self.state.borrow_mut();
            state.restart = setup.restart.clone();
            state.config = ConfigCondition::of(setup.coexistence.config_state);
            if setup.coexistence.config_state != ConfigState::ConfigUnreadable {
                state.unreadable = None;
            }
            state.setup = Some(setup);
        }

        self.notify(Change::Setup);
        Ok(())
    }

    /// `settings.sections`: the one inventory the pane list and the search
    /// index are built from.
    pub async fn refresh_sections(&self) {
        let Some(_permit) = self.gate.read() else {
            return;
        };

        let issued = ask::<_, SettingsSectionsResult>(
            self.api.as_ref(),
            "settings.sections",
            &serde_json::json!({}),
            READ_DEADLINE,
        )
        .await;

        let Some(Ok(result)) = accept(self.api.as_ref(), issued) else {
            return;
        };

        self.state.borrow_mut().sections = result.sections;
        self.notify(Change::Sections);
    }

    /// `setup.detect` for the targets one surface needs.
    ///
    /// The answers land on the one model rather than on the surface that asked,
    /// because a detection moves a provider's verb and the meeting sign-in
    /// state, and an install that finishes while its pane is hidden still has
    /// to move them.
    pub async fn refresh_detections(&self, targets: &[DetectTarget]) {
        if targets.is_empty() {
            return;
        }

        let Some(_permit) = self.gate.read() else {
            return;
        };

        let params = SetupDetectParams {
            targets: targets.to_vec(),
        };
        let issued =
            ask::<_, SetupDetectResult>(self.api.as_ref(), "setup.detect", &params, READ_DEADLINE)
                .await;

        let Some(result) = accept(self.api.as_ref(), issued) else {
            return;
        };

        match result {
            Ok(detected) => {
                let mut state = self.state.borrow_mut();
                for answer in detected.results {
                    state.detections.retain(|held| held.target != answer.target);
                    state.detections.push(answer);
                }
            }
            // A refused probe is not a negative answer, and nothing is claimed
            // from one: the target keeps whatever it last said, or nothing.
            Err(_) => {
                let mut state = self.state.borrow_mut();
                for target in targets {
                    state.detections.retain(|held| held.target != *target);
                }
            }
        }

        self.notify(Change::Detections);
    }

    /// `settings.get` for one section.
    ///
    /// A section already read is read again: the daemon is the value, and a
    /// cache that never refreshes is a second copy of the configuration.
    pub async fn refresh_section(&self, section: &str) {
        let Some(_permit) = self.gate.read() else {
            return;
        };

        let params = SettingsGetParams {
            section: section.to_string(),
        };
        let issued =
            ask::<_, SettingsGetResult>(self.api.as_ref(), "settings.get", &params, READ_DEADLINE)
                .await;

        let Some(result) = accept(self.api.as_ref(), issued) else {
            return;
        };

        match result {
            Ok(descriptor) => {
                let mut state = self.state.borrow_mut();
                // The accepted re-read is what ends an optimistic value: the
                // rows it carries are the daemon's own answer for them.
                state
                    .in_flight
                    .retain(|(owner, _), _| owner.as_str() != section);
                state.descriptors.insert(section.to_string(), descriptor);
            }
            Err(error) => {
                self.record_configuration_refusal(&error);
                return;
            }
        }

        self.notify(Change::Section(section.to_string()));
    }

    /// Every section of one pane, read.
    pub async fn refresh_pane(&self, pane: SettingsPane) {
        let sections: Vec<String> = self
            .state
            .borrow()
            .sections_of(pane)
            .iter()
            .map(|section| section.id.clone())
            .collect();

        for section in sections {
            self.refresh_section(&section).await;
        }
    }

    /// `fermix service status --json`.
    pub async fn refresh_service(&self) {
        let Some(_permit) = self.gate.read() else {
            return;
        };

        let cancellable = gio::Cancellable::new();
        match self.service.status(&cancellable).await {
            Ok(status) => {
                // The command line is where the socket's path comes from: a
                // packaged run does not know the home it is bound to until this
                // read lands, and the socket is the one inside that home
                // (M38 section 4.7).
                if let Some(socket) = bound_socket(&status) {
                    self.api.rebind(&socket);
                }

                let mut state = self.state.borrow_mut();
                state.service = Some(status);
                state.service_refusal = None;
            }
            Err(error) => {
                let mut state = self.state.borrow_mut();
                state.service_refusal = Some(Sentence::of_service(&error));
            }
        }

        self.notify(Change::Service);
    }

    // ---- Writes ---------------------------------------------------------

    /// What a person typed and has not committed.
    ///
    /// Kept in the model rather than in the widget, so it survives leaving the
    /// pane, a poll landing underneath it, and the row being redrawn.
    pub fn set_draft(&self, section: &str, key: &str, value: SettingValue) {
        let id = (section.to_string(), key.to_string());
        self.state.borrow_mut().drafts.insert(id.clone(), value);
        self.notify(Change::Row(id));
    }

    /// Forget a draft without writing it, which is what Escape does.
    pub fn discard_draft(&self, section: &str, key: &str) {
        let id = (section.to_string(), key.to_string());
        let had = self.state.borrow_mut().drafts.remove(&id).is_some();
        if had {
            self.notify(Change::Row(id));
        }
    }

    /// The draft for one row, where there is one.
    pub fn draft(&self, section: &str, key: &str) -> Option<SettingValue> {
        self.state
            .borrow()
            .drafts
            .get(&(section.to_string(), key.to_string()))
            .cloned()
    }

    /// Write one row and confirm it.
    ///
    /// The control keeps the typed value while the write is out; an accepted
    /// re-read is what releases it, and a refusal puts the daemon's value back
    /// with the daemon's own sentence under the row.
    pub async fn apply(&self, section: &str, key: &str, value: SettingValue) {
        let id = (section.to_string(), key.to_string());

        let Some(_permit) = self.gate.write() else {
            self.record_row_refusal(&id, busy_sentence());
            return;
        };

        {
            let mut state = self.state.borrow_mut();
            state.drafts.remove(&id);
            state.refusals.remove(&id);
            state.in_flight.insert(id.clone(), value.clone());
            state.busy.insert(id.clone());
        }
        self.notify(Change::Row(id.clone()));

        let accepted = self
            .send_apply(section, BTreeMap::from([(key.to_string(), value)]))
            .await;
        self.state.borrow_mut().busy.remove(&id);

        match accepted {
            None => {
                // The connection moved while the write was out. Nothing is
                // claimed about whether it landed; the next read says.
                self.state.borrow_mut().in_flight.remove(&id);
                self.notify(Change::Row(id));
            }
            Some(Ok(result)) => {
                self.state.borrow_mut().restart = result.restart.clone();
                self.notify(Change::Setup);
                self.refresh_section(section).await;
                self.notify(Change::Row(id));
            }
            Some(Err(error)) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.in_flight.remove(&id);
                    state.refusals.insert(id.clone(), Sentence::of(&error));
                }
                self.record_configuration_refusal(&error);
                self.notify(Change::Row(id));
            }
        }
    }

    /// Write a whole form at once and say what the daemon answered.
    ///
    /// One section, several keys, one write: the Setup assistant's About you
    /// screen collects four values whose absence is what keeps the assistant
    /// open, and the daemon takes them whole or refuses them whole. A refusal
    /// is the daemon's own sentence, handed to the screen that asked.
    pub async fn apply_values(
        &self,
        section: &str,
        values: BTreeMap<String, SettingValue>,
    ) -> Result<SettingsApplyResult, Sentence> {
        let Some(_permit) = self.gate.write() else {
            return Err(busy_sentence());
        };

        match self.send_apply(section, values).await {
            // The connection moved while the write was out. Nothing is claimed
            // about whether it landed; the next read says.
            None => Err(busy_sentence()),
            Some(Ok(result)) => {
                self.state.borrow_mut().restart = result.restart.clone();
                self.notify(Change::Setup);
                self.refresh_section(section).await;
                Ok(result)
            }
            Some(Err(error)) => {
                let sentence = Sentence::of(&error);
                self.record_configuration_refusal(&error);
                Err(sentence)
            }
        }
    }

    /// The one `settings.apply` call site. Both the row write and the form
    /// write go through it, so there is one place the wire is spoken to and one
    /// shape on it.
    async fn send_apply(
        &self,
        section: &str,
        values: BTreeMap<String, SettingValue>,
    ) -> Option<Result<SettingsApplyResult, ManagementError>> {
        let params = SettingsApplyParams {
            section: section.to_string(),
            values,
        };
        let issued = ask::<_, SettingsApplyResult>(
            self.api.as_ref(),
            "settings.apply",
            &params,
            WRITE_DEADLINE,
        )
        .await;

        accept(self.api.as_ref(), issued)
    }

    /// `settings.reload`, the one action behind the external-change banner.
    pub async fn reload(&self) {
        let Some(_permit) = self.gate.write() else {
            return;
        };

        let issued = ask::<_, SettingsReloadResult>(
            self.api.as_ref(),
            "settings.reload",
            &serde_json::json!({}),
            WRITE_DEADLINE,
        )
        .await;

        let Some(result) = accept(self.api.as_ref(), issued) else {
            return;
        };

        match result {
            Ok(reloaded) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.restart = reloaded.restart.clone();
                    state.config = ConfigCondition::of(reloaded.config_state);
                    state.refusals.clear();
                    if state.config != ConfigCondition::Unreadable {
                        state.unreadable = None;
                    }
                    state.descriptors.clear();
                }
                self.notify(Change::Setup);
                self.refresh_setup().await;
            }
            Err(error) => {
                self.record_configuration_refusal(&error);
            }
        }
    }

    /// `secret.set`. The value arrives from the one dialog that owns a secret
    /// entry; a blank one is refused there and never reaches the wire.
    pub async fn set_secret(
        &self,
        section: &str,
        key: &str,
        value: String,
    ) -> Result<(), Sentence> {
        self.send_secret(section, key, value, None, None, WRITE_DEADLINE)
            .await
    }

    /// `secret.set` into the private file store, because the owner chose it.
    ///
    /// The choice travels in this one request rather than in a flag set here:
    /// the engine never falls back on its own, so a value in the file store
    /// can only have come from a button that said what it would do.
    pub async fn store_secret_on_this_computer(
        &self,
        section: &str,
        key: &str,
        value: String,
    ) -> Result<(), Sentence> {
        self.send_secret(section, key, value, Some("file"), None, WRITE_DEADLINE)
            .await
    }

    /// `secret.set` again, waiting for the owner to finish unlocking.
    ///
    /// The only call that waits on a person, so the only one given the long
    /// deadline.
    pub async fn retry_secret_after_unlock(
        &self,
        section: &str,
        key: &str,
        value: String,
    ) -> Result<(), Sentence> {
        self.send_secret(section, key, value, None, Some(true), UNLOCK_DEADLINE)
            .await
    }

    /// One `secret.set`, however the caller chose to store it.
    ///
    /// Every store marks the row busy and records the answer the same way, so
    /// a refusal from the file store reads like any other.
    async fn send_secret(
        &self,
        section: &str,
        key: &str,
        value: String,
        store: Option<&'static str>,
        unlock: Option<bool>,
        deadline: std::time::Duration,
    ) -> Result<(), Sentence> {
        let id = (section.to_string(), key.to_string());
        if value.is_empty() {
            return Ok(());
        }

        let Some(_permit) = self.gate.write() else {
            let sentence = busy_sentence();
            self.record_row_refusal(&id, sentence.clone());
            return Err(sentence);
        };

        self.state.borrow_mut().busy.insert(id.clone());
        self.notify(Change::Row(id.clone()));

        let params = SecretSetParams {
            id: key.to_string(),
            value,
            store,
            unlock,
        };
        let issued =
            ask::<_, SecretResult>(self.api.as_ref(), "secret.set", &params, deadline).await;

        self.finish_secret(section, &id, accept(self.api.as_ref(), issued))
            .await
    }

    /// Where secrets live, as the last setup snapshot published it.
    ///
    /// `None` when the engine published nothing, which is not `none`: an
    /// engine that predates the field has not said there is no store, it has
    /// said nothing at all, and a row that claimed otherwise would be making
    /// the same kind of untrue statement this work exists to remove.
    pub fn secret_store_kind(&self) -> Option<StoreKind> {
        let state = self.state.borrow();
        let secrets = state.setup.as_ref()?.secrets.as_ref()?;
        StoreKind::of(&secrets.store)
    }

    /// `secret.migrate_to_keyring`: the way back from the file store.
    ///
    /// Takes no secret, because this application cannot read one back out of
    /// the file store and the owner should not have to retype what the engine
    /// already holds. A refusal leaves every value where it was.
    pub async fn migrate_to_keyring(&self, unlock: bool) -> Result<Vec<String>, Sentence> {
        let Some(_permit) = self.gate.write() else {
            return Err(busy_sentence());
        };

        let deadline = if unlock {
            UNLOCK_DEADLINE
        } else {
            WRITE_DEADLINE
        };
        let params = SecretMigrateParams { unlock };
        let issued = ask::<_, SecretMigrateResult>(
            self.api.as_ref(),
            "secret.migrate_to_keyring",
            &params,
            deadline,
        )
        .await;

        let moved = match accept(self.api.as_ref(), issued) {
            Some(Ok(result)) => result.moved,
            Some(Err(error)) => return Err(Sentence::of(&error)),
            None => return Ok(Vec::new()),
        };

        // The permit is released before the read, because the gate counts a
        // write against the same ceiling and this read is the whole point of
        // the write: the values are in the keyring now, so the row has to stop
        // saying otherwise and stop offering a way back already taken.
        drop(_permit);
        self.reread_setup_after_write().await;
        Ok(moved)
    }

    /// `secret.clear`.
    pub async fn clear_secret(&self, section: &str, key: &str) -> Result<(), Sentence> {
        let id = (section.to_string(), key.to_string());

        let Some(_permit) = self.gate.write() else {
            let sentence = busy_sentence();
            self.record_row_refusal(&id, sentence.clone());
            return Err(sentence);
        };

        self.state.borrow_mut().busy.insert(id.clone());
        self.notify(Change::Row(id.clone()));

        let params = SecretClearParams {
            id: key.to_string(),
        };
        let issued =
            ask::<_, SecretResult>(self.api.as_ref(), "secret.clear", &params, WRITE_DEADLINE)
                .await;

        self.finish_secret(section, &id, accept(self.api.as_ref(), issued))
            .await
    }

    /// The answer to one secret write, and the sentence where it was refused.
    async fn finish_secret(
        &self,
        section: &str,
        id: &RowId,
        result: Option<Result<SecretResult, ManagementError>>,
    ) -> Result<(), Sentence> {
        self.state.borrow_mut().busy.remove(id);

        match result {
            // The connection moved while the write was out. Nothing is claimed
            // about whether it landed; the next read says.
            None => {
                self.notify(Change::Row(id.clone()));
                Ok(())
            }
            Some(Ok(answer)) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.restart = answer.restart.clone();
                    state.refusals.remove(id);
                }
                self.notify(Change::Setup);
                // Where secrets live can have moved, and only the engine knows
                // whether it did: consenting to the file store is a write whose
                // visible consequence is a different answer on the next
                // snapshot. A notify alone redraws the row from the cached one,
                // so the row would state the old store immediately after the
                // owner chose a new one.
                self.reread_setup_after_write().await;
                self.refresh_section(section).await;
                Ok(())
            }
            Some(Err(error)) => {
                let sentence = Sentence::of(&error);
                self.record_row_refusal(id, sentence.clone());
                self.record_configuration_refusal(&error);
                Err(sentence)
            }
        }
    }

    // ---- The two background controls ------------------------------------

    /// Turn the background service on or off through the command line.
    ///
    /// The switch holds the position it was moved to until the answer lands; a
    /// refusal puts it back and hands the command line's own sentence to the
    /// caller, which is what shows it under the row.
    pub async fn set_background_service(&self, enabled: bool) -> Result<(), Sentence> {
        let Some(_permit) = self.gate.lifecycle() else {
            return Err(busy_sentence());
        };

        self.state.borrow_mut().background_pending = Some(enabled);
        self.notify(Change::Service);

        let cancellable = gio::Cancellable::new();
        let outcome = if enabled {
            self.service
                .install(None, None, &cancellable)
                .await
                .map(|_| ())
        } else {
            self.service.uninstall(&cancellable).await.map(|_| ())
        };

        self.state.borrow_mut().background_pending = None;

        let answer = match outcome {
            Ok(()) => Ok(()),
            Err(error) => Err(Sentence::of_service(&error)),
        };

        self.refresh_service().await;
        answer
    }

    /// Write, or hide, the autostart entry.
    pub fn set_open_at_login(&self, enabled: bool) -> Result<(), String> {
        let outcome = autostart::set_enabled(enabled).map_err(|error| error.to_string());

        self.state.borrow_mut().open_at_login = autostart::is_enabled();
        self.notify(Change::Service);

        outcome
    }

    /// `fermix restart --json [--when-idle]`.
    pub async fn restart(&self, when_idle: bool) -> Result<Restart, Sentence> {
        let Some(_permit) = self.gate.lifecycle() else {
            return Err(busy_sentence());
        };

        let cancellable = gio::Cancellable::new();
        let outcome = self.service.restart(when_idle, &cancellable).await;

        match outcome {
            Ok(restarted) => {
                // The daemon this client was speaking to has gone. A new epoch
                // means nothing issued under the old connection can land on the
                // model built from the new one.
                self.api.reset();
                self.refresh_service().await;
                self.refresh_overview().await;
                self.refresh_setup().await;
                Ok(restarted)
            }
            Err(error) => Err(Sentence::of_service(&error)),
        }
    }

    // ---- The selected pane ----------------------------------------------

    /// Show one pane. The selection is part of the model, so leaving Settings
    /// and coming back lands where it was left.
    pub fn select_pane(&self, pane: SettingsPane) {
        if self.state.borrow().pane == pane {
            return;
        }
        self.state.borrow_mut().pane = pane;
        self.notify(Change::Pane);
    }

    /// The pane Settings is showing.
    pub fn pane(&self) -> SettingsPane {
        self.state.borrow().pane
    }

    // ---- Refusals -------------------------------------------------------

    fn record_row_refusal(&self, id: &RowId, sentence: Sentence) {
        self.state
            .borrow_mut()
            .refusals
            .insert(id.clone(), sentence);
        self.notify(Change::Row(id.clone()));
    }

    /// A refusal that is about the settings file rather than about one row.
    /// Both kinds carry the daemon's own sentence; this is the half that moves
    /// the whole surface.
    fn record_configuration_refusal(&self, error: &ManagementError) {
        let ManagementError::Wire(wire) = error else {
            return;
        };

        match Refusal::of(wire) {
            Refusal::ExternalChange => {
                self.state.borrow_mut().config = ConfigCondition::ExternalChange;
                self.notify(Change::Setup);
            }
            Refusal::ConfigUnreadable => {
                {
                    let mut state = self.state.borrow_mut();
                    state.config = ConfigCondition::Unreadable;
                    state.unreadable = Some(Sentence::of(error));
                }
                self.notify(Change::Setup);
            }
            _ => {}
        }
    }
}

/// The one refusal the application itself answers with, and the only one: the
/// bounds of M38 section 12.2 are the application's own, so the sentence for
/// hitting them is the application's own too.
fn busy_sentence() -> Sentence {
    Sentence {
        code: Some("busy".to_string()),
        text: crate::copy::text(crate::copy::Key::BusyQueueFull),
        reason: None,
    }
}

/// The control socket of the home this account is bound to.
///
/// One resolver: the home the command line reports, and the socket name M38
/// section 4.7 fixes inside it. Nothing here searches for a home or guesses a
/// default.
pub fn bound_socket(status: &ServiceStatus) -> Option<std::path::PathBuf> {
    let home = status.bound_home()?;
    Some(std::path::PathBuf::from(home).join(crate::paths::SOCKET_NAME))
}

/// Whether an error means the daemon is not there, as opposed to refusing.
///
/// A missing socket file and a socket nothing accepts on are the same fact to a
/// person. A deadline is not: the peer answered the connection and then said
/// nothing, which is a daemon in trouble rather than a daemon that is gone.
fn is_stopped(error: &ManagementError) -> bool {
    matches!(
        error,
        ManagementError::Transport(TransportError::NotRunning)
    )
}
