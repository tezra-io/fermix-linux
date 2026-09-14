//! Home, as data.
//!
//! One snapshot per read: the state word, the rows that need someone, the
//! runtime facts and the two background controls. Every value in it is the
//! daemon's, the command line's or this session's; nothing here decides a state
//! any of them already decided.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::copy::Key;
use crate::management::types::SettingsPane;
use crate::management::vocabulary::{gaps, Gap, Readiness};
use crate::service::types::Alignment;
use crate::session::build::{self, GuiAlignment};
use crate::session::{DesktopFacts, DesktopSession};

use super::settings_model::{SettingsModel, State};
use super::{spawn, Poller};

/// How often Home re-reads the overview while it is in front of someone.
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// The most polls one visible stretch performs: six hours at the interval
/// above. Reaching it stops the poll, and the next time the page is shown or
/// the window is brought forward it starts again.
pub const POLL_CAP: u32 = 4_320;

/// The state word, which is one of exactly four, or none yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusWord {
    Running,
    SetupRequired,
    RestartToFinishUpdating,
    NotRunning,
    /// Nothing has been read yet, so nothing is claimed.
    Unknown,
}

impl StatusWord {
    /// The word for this state.
    pub fn key(self) -> Key {
        match self {
            StatusWord::Running => Key::HomeStatusRunning,
            StatusWord::SetupRequired => Key::HomeStatusSetupRequired,
            StatusWord::RestartToFinishUpdating => Key::HomeStatusRestartToFinishUpdating,
            StatusWord::NotRunning => Key::HomeStatusNotRunning,
            StatusWord::Unknown => Key::HomeRuntimeUnavailable,
        }
    }
}

/// The one prominent toolbar action, while its condition holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction {
    ContinueSetup,
    FinishUpdating,
}

impl ToolbarAction {
    /// The words on the button.
    pub fn key(self) -> Key {
        match self {
            ToolbarAction::ContinueSetup => Key::HomeToolbarContinueSetup,
            ToolbarAction::FinishUpdating => Key::HomeToolbarFinishUpdating,
        }
    }
}

/// What one attention row's button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionAction {
    /// Open the pane that can close this gap.
    OpenPane(SettingsPane, Key),
    /// Open the restart dialog.
    Restart,
    /// The same confirmation, under the words M38 section 6.5 fixes for the one
    /// row that finishes an update.
    FinishUpdating,
    /// Read the settings file again.
    Reload,
    /// Open Doctor, where the engine's own check and remediation for this live.
    OpenDoctor,
    /// Quit, so the version the package installed is the one that opens next.
    /// This process cannot reopen itself, and the row says so.
    Quit,
}

impl AttentionAction {
    /// The words on the button.
    pub fn key(&self) -> Key {
        match self {
            AttentionAction::OpenPane(_, key) => *key,
            AttentionAction::Restart => Key::MenuRestart,
            AttentionAction::FinishUpdating => Key::SkewRestartToFinish,
            AttentionAction::Reload => Key::BannerReloadFromDisk,
            AttentionAction::OpenDoctor => Key::AttentionOpenDoctorAction,
            AttentionAction::Quit => Key::MenuQuit,
        }
    }
}

/// What a row is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTitle {
    /// The product's words for a gap it has a template for.
    Words(Key),
    /// The identifier the daemon wrote, for a gap this build has no words for.
    /// Shown as an identifier, because an id with its first letter raised is a
    /// spelling the application invented.
    Identifier(String),
}

/// One row under Attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionRow {
    /// The daemon's own key for this gap, which is the row's identity.
    pub id: String,
    /// The product's words for what the gap is.
    pub title: RowTitle,
    /// What the daemon named inside it: a channel, a provider, a path. Rendered
    /// as the daemon wrote it.
    pub detail: Option<String>,
    /// The one thing to do about it.
    pub action: Option<AttentionAction>,
}

/// One labelled runtime fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub label: Key,
    pub value: String,
}

/// Everything Home draws, in one read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSnapshot {
    pub status: StatusWord,
    pub toolbar: Option<ToolbarAction>,
    pub attention: Vec<AttentionRow>,
    pub facts: Vec<Fact>,
    pub background_enabled: bool,
    pub open_at_login: bool,
}

/// Home's own model: the poll it owns and the session facts it observes.
pub struct HomeModel {
    settings: Rc<SettingsModel>,
    poller: Poller,
    desktop: RefCell<Option<DesktopFacts>>,
}

impl HomeModel {
    /// A Home model over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        Rc::new(Self {
            settings,
            poller: Poller::new(),
            desktop: RefCell::new(None),
        })
    }

    /// Take this session's own observation, once.
    pub async fn observe_desktop(&self) {
        if self.desktop.borrow().is_some() {
            return;
        }
        let facts = DesktopSession::observe().await;
        self.desktop.replace(Some(facts));
    }

    /// Start the overview poll. Home is the only surface that reads the
    /// overview, and it reads it only while someone is looking at it.
    pub fn start_polling(self: &Rc<Self>) {
        let model = Rc::clone(self);
        self.poller.start(POLL_INTERVAL, POLL_CAP, move || {
            let settings = Rc::clone(&model.settings);
            spawn(async move {
                settings.refresh_overview().await;
            });
            true
        });
    }

    /// Stop it. Leaving the surface, or the window going behind another one.
    pub fn stop_polling(&self) {
        self.poller.stop();
    }

    /// Whether the poll is running.
    pub fn is_polling(&self) -> bool {
        self.poller.is_running()
    }

    /// Everything Home draws.
    pub fn snapshot(&self) -> HomeSnapshot {
        let state = self.settings.state();
        let desktop = self.desktop.borrow().clone();

        HomeSnapshot {
            status: status_word(&state),
            toolbar: toolbar_action(&state),
            attention: attention(&state, build::alignment()),
            facts: facts(
                &state,
                desktop.as_ref(),
                self.settings.api().negotiated_version(),
            ),
            background_enabled: state.background_enabled(),
            open_at_login: state.open_at_login,
        }
    }
}

/// Every row under Attention: the daemon's own gaps first, then what comparing
/// builds says.
pub fn attention(state: &State, gui: GuiAlignment) -> Vec<AttentionRow> {
    let mut rows = attention_rows(state);
    rows.extend(skew_rows(state, gui));
    rows
}

/// The state word, in the order the four are decided.
pub fn status_word(state: &State) -> StatusWord {
    match state.reachable {
        None => StatusWord::Unknown,
        Some(false) => StatusWord::NotRunning,
        Some(true) => {
            if readiness(state) == Readiness::SetupRequired {
                return StatusWord::SetupRequired;
            }
            if alignment(state) == Some(Alignment::PendingRestart) {
                return StatusWord::RestartToFinishUpdating;
            }
            StatusWord::Running
        }
    }
}

/// The one prominent action, and only while its condition holds.
pub fn toolbar_action(state: &State) -> Option<ToolbarAction> {
    match status_word(state) {
        StatusWord::SetupRequired => Some(ToolbarAction::ContinueSetup),
        StatusWord::RestartToFinishUpdating => Some(ToolbarAction::FinishUpdating),
        _ => None,
    }
}

/// The readiness behind the status word.
///
/// `overview.get` is the source M38 section 5.6 names for Status, and it is the
/// read that is available one release back. The setup state answers only when
/// the overview has not been read yet: the two are the same daemon, and
/// preferring the one the design names keeps one answer rather than two.
fn readiness(state: &State) -> Readiness {
    let from_overview = state
        .overview
        .as_ref()
        .map(|overview| Readiness::parse(overview.readiness.status.as_deref()));

    match from_overview {
        Some(Readiness::Unknown) | None => state
            .setup
            .as_ref()
            .map(|setup| Readiness::parse(setup.readiness.status.as_deref()))
            .unwrap_or(Readiness::Unknown),
        Some(known) => known,
    }
}

fn alignment(state: &State) -> Option<Alignment> {
    state.service.as_ref().map(|status| status.alignment)
}

/// One row per gap the daemon reported, in the order it reported them.
pub fn attention_rows(state: &State) -> Vec<AttentionRow> {
    let Some(setup) = state.setup.as_ref() else {
        return Vec::new();
    };

    let panes: Vec<Option<SettingsPane>> = setup
        .readiness
        .failures
        .iter()
        .map(|failure| Some(failure.pane))
        .collect();

    gaps(setup)
        .into_iter()
        .enumerate()
        .map(|(index, gap)| attention_row(state, gap, panes.get(index).copied().flatten()))
        .collect()
}

/// The rows that come from comparing builds rather than from the daemon's own
/// readiness (M38 section 9.2).
///
/// Three facts, each with its own row: what is installed against what is
/// running, and this window against the application on disk. An identity that
/// could not be established draws the row that says so rather than none, and
/// never one that claims alignment.
pub fn skew_rows(state: &State, gui: GuiAlignment) -> Vec<AttentionRow> {
    let mut rows = Vec::new();

    match alignment(state) {
        Some(Alignment::PendingRestart) => rows.push(AttentionRow {
            id: "engine-skew".to_string(),
            title: RowTitle::Words(Key::SkewNewerInstalledTitle),
            detail: Some(crate::copy::text(Key::SkewNewerInstalledDetail)),
            action: Some(AttentionAction::FinishUpdating),
        }),
        Some(Alignment::Unknown) => rows.push(AttentionRow {
            id: "engine-unknown".to_string(),
            title: RowTitle::Words(Key::SkewUnknownIdentityTitle),
            detail: Some(crate::copy::text(Key::SkewUnknownIdentityBody)),
            action: None,
        }),
        Some(Alignment::OwnershipConflict) => rows.push(AttentionRow {
            id: "engine-conflict".to_string(),
            title: RowTitle::Words(Key::SkewOwnershipConflictTitle),
            detail: Some(crate::copy::text(Key::SkewOwnershipConflictBody)),
            action: None,
        }),
        _ => {}
    }

    if gui == GuiAlignment::Stale {
        rows.push(AttentionRow {
            id: "gui-skew".to_string(),
            title: RowTitle::Words(Key::SkewStaleGuiTitle),
            detail: Some(crate::copy::text(Key::SkewStaleGuiBody)),
            action: Some(AttentionAction::Quit),
        });
    }

    rows
}

fn attention_row(state: &State, gap: Gap, pane: Option<SettingsPane>) -> AttentionRow {
    let (title, action) = words_and_action(&gap, pane);

    AttentionRow {
        id: identity(&gap),
        title,
        detail: detail(state, &gap),
        action,
    }
}

/// What one gap is called, and the one thing to do about it.
fn words_and_action(gap: &Gap, pane: Option<SettingsPane>) -> (RowTitle, Option<AttentionAction>) {
    let open = |pane, key| Some(AttentionAction::OpenPane(pane, key));

    match gap {
        Gap::Personalization => (
            RowTitle::Words(Key::AttentionPersonalizationTitle),
            open(
                SettingsPane::Personality,
                Key::AttentionPersonalizationAction,
            ),
        ),
        Gap::Provider(_) => (
            RowTitle::Words(Key::AttentionProviderTitle),
            open(SettingsPane::Providers, Key::AttentionProviderAction),
        ),
        Gap::Channel(_) => (
            RowTitle::Words(Key::AttentionChannelTitle),
            open(SettingsPane::Channels, Key::AttentionChannelAction),
        ),
        Gap::Realtime => (
            RowTitle::Words(Key::AttentionRealtimeTitle),
            open(SettingsPane::Voice, Key::AttentionRealtimeAction),
        ),
        Gap::RestartPending => (
            RowTitle::Words(Key::AttentionRestartPendingTitle),
            Some(AttentionAction::Restart),
        ),
        Gap::ExternalConfigChange => (
            RowTitle::Words(Key::BannerExternalChangeTitle),
            Some(AttentionAction::Reload),
        ),
        // The settings file cannot be read, so there is no reload to offer: the
        // reload would re-run the read that failed. Recovery is where this goes,
        // and the banner is what takes someone there.
        Gap::ConfigUnreadable => (RowTitle::Words(Key::RecoveryConfigUnreadableTitle), None),
        Gap::LegacyServiceUnit => (
            RowTitle::Words(Key::AttentionLegacyServiceUnitTitle),
            Some(AttentionAction::OpenDoctor),
        ),
        Gap::EnginePathBaseline => (
            RowTitle::Words(Key::AttentionEnginePathBaselineTitle),
            Some(AttentionAction::OpenDoctor),
        ),
        // A gap this build has no words for renders under the daemon's own
        // component name, with the deep link the daemon published if this build
        // can route to it.
        Gap::Unrecognized(key) => (
            RowTitle::Identifier(key.clone()),
            pane.filter(|pane| *pane != SettingsPane::Unrecognized)
                .map(|pane| AttentionAction::OpenPane(pane, Key::ActionOpenSettingsPane)),
        ),
    }
}

fn identity(gap: &Gap) -> String {
    match gap {
        Gap::Personalization => "personalization".to_string(),
        Gap::Provider(Some(provider)) => format!("provider:{provider}"),
        Gap::Provider(None) => "provider".to_string(),
        Gap::Channel(name) => format!("channel:{name}"),
        Gap::Realtime => "realtime".to_string(),
        Gap::RestartPending => "restart".to_string(),
        Gap::ExternalConfigChange => "external-change".to_string(),
        Gap::ConfigUnreadable => "unreadable".to_string(),
        Gap::LegacyServiceUnit => "legacy-unit".to_string(),
        Gap::EnginePathBaseline => "path-baseline".to_string(),
        Gap::Unrecognized(key) => key.clone(),
    }
}

/// The daemon's own supporting fact for one gap, where the wire carries one.
fn detail(state: &State, gap: &Gap) -> Option<String> {
    match gap {
        Gap::Provider(Some(provider)) => Some(provider_label(state, provider)),
        Gap::Channel(name) => Some(channel_label(state, name)),
        Gap::RestartPending => {
            let reasons: Vec<String> = state
                .restart
                .reasons
                .iter()
                .map(|reason| reason.sentence.clone())
                .collect();
            if reasons.is_empty() {
                None
            } else {
                Some(reasons.join(" "))
            }
        }
        Gap::ConfigUnreadable => state.unreadable.as_ref().map(|reason| reason.text.clone()),
        Gap::LegacyServiceUnit => state
            .setup
            .as_ref()
            .and_then(|setup| setup.coexistence.legacy_service_unit.path.clone()),
        Gap::EnginePathBaseline => state
            .service
            .as_ref()
            .map(|status| status.path_source.clone()),
        _ => None,
    }
}

/// The product's name for a provider: the daemon's label where it published
/// one, and the identifier exactly as the daemon wrote it otherwise.
fn provider_label(state: &State, provider: &str) -> String {
    state
        .setup
        .as_ref()
        .and_then(|setup| {
            setup
                .providers
                .iter()
                .find(|row| row.id == provider)
                .map(|row| row.label.clone())
        })
        .unwrap_or_else(|| provider.to_string())
}

/// The product's name for a channel, which is its section's own title.
fn channel_label(state: &State, channel: &str) -> String {
    let section = format!("channels.{channel}");
    state
        .sections
        .iter()
        .find(|candidate| candidate.id == section)
        .map(|candidate| candidate.title.clone())
        .unwrap_or_else(|| channel.to_string())
}

/// The nine labelled facts, in the order M38 section 5.6 lists them.
pub fn facts(state: &State, desktop: Option<&DesktopFacts>, protocol: Option<u32>) -> Vec<Fact> {
    vec![
        Fact {
            label: Key::HomeRuntimeEngine,
            value: engine_fact(state),
        },
        Fact {
            label: Key::HomeRuntimeManagementProtocol,
            value: protocol
                .map(|version| version.to_string())
                .unwrap_or_else(unavailable),
        },
        Fact {
            label: Key::HomeRuntimeUptime,
            value: uptime_fact(state),
        },
        Fact {
            label: Key::HomeRuntimeProvider,
            value: state
                .overview
                .as_ref()
                .and_then(|overview| provider_fact(state, overview))
                .unwrap_or_else(none),
        },
        Fact {
            label: Key::HomeRuntimeChannels,
            value: channels_fact(state),
        },
        Fact {
            label: Key::HomeRuntimeSkills,
            value: capabilities(state)
                .map(|counts| counts.skill.to_string())
                .unwrap_or_else(unavailable),
        },
        Fact {
            label: Key::HomeRuntimeTools,
            // Tools are the callable population: what this build ships plus
            // what the integrations add. Skills are counted separately, which
            // is what M38 section 5.6 asks for.
            value: capabilities(state)
                .map(|counts| (counts.builtin + counts.mcp).to_string())
                .unwrap_or_else(unavailable),
        },
        Fact {
            label: Key::HomeRuntimeService,
            value: state
                .service
                .as_ref()
                .and_then(|status| status.sub_state.clone())
                .unwrap_or_else(unavailable),
        },
        Fact {
            label: Key::HomeRuntimeSession,
            value: desktop.and_then(session_fact).unwrap_or_else(unavailable),
        },
    ]
}

/// A fact the daemon has not answered for yet, and one it answered with
/// nothing. They are different things and they read differently.
fn unavailable() -> String {
    crate::copy::text(Key::HomeRuntimeUnavailable)
}

fn none() -> String {
    crate::copy::text(Key::HomeRuntimeNone)
}

fn capabilities(state: &State) -> Option<crate::management::types::OverviewCapabilities> {
    state
        .overview
        .as_ref()
        .map(|overview| overview.capabilities)
}

/// The engine's own version: its identity where `hello` has landed, and the
/// overview's answer otherwise.
fn engine_fact(state: &State) -> String {
    state
        .hello
        .as_ref()
        .map(|hello| hello.engine.product_version.clone())
        .or_else(|| {
            state
                .overview
                .as_ref()
                .and_then(|overview| overview.daemon.version.clone())
        })
        .unwrap_or_else(unavailable)
}

fn uptime_fact(state: &State) -> String {
    state
        .overview
        .as_ref()
        .and_then(|overview| overview.daemon.uptime_ms)
        .map(uptime)
        .unwrap_or_else(unavailable)
}

/// The channels that are switched on, by the names the daemon wrote.
fn channels_fact(state: &State) -> String {
    let Some(overview) = state.overview.as_ref() else {
        return unavailable();
    };

    let enabled: Vec<String> = overview
        .channels
        .iter()
        .filter(|channel| channel.enabled)
        .map(|channel| channel.name.clone())
        .collect();

    if enabled.is_empty() {
        return none();
    }
    enabled.join(", ")
}

/// The provider fact carries the model beside it, because the fact lives only
/// on this row.
fn provider_fact(
    state: &State,
    overview: &crate::management::types::OverviewResult,
) -> Option<String> {
    let active = overview.provider.active.as_ref()?;
    if active.is_empty() {
        return None;
    }

    let label = provider_label(state, active);
    Some(match overview.provider.model.as_ref() {
        Some(model) if !model.is_empty() => joined(&label, model),
        _ => label,
    })
}

fn session_fact(desktop: &DesktopFacts) -> Option<String> {
    match (desktop.session_type.as_deref(), desktop.desktop.as_deref()) {
        (Some(session), Some(name)) => Some(joined(session, name)),
        (Some(session), None) => Some(session.to_string()),
        (None, Some(name)) => Some(name.to_string()),
        (None, None) => None,
    }
}

/// Two facts on one line. The separator is punctuation, not a word: it carries
/// no meaning to translate and no casing to get wrong.
pub fn joined(leading: &str, trailing: &str) -> String {
    format!("{leading} \u{00b7} {trailing}")
}

/// How long the daemon has been up, in the two largest units that fit.
///
/// Unit abbreviations rather than words, because a duration in words needs a
/// plural rule per language and this one is read at a glance.
pub fn uptime(milliseconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    let seconds = milliseconds / 1_000;

    if seconds >= DAY {
        return crate::copy::fill(
            Key::HomeUptimeDaysHours,
            &[
                ("{days}", &(seconds / DAY).to_string()),
                ("{hours}", &((seconds % DAY) / HOUR).to_string()),
            ],
        );
    }
    if seconds >= HOUR {
        return crate::copy::fill(
            Key::HomeUptimeHoursMinutes,
            &[
                ("{hours}", &(seconds / HOUR).to_string()),
                ("{minutes}", &((seconds % HOUR) / MINUTE).to_string()),
            ],
        );
    }
    if seconds >= MINUTE {
        return crate::copy::fill(
            Key::HomeUptimeMinutes,
            &[("{minutes}", &(seconds / MINUTE).to_string())],
        );
    }

    crate::copy::text(Key::HomeUptimeJustStarted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_reads_in_the_two_largest_units_that_fit() {
        assert_eq!(uptime(0), "Just started");
        assert_eq!(uptime(59_000), "Just started");
        assert_eq!(uptime(90_000), "1 min");
        assert_eq!(uptime(3_600_000 + 120_000), "1 h 2 min");
        assert_eq!(uptime(86_400_000 * 2 + 3_600_000 * 3), "2 d 3 h");
    }

    #[test]
    fn nothing_read_yet_claims_nothing() {
        let state = State::default();
        assert_eq!(status_word(&state), StatusWord::Unknown);
        assert_eq!(toolbar_action(&state), None);
        assert!(attention_rows(&state).is_empty());
    }

    #[test]
    fn a_daemon_that_does_not_answer_is_not_running() {
        let state = State {
            reachable: Some(false),
            ..State::default()
        };
        assert_eq!(status_word(&state), StatusWord::NotRunning);
        assert_eq!(toolbar_action(&state), None);
    }

    #[test]
    fn the_facts_say_unavailable_rather_than_nothing() {
        let facts = facts(&State::default(), None, None);
        assert_eq!(facts.len(), 9);
        for fact in &facts {
            assert!(!fact.value.is_empty(), "{:?} has no value", fact.label);
        }
    }
}
