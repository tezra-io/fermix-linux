//! Providers, as data.
//!
//! One row per provider the daemon publishes, each carrying where it stands and
//! the one thing to do about it. Every fact is the daemon's: the label, the
//! account, the token state, the models and the refusals. What this file owns
//! is the rendering of two closed vocabularies the wire publishes as booleans
//! and atoms — the six status words and the one verb a row leads with — and
//! which method each verb runs.
//!
//! The verb is the part a detection moves and nothing else does. A machine with
//! the Claude Code command line signed in shows "Use the Claude Code sign-in"
//! where one without it shows "Add setup token"; they are the same row, and
//! `auth_modes` alone is never the signal: Anthropic publishes `oauth` there
//! and `auth.start` refuses it, because the daemon has no browser flow for it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::copy::Key;
use crate::management::types::{
    AuthImportParams, AuthLogoutResult, AuthStartResult, DetectTarget, JobStatus, JobView,
    ProviderParams, ProviderParams as ProbeParams, ProvidersModelsListParams,
    ProvidersModelsListResult, ProvidersSetPrimaryResult, SettingsRow, SettingsRowKind,
    SetupProviderRow,
};
use crate::management::vocabulary::{AuthMode, TokenState};

use super::api::{accept, ask, READ_DEADLINE, WRITE_DEADLINE};
use super::jobs::JobRunner;
use super::settings_model::{Sentence, SettingsModel};
use super::{spawn, Observers};

/// The one verb a provider row leads with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderVerb {
    /// A browser hop the daemon will actually start.
    SignIn,
    /// Adopt the sign-in the Claude Code command line already holds.
    ImportClaudeCode,
    /// Adopt the sign-in the Codex command line already holds.
    ImportCodexCli,
    /// Anthropic's other door: a long-lived subscription credential.
    AddSetupToken,
    /// A key the operator types.
    AddKey,
}

impl ProviderVerb {
    /// The word on the button.
    pub fn key(self) -> Key {
        match self {
            ProviderVerb::SignIn => Key::ProviderSignIn,
            ProviderVerb::ImportClaudeCode => Key::ProviderImportClaudeCode,
            ProviderVerb::ImportCodexCli => Key::ProviderImportCodexCli,
            ProviderVerb::AddSetupToken => Key::ProviderAddSetupToken,
            ProviderVerb::AddKey => Key::ProviderAddKey,
        }
    }

    /// Whether this verb writes a secret, which it can only do into a slot the
    /// daemon named.
    pub fn writes_secret(self) -> bool {
        matches!(self, ProviderVerb::AddKey | ProviderVerb::AddSetupToken)
    }
}

/// Where a provider stands, in the six words this door owns.
///
/// The wire carries booleans and a token atom rather than a word, and a word
/// has to exist somewhere: this is the same rule the Doctor statuses and the
/// job phases follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStanding {
    Primary,
    PrimaryNotConnected,
    Connected,
    KeyStored,
    Reconnect,
    NotConnected,
}

impl ProviderStanding {
    /// The word for this standing.
    pub fn key(self) -> Key {
        match self {
            ProviderStanding::Primary => Key::ProviderStatusPrimary,
            ProviderStanding::PrimaryNotConnected => Key::ProviderStatusPrimaryNotConnected,
            ProviderStanding::Connected => Key::ProviderStatusConnected,
            ProviderStanding::KeyStored => Key::ProviderStatusKeyStored,
            ProviderStanding::Reconnect => Key::ProviderStatusReconnect,
            ProviderStanding::NotConnected => Key::ProviderStatusNotConnected,
        }
    }
}

/// One provider row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRow {
    pub id: String,
    /// The daemon's own label for this provider.
    pub label: String,
    pub standing: ProviderStanding,
    /// The account the daemon named, where it named one.
    pub account: Option<String>,
    /// The model the daemon reports in use, where this is the primary.
    pub model: Option<String>,
    pub verb: Option<ProviderVerb>,
    pub primary: bool,
    pub configured: bool,
    pub present_key: bool,
    /// The `secret.set` id this row's key dialog writes to, where there is one.
    /// Never minted from the provider id: a provider id is not a secret id.
    pub secret_id: Option<String>,
    /// The section the daemon publishes for this provider's own rows.
    pub section: String,
}

impl ProviderRow {
    /// Whether this row's verb can be carried out right now. A verb that writes
    /// a secret waits for the slot the daemon published rather than guessing
    /// one.
    pub fn can_perform(&self) -> bool {
        match self.verb {
            Some(verb) if verb.writes_secret() => self.secret_id.is_some(),
            Some(_) => true,
            None => false,
        }
    }
}

/// The section one provider's own rows are published under.
pub fn section_of(provider: &str) -> String {
    format!("providers.{provider}")
}

/// The provider each import source answers for. The pairing is the contract's
/// own: `claude_code` exists to answer for Anthropic and `codex_cli` for the
/// ChatGPT provider, and nothing else can pair them.
pub fn import_source(provider: &str) -> Option<DetectTarget> {
    match provider {
        ANTHROPIC => Some(DetectTarget::ClaudeCode),
        CODEX => Some(DetectTarget::CodexCli),
        _ => None,
    }
}

/// Anthropic, the one provider whose doors are not the ones its `auth_modes`
/// suggest.
const ANTHROPIC: &str = "anthropic";
/// The ChatGPT subscription provider.
const CODEX: &str = "openai_codex";

/// Every provider `auth.start` will actually start a browser sign-in for.
const BROWSER_SIGN_IN: &[&str] = &[CODEX, "xai"];

/// Providers, as one surface reads them.
pub struct ProvidersModel {
    settings: Rc<SettingsModel>,
    /// The sign-in in flight, and the address it published once.
    sign_in: RefCell<Option<SignIn>>,
    sign_in_job: Rc<JobRunner>,
    probe_job: Rc<JobRunner>,
    /// Which provider the probe is for, so the result lands on its row.
    probing: RefCell<Option<String>>,
    refusal: RefCell<Option<Sentence>>,
    observers: Observers,
}

/// One sign-in a person is in the middle of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub provider: String,
    /// The address the daemon published, returned once. Shown as copyable text
    /// where the browser did not open.
    pub authorize_url: Option<String>,
    /// Whether this is an adopted sign-in rather than a browser hop.
    pub imported: bool,
}

impl ProvidersModel {
    /// A providers model over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let api = settings.api();
        Rc::new(Self {
            settings,
            sign_in: RefCell::new(None),
            sign_in_job: JobRunner::new(Rc::clone(&api)),
            probe_job: JobRunner::new(api),
            probing: RefCell::new(None),
            refusal: RefCell::new(None),
            observers: Observers::default(),
        })
    }

    /// Tell me when something this surface draws moves.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// The job behind the sign-in in flight.
    pub fn sign_in_job(&self) -> Rc<JobRunner> {
        Rc::clone(&self.sign_in_job)
    }

    /// The job behind the probe in flight.
    pub fn probe_job(&self) -> Rc<JobRunner> {
        Rc::clone(&self.probe_job)
    }

    /// The provider the probe in flight belongs to.
    pub fn probing(&self) -> Option<String> {
        self.probing.borrow().clone()
    }

    /// The sign-in in flight, where there is one.
    pub fn sign_in(&self) -> Option<SignIn> {
        self.sign_in.borrow().clone()
    }

    /// The daemon's own sentence for the last refused action.
    pub fn refusal(&self) -> Option<Sentence> {
        self.refusal.borrow().clone()
    }

    /// Read everything this surface draws: the provider rows, every provider's
    /// own section, and the two detections that move a verb.
    pub async fn refresh(&self) {
        self.settings.refresh_setup().await;

        for id in self.provider_ids() {
            self.settings.refresh_section(&section_of(&id)).await;
        }

        self.settings
            .refresh_detections(&[DetectTarget::ClaudeCode, DetectTarget::CodexCli])
            .await;

        self.observers.notify();
    }

    /// The rows, in the daemon's own order.
    pub fn rows(&self) -> Vec<ProviderRow> {
        let state = self.settings.state();
        let Some(setup) = state.setup.as_ref() else {
            return Vec::new();
        };

        setup
            .providers
            .iter()
            .map(|provider| {
                let section = section_of(&provider.id);
                let rows = state.rows(&section);
                let verb = verb_for(provider, self.detected(DetectTarget::ClaudeCode, provider));

                ProviderRow {
                    id: provider.id.clone(),
                    label: provider.label.clone(),
                    standing: standing_of(provider),
                    account: provider.account_label.clone(),
                    model: provider.default_model.clone(),
                    verb,
                    primary: provider.primary,
                    configured: provider.configured,
                    present_key: provider.present_key,
                    secret_id: verb.and_then(|verb| secret_id(verb, &rows)),
                    section,
                }
            })
            .collect()
    }

    /// One row, by provider id, out of the answer the daemon last published.
    ///
    /// A sub-page addresses its provider rather than capturing it: every action
    /// re-reads, and a captured row would go on describing the state the page
    /// opened on.
    pub fn row(&self, provider: &str) -> Option<ProviderRow> {
        self.rows().into_iter().find(|row| row.id == provider)
    }

    /// Which provider's own section leads the pane.
    ///
    /// None where the daemon reports no primary, which is a fresh home: there
    /// is no model in use to put at the top of the pane yet.
    pub fn primary(&self) -> Option<ProviderRow> {
        self.rows().into_iter().find(|row| row.primary)
    }

    /// Whether a detection answered that the source this provider adopts from
    /// is present on this machine.
    fn detected(&self, _preferred: DetectTarget, provider: &SetupProviderRow) -> bool {
        let Some(target) = import_source(&provider.id) else {
            return false;
        };
        self.settings
            .state()
            .detection(target)
            .map(|result| result.present)
            .unwrap_or(false)
    }

    fn provider_ids(&self) -> Vec<String> {
        let state = self.settings.state();
        state
            .setup
            .as_ref()
            .map(|setup| {
                setup
                    .providers
                    .iter()
                    .map(|provider| provider.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---- The actions ----------------------------------------------------

    /// `providers.set_primary`, and the side effects the daemon names.
    pub async fn set_primary(&self, provider: &str) -> Result<Vec<String>, Sentence> {
        let issued = ask::<_, ProvidersSetPrimaryResult>(
            self.settings.api().as_ref(),
            "providers.set_primary",
            &ProviderParams {
                provider: provider.to_string(),
            },
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Err(self.record(Sentence {
                code: None,
                text: crate::copy::text(Key::BusyQueueFull),
                reason: None,
            })),
            Some(Ok(result)) => {
                self.refresh().await;
                Ok(result.side_effects)
            }
            Some(Err(error)) => Err(self.record(Sentence::of(&error))),
        }
    }

    /// `auth.logout`: the local session is forgotten and nothing is revoked
    /// upstream.
    pub async fn sign_out(&self, provider: &str) -> Result<(), Sentence> {
        let issued = ask::<_, AuthLogoutResult>(
            self.settings.api().as_ref(),
            "auth.logout",
            &ProviderParams {
                provider: provider.to_string(),
            },
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Ok(()),
            Some(Ok(_)) => {
                self.refresh().await;
                Ok(())
            }
            Some(Err(error)) => Err(self.record(Sentence::of(&error))),
        }
    }

    /// `auth.start`: the browser hop, and the address it publishes once.
    pub async fn start_sign_in(&self, provider: &str) -> Result<SignIn, Sentence> {
        let issued = ask::<_, AuthStartResult>(
            self.settings.api().as_ref(),
            "auth.start",
            &ProviderParams {
                provider: provider.to_string(),
            },
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Err(self.record(Sentence {
                code: None,
                text: crate::copy::text(Key::BusyQueueFull),
                reason: None,
            })),
            Some(Ok(result)) => {
                let started = SignIn {
                    provider: provider.to_string(),
                    authorize_url: result.authorize_url.clone(),
                    imported: false,
                };
                self.sign_in.replace(Some(started.clone()));
                self.sign_in_job.adopt(result.job);
                self.observers.notify();
                Ok(started)
            }
            Some(Err(error)) => Err(self.record(Sentence::of(&error))),
        }
    }

    /// `auth.import.start`: adopt a sign-in this machine already has.
    pub async fn import_sign_in(
        &self,
        provider: &str,
        source: DetectTarget,
    ) -> Result<SignIn, Sentence> {
        let source = match source {
            DetectTarget::ClaudeCode => "claude_code",
            DetectTarget::CodexCli => "codex_cli",
            // Only two sources adopt a sign-in, and the pairing is the
            // contract's. Anything else is a defect at the call site.
            other => {
                return Err(self.record(Sentence {
                    code: None,
                    text: format!("{other:?}"),
                    reason: None,
                }));
            }
        };

        let issued = ask::<_, AuthStartResult>(
            self.settings.api().as_ref(),
            "auth.import.start",
            &AuthImportParams {
                source: source.to_string(),
            },
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Err(self.record(Sentence {
                code: None,
                text: crate::copy::text(Key::BusyQueueFull),
                reason: None,
            })),
            Some(Ok(result)) => {
                let started = SignIn {
                    provider: provider.to_string(),
                    authorize_url: result.authorize_url.clone(),
                    imported: true,
                };
                self.sign_in.replace(Some(started.clone()));
                self.sign_in_job.adopt(result.job);
                self.observers.notify();
                Ok(started)
            }
            Some(Err(error)) => Err(self.record(Sentence::of(&error))),
        }
    }

    /// The sign-in is over, however it ended: the rows are read again and the
    /// dialog's state is dropped.
    pub async fn sign_in_finished(&self) {
        self.sign_in.replace(None);
        self.refresh().await;
    }

    /// `providers.probe.start`: one metered call, as a job.
    pub async fn probe(&self, provider: &str) -> Result<JobView, Sentence> {
        let issued = ask::<_, JobView>(
            self.settings.api().as_ref(),
            "providers.probe.start",
            &ProbeParams {
                provider: provider.to_string(),
            },
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Err(self.record(Sentence {
                code: None,
                text: crate::copy::text(Key::BusyQueueFull),
                reason: None,
            })),
            Some(Ok(job)) => {
                self.probing.replace(Some(provider.to_string()));
                self.probe_job.adopt(job.clone());
                self.observers.notify();
                Ok(job)
            }
            Some(Err(error)) => Err(self.record(Sentence::of(&error))),
        }
    }

    /// `providers.models.list`: one page of models.
    ///
    /// A live listing never degrades to the catalog: the two answer different
    /// questions, and the daemon's own refusal is what a failed live fetch
    /// renders.
    pub async fn models(
        &self,
        provider: &str,
        query: Option<String>,
        cursor: Option<String>,
        live: bool,
    ) -> Result<ProvidersModelsListResult, Sentence> {
        let issued = ask::<_, ProvidersModelsListResult>(
            self.settings.api().as_ref(),
            "providers.models.list",
            &ProvidersModelsListParams {
                provider: provider.to_string(),
                live: live.then_some(true),
                query,
                cursor,
                limit: None,
            },
            READ_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => Err(Sentence {
                code: None,
                text: crate::copy::text(Key::BusyQueueFull),
                reason: None,
            }),
            Some(Ok(result)) => Ok(result),
            Some(Err(error)) => Err(Sentence::of(&error)),
        }
    }

    fn record(&self, sentence: Sentence) -> Sentence {
        self.refusal.replace(Some(sentence.clone()));
        self.observers.notify();
        sentence
    }

    /// Read the rows again on the main context, for a caller that is not
    /// already inside one.
    pub fn reload(self: &Rc<Self>) {
        let model = Rc::clone(self);
        spawn(async move {
            model.refresh().await;
        });
    }
}

/// What the probe answered, in the daemon's own numbers.
pub fn probe_sentence(job: &JobView) -> Option<String> {
    if let Some(failure) = job.failure.as_ref() {
        return Some(failure.sentence.clone());
    }

    match job.status {
        JobStatus::Running => job
            .phase
            .as_deref()
            .and_then(super::jobs::phase_word)
            .map(crate::copy::text),
        JobStatus::Completed => {
            let result = job.result.as_ref()?;
            let model = result.get("model")?.as_str()?;
            let latency = result.get("latency_ms")?.as_u64()?;
            Some(crate::copy::fill(
                Key::ProviderProbeResult,
                &[("{count}", &latency.to_string()), ("{model}", model)],
            ))
        }
        JobStatus::Cancelled => Some(crate::copy::text(Key::JobCancelled)),
        JobStatus::TimedOut => Some(crate::copy::text(Key::JobTimedOut)),
        _ => None,
    }
}

/// The six status words, in the order they win.
fn standing_of(provider: &SetupProviderRow) -> ProviderStanding {
    if TokenState::of(provider.token_state.as_deref()).is_stale() {
        return ProviderStanding::Reconnect;
    }
    if provider.primary && !provider.configured {
        return ProviderStanding::PrimaryNotConnected;
    }
    if provider.primary {
        return ProviderStanding::Primary;
    }
    if provider.configured {
        return ProviderStanding::Connected;
    }
    if provider.present_key {
        return ProviderStanding::KeyStored;
    }
    ProviderStanding::NotConnected
}

/// The verb, which the detections move and nothing else does.
fn verb_for(provider: &SetupProviderRow, adoptable: bool) -> Option<ProviderVerb> {
    let modes = AuthMode::of(&provider.auth_modes);

    // A provider that takes no credential has nothing to add: Ollama answers on
    // localhost, and a key verb on it is a button that can never be pressed.
    if modes == [AuthMode::None] {
        return None;
    }

    let token_usable = !TokenState::of(provider.token_state.as_deref()).is_stale();
    // A provider the daemon already reports as working has nothing for the list
    // to do: replacing a key and signing out live on its own sub-page.
    //
    // Being primary or holding a key is deliberately not required. The macOS
    // projection still asks for one of them
    // (FermixAppCore/Settings/Panes/ProviderProjection.swift:315), and that
    // rule is wrong for a provider signed in through a browser: an OAuth
    // provider stores no key, and the engine promotes a new sign-in to primary
    // only when nothing else already is. Sign in to Codex on a machine that
    // already has a primary and both halves are false, so the row went on
    // offering Sign In to an account it was signed in to — while the same
    // provider's own page offered Sign Out. The divergence is stated here
    // rather than left to be discovered.
    if provider.configured && token_usable {
        return None;
    }

    if adoptable {
        return match import_source(&provider.id) {
            Some(DetectTarget::ClaudeCode) => Some(ProviderVerb::ImportClaudeCode),
            Some(DetectTarget::CodexCli) => Some(ProviderVerb::ImportCodexCli),
            _ => None,
        };
    }

    // Before the sign-in branch, because Anthropic publishes `oauth` and has no
    // browser flow: its doors are an adopted Claude Code sign-in and a setup
    // token.
    if provider.id == ANTHROPIC {
        return Some(ProviderVerb::AddSetupToken);
    }
    if modes.contains(&AuthMode::Oauth) && BROWSER_SIGN_IN.contains(&provider.id.as_str()) {
        return Some(ProviderVerb::SignIn);
    }
    if modes.contains(&AuthMode::ApiKey) {
        return Some(ProviderVerb::AddKey);
    }
    None
}

/// The `secret.set` id a verb writes to.
///
/// A provider id is never a secret id: the slot is read off the secret row the
/// daemon published in that provider's own section. Anthropic's setup token is
/// the one id no descriptor row can carry, and the contract names it.
fn secret_id(verb: ProviderVerb, rows: &[SettingsRow]) -> Option<String> {
    match verb {
        ProviderVerb::AddSetupToken => Some(crate::management::vocabulary::SETUP_TOKEN.to_string()),
        ProviderVerb::AddKey => rows
            .iter()
            .find(|row| row.kind == SettingsRowKind::Secret)
            .map(|row| row.key.clone()),
        _ => None,
    }
}

/// The verbs a provider's own sub-page offers for changing how it signs in.
///
/// Replacing a connected account uses the same doors as its first connection,
/// without signing the current one out first.
pub fn detail_verbs(provider: &str, adoptable: bool, auth_mode: Option<&str>) -> Vec<ProviderVerb> {
    if auth_mode == Some(AuthMode::ApiKey.recorded()) {
        return Vec::new();
    }

    let mut verbs = Vec::new();
    if BROWSER_SIGN_IN.contains(&provider) {
        verbs.push(ProviderVerb::SignIn);
    }
    if adoptable {
        match import_source(provider) {
            Some(DetectTarget::ClaudeCode) => verbs.push(ProviderVerb::ImportClaudeCode),
            Some(DetectTarget::CodexCli) => verbs.push(ProviderVerb::ImportCodexCli),
            _ => {}
        }
    }
    if provider == ANTHROPIC {
        verbs.push(ProviderVerb::AddSetupToken);
    }
    verbs
}
