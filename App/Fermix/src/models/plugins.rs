//! Integrations, as data.
//!
//! Every word on an integration row is the daemon's: the status sentence, the
//! consent sentence, the disclosure and the verbs. What this file owns is which
//! method each published action id runs, the four counted filters, the one
//! search rule both row kinds are read by, and the order of the one chain a
//! person starts with a single gesture.
//!
//! The pairing is the contract's and it is never crossed: a button is painted
//! with `verbs[i]` and runs `actions[i]`. Routing on a word, or inferring the
//! next operation from a status atom, is how a button came to say one thing and
//! do another.

use std::cell::RefCell;
use std::rc::Rc;

use crate::copy::Key;
use crate::management::types::{
    JobView, PluginAction, PluginNameParams, PluginOAuthClient, PluginOAuthClientSetParams,
    PluginRow, PluginRowResult, PluginRuntimeKind, PluginSettingSetParams, PluginWorkspace,
    PluginWorkspaceSelectParams, PluginsListResult, SettingsPane, SetupFeatures,
};

use super::api::{accept, ask, READ_DEADLINE, WRITE_DEADLINE};
use super::jobs::JobRunner;
use super::settings_model::{Sentence, SettingsModel};
use super::Observers;

/// The four counted filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationFilter {
    Installed,
    Available,
    Mcps,
    Features,
}

/// The filters, in the order they are drawn.
pub const FILTERS: &[IntegrationFilter] = &[
    IntegrationFilter::Installed,
    IntegrationFilter::Available,
    IntegrationFilter::Mcps,
    IntegrationFilter::Features,
];

impl IntegrationFilter {
    /// The word on the toggle.
    pub fn key(self) -> Key {
        match self {
            IntegrationFilter::Installed => Key::IntegrationsFilterInstalled,
            IntegrationFilter::Available => Key::IntegrationsFilterAvailable,
            IntegrationFilter::Mcps => Key::IntegrationsFilterMcps,
            IntegrationFilter::Features => Key::IntegrationsFilterFeatures,
        }
    }

    /// Whether one plugin row belongs under this filter.
    ///
    /// The runtime kinds are the one runtime vocabulary this door reads, and it
    /// reads them to filter rather than to word anything.
    pub fn admits(self, row: &IntegrationRow) -> bool {
        match self {
            IntegrationFilter::Installed => row.installed,
            IntegrationFilter::Available => !row.installed,
            IntegrationFilter::Mcps => matches!(
                row.runtime_kind,
                Some(PluginRuntimeKind::RemoteMcp) | Some(PluginRuntimeKind::LocalStdio)
            ),
            IntegrationFilter::Features => false,
        }
    }
}

/// One integration row, as the daemon published it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationRow {
    pub name: String,
    pub title: String,
    /// The manifest's own one-line description.
    pub summary: Option<String>,
    /// The daemon's own sentence for where this plugin stands.
    pub status: String,
    pub primary_verb: Option<String>,
    pub primary_action: Option<PluginAction>,
    /// The words to paint, paired index for index with the actions below.
    pub verbs: Vec<String>,
    pub actions: Vec<PluginAction>,
    pub installed: bool,
    pub enabled: bool,
    pub runtime_kind: Option<PluginRuntimeKind>,
    pub credential_present: bool,
    /// The sign-in family this plugin belongs to, which is what names its entry
    /// among the sign-in clients.
    pub auth_provider: Option<String>,
    pub consent: String,
    pub disclosure: Option<String>,
    pub settings: Vec<crate::management::types::PluginSetting>,
    pub access_profiles: Vec<crate::management::types::PluginAccessProfile>,
    pub workspaces: Vec<PluginWorkspace>,
    pub workspace_label: Option<String>,
}

impl IntegrationRow {
    /// The row's second line.
    ///
    /// An installed row reads the daemon's own status sentence, because where
    /// it stands is the whole question about something already on this machine.
    /// A row that is not installed reads the manifest summary, because what it
    /// does is the only question there is about it yet.
    pub fn subtitle(&self) -> &str {
        if self.installed {
            return &self.status;
        }
        match self.summary.as_deref() {
            Some(summary) if !summary.trim().is_empty() => summary,
            _ => &self.status,
        }
    }

    /// The buttons a detail draws: the daemon's own pairs, minus the ones
    /// answered by a slot of their own and the ones this build cannot route.
    pub fn buttons(&self) -> Vec<(String, PluginAction)> {
        self.verbs
            .iter()
            .cloned()
            .zip(self.actions.iter().copied())
            .filter(|(_, action)| draws_button(*action))
            .collect()
    }

    /// Whether this plugin binds to a workspace at all.
    pub fn binds_workspace(&self) -> bool {
        !self.access_profiles.is_empty()
    }

    /// Whether the search matches this row.
    pub fn matches(&self, query: &str) -> bool {
        matches(query, &self.title, self.summary.as_deref())
    }
}

/// Whether a detail draws a button for one action.
///
/// Neither token verb does: the credential slot is the secret row, which is the
/// one door to that slot, and a button beside it would be a second control for
/// one thing. An id this build cannot route draws nothing either, because a
/// button that routes nowhere is worse than none.
pub fn draws_button(action: PluginAction) -> bool {
    !matches!(
        action,
        PluginAction::AddToken | PluginAction::ReplaceToken | PluginAction::Unrecognized
    )
}

/// Whether an action is answered by a dialog rather than by a call.
pub fn answered_by_dialog(action: PluginAction) -> bool {
    matches!(
        action,
        PluginAction::AddToken
            | PluginAction::ReplaceToken
            | PluginAction::SetUpClient
            | PluginAction::ChooseWorkspace
    )
}

/// The leading steps a person has to carry out themselves.
///
/// These four are the ids whose method is a door the operator walks through, so
/// the row's detail is opened on them once a switch-on has landed. Every other
/// id is either something the daemon does on its own or something the switch
/// just did.
pub fn needs_the_operator(action: PluginAction) -> bool {
    matches!(
        action,
        PluginAction::SignIn
            | PluginAction::AddToken
            | PluginAction::SetUpClient
            | PluginAction::ChooseWorkspace
    )
}

/// The page's one search rule: the name and the one-line description.
pub fn matches(query: &str, title: &str, summary: Option<&str>) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }

    title.to_lowercase().contains(&needle)
        || summary.unwrap_or_default().to_lowercase().contains(&needle)
}

/// One native driver feature, which is a daemon setting rather than a plugin.
///
/// A row here says where it stands and opens the pane that owns its switch. It
/// draws no switch of its own: a second writer of one daemon key would need
/// that key's name written in Rust, which is the field inventory the design
/// forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureRow {
    /// The mark key, which is the app's own name for this driver.
    pub id: &'static str,
    pub title: Key,
    pub summary: Key,
    /// Where the daemon says it stands, or nothing where nothing has been read.
    /// Three-valued, because off on an unread snapshot is a claim about a
    /// daemon nobody asked.
    pub enabled: Option<bool>,
    /// The pane that owns this feature's switch.
    pub pane: SettingsPane,
}

impl FeatureRow {
    /// The word for where it stands.
    pub fn standing(&self) -> Key {
        match self.enabled {
            Some(true) => Key::StateOn,
            Some(false) => Key::StateOff,
            None => Key::StateNotReported,
        }
    }

    /// Whether the search matches this row.
    pub fn matches(&self, query: &str) -> bool {
        matches(
            query,
            &crate::copy::text(self.title),
            Some(&crate::copy::text(self.summary)),
        )
    }
}

/// The native driver rows, from the daemon's own feature flags.
///
/// Two, not three: computer history is macOS-only in the engine, so this door
/// renders no row for it anywhere. An unread snapshot still draws both, because
/// a row that vanished while the daemon was being read would move the count for
/// a reason nobody could see.
pub fn feature_rows(features: Option<&SetupFeatures>) -> Vec<FeatureRow> {
    vec![
        FeatureRow {
            id: "computer_use",
            title: Key::IntegrationsFeatureComputerUse,
            summary: Key::IntegrationsFeatureComputerUseBody,
            enabled: features.map(|features| features.computer_use),
            pane: SettingsPane::Computer,
        },
        FeatureRow {
            id: "meetings",
            title: Key::IntegrationsFeatureMeetings,
            summary: Key::IntegrationsFeatureMeetingsBody,
            enabled: features.map(|features| features.meetings),
            pane: SettingsPane::Meetings,
        },
    ]
}

/// What a switch-on left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnableOutcome {
    /// The daemon refused a stage. The chain stops and the sentence is shown.
    Refused(Sentence),
    /// The daemon's next step needs the person, so the detail is opened on it.
    Configure(Box<IntegrationRow>),
    /// Nothing is left to do here.
    Done,
}

/// Integrations, as one surface reads them.
pub struct PluginsModel {
    settings: Rc<SettingsModel>,
    catalogue: RefCell<Option<PluginsListResult>>,
    refusal: RefCell<Option<Sentence>>,
    job: Rc<JobRunner>,
    observers: Observers,
}

impl PluginsModel {
    /// An integrations model over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let api = settings.api();
        Rc::new(Self {
            settings,
            catalogue: RefCell::new(None),
            refusal: RefCell::new(None),
            job: JobRunner::new(api),
            observers: Observers::default(),
        })
    }

    /// Tell me when the catalogue moves.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// The one runner every plugin job runs through.
    pub fn job(&self) -> Rc<JobRunner> {
        Rc::clone(&self.job)
    }

    /// The daemon's own sentence for the last refused action.
    pub fn refusal(&self) -> Option<Sentence> {
        self.refusal.borrow().clone()
    }

    /// Forget the last refusal, which is what starting another action does.
    pub fn clear_refusal(&self) {
        self.refusal.replace(None);
    }

    /// `plugins.list`.
    pub async fn refresh(&self) {
        let issued = ask::<_, PluginsListResult>(
            self.settings.api().as_ref(),
            "plugins.list",
            &serde_json::json!({}),
            READ_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => return,
            Some(Ok(catalogue)) => {
                self.catalogue.replace(Some(catalogue));
            }
            Some(Err(error)) => {
                self.refusal.replace(Some(Sentence::of(&error)));
            }
        }

        self.observers.notify();
    }

    /// Whether the catalogue has been read at all.
    pub fn is_loaded(&self) -> bool {
        self.catalogue.borrow().is_some()
    }

    /// The plugin rows, in the daemon's own order.
    pub fn rows(&self) -> Vec<IntegrationRow> {
        self.catalogue
            .borrow()
            .as_ref()
            .map(|catalogue| catalogue.plugins.iter().map(row_of).collect())
            .unwrap_or_default()
    }

    /// One row, by name, out of the answer the daemon last published.
    pub fn row(&self, name: &str) -> Option<IntegrationRow> {
        self.rows().into_iter().find(|row| row.name == name)
    }

    /// The sign-in clients the daemon published.
    pub fn clients(&self) -> Vec<PluginOAuthClient> {
        self.catalogue
            .borrow()
            .as_ref()
            .map(|catalogue| catalogue.oauth_clients.clone())
            .unwrap_or_default()
    }

    /// The sign-in client one row's credential would be registered under.
    ///
    /// `auth_provider` is the daemon's own tie between a plugin and a client;
    /// deriving it from the plugin's name would be this door deciding which
    /// family a plugin signs in with.
    pub fn client_for(&self, row: &IntegrationRow) -> Option<PluginOAuthClient> {
        let provider = row.auth_provider.as_ref()?;
        self.clients()
            .into_iter()
            .find(|client| client.provider == *provider)
    }

    /// The native driver rows.
    pub fn features(&self) -> Vec<FeatureRow> {
        let state = self.settings.state();
        feature_rows(state.setup.as_ref().map(|setup| &setup.features))
    }

    /// The count under each filter.
    pub fn count(&self, filter: IntegrationFilter) -> usize {
        match filter {
            IntegrationFilter::Features => self.features().len(),
            other => self.rows().iter().filter(|row| other.admits(row)).count(),
        }
    }

    // ---- The actions ----------------------------------------------------

    /// Run one action, addressed by the id the daemon published.
    ///
    /// Answers the daemon's sentence on a refusal and nothing on success. The
    /// install and the checks are jobs; the rest answer with the row.
    pub async fn perform(&self, action: PluginAction, name: &str) -> Option<Sentence> {
        match action {
            PluginAction::Install => self.start_job("plugins.install.start", name).await,
            PluginAction::Check => self.start_job("plugins.check.start", name).await,
            PluginAction::Enable => self.call("plugins.enable", name).await,
            PluginAction::Disable => self.call("plugins.disable", name).await,
            PluginAction::Disconnect => self.call("plugins.disconnect", name).await,
            // A sign-in is the same browser hop a provider takes, under the
            // plugin spelling the contract publishes, and the surface that
            // started it owns the dialog it runs in. This is not the door for
            // it, and the one caller routes it before it reaches here.
            PluginAction::SignIn => None,
            // The four doors a person walks through are answered by a dialog,
            // and the caller opens it rather than performing anything.
            PluginAction::AddToken
            | PluginAction::ReplaceToken
            | PluginAction::SetUpClient
            | PluginAction::ChooseWorkspace => None,
            PluginAction::Unrecognized => None,
        }
    }

    /// The row's switch, both ways, and what the page does after it.
    ///
    /// Enabling something is not the same as finishing it: the daemon publishes
    /// the state it is left in, so the answer is read again and a next step that
    /// needs the operator opens the row's detail rather than being left to be
    /// found. Nothing else is started: a sign-in this door raised by itself
    /// would be a browser window nobody asked for.
    pub async fn set_enabled(&self, on: bool, name: &str) -> EnableOutcome {
        let action = if on {
            PluginAction::Enable
        } else {
            PluginAction::Disable
        };

        if let Some(sentence) = self.perform(action, name).await {
            return EnableOutcome::Refused(sentence);
        }
        if !on {
            return EnableOutcome::Done;
        }

        self.next_step(name)
    }

    /// What the daemon's own answer says is left to do on one row.
    ///
    /// Read out of the re-read the enable performed rather than out of the row
    /// the switch was drawn from: that one is the state before the write.
    pub fn next_step(&self, name: &str) -> EnableOutcome {
        let Some(row) = self.row(name) else {
            return EnableOutcome::Done;
        };
        match row.primary_action {
            Some(action) if needs_the_operator(action) => EnableOutcome::Configure(Box::new(row)),
            _ => EnableOutcome::Done,
        }
    }

    /// The install a consent answered has ended.
    ///
    /// Enabling something not yet installed is one gesture, so this finishes it:
    /// a run that worked is followed by the enable the switch asked for. A
    /// refusal answers with the daemon's own sentence and enables nothing, and
    /// either way the catalogue is read again because the install moved it.
    pub async fn install_finished(&self, name: &str) -> EnableOutcome {
        let failure = self
            .job
            .job()
            .and_then(|job| job.failure)
            .map(|failure| Sentence {
                code: None,
                text: failure.sentence,
                reason: None,
            });

        if let Some(sentence) = failure {
            self.refusal.replace(Some(sentence.clone()));
            self.refresh().await;
            return EnableOutcome::Refused(sentence);
        }

        self.refresh().await;
        self.set_enabled(true, name).await
    }

    /// `plugins.workspaces.discover.start`.
    pub async fn discover_workspaces(&self, name: &str) -> Option<Sentence> {
        self.start_job("plugins.workspaces.discover.start", name)
            .await
    }

    /// `plugins.workspace.select.start`.
    ///
    /// The daemon republishes the binding on the plugin's row, so the catalogue
    /// is read again once the job has started: the workspace label a row shows
    /// is the daemon's answer and never the label this dialog happened to send.
    pub async fn select_workspace(
        &self,
        name: &str,
        profile: &str,
        workspace: &PluginWorkspace,
    ) -> Option<Sentence> {
        let params = PluginWorkspaceSelectParams {
            name: name.to_string(),
            profile: profile.to_string(),
            workspace_id: workspace.id.clone(),
            label: workspace.label.clone(),
        };

        let issued = ask::<_, JobView>(
            self.settings.api().as_ref(),
            "plugins.workspace.select.start",
            &params,
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => None,
            Some(Ok(job)) => {
                self.job.adopt(job);
                self.refresh().await;
                None
            }
            Some(Err(error)) => Some(self.record(Sentence::of(&error))),
        }
    }

    /// `plugins.oauth_client.set`.
    ///
    /// The region is sent exactly where the client row publishes regions to
    /// choose from: the daemon refuses one for a provider that serves a single
    /// region, so nothing is what that means here.
    pub async fn set_oauth_client(
        &self,
        provider: &str,
        client_id: &str,
        redirect_port: u32,
        region: Option<String>,
    ) -> Option<Sentence> {
        let params = PluginOAuthClientSetParams {
            provider: provider.to_string(),
            client_id: client_id.to_string(),
            redirect_port,
            region,
        };

        let issued = ask::<_, crate::management::types::PluginOAuthClientResult>(
            self.settings.api().as_ref(),
            "plugins.oauth_client.set",
            &params,
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => None,
            Some(Ok(_)) => {
                self.refresh().await;
                None
            }
            Some(Err(error)) => Some(self.record(Sentence::of(&error))),
        }
    }

    /// `plugins.setting.set`. The value is always a string on the wire, and a
    /// boolean setting takes only the two words the contract names.
    pub async fn set_setting(&self, name: &str, key: &str, value: &str) -> Option<Sentence> {
        let params = PluginSettingSetParams {
            name: name.to_string(),
            key: key.to_string(),
            value: value.to_string(),
        };

        let issued = ask::<_, PluginRowResult>(
            self.settings.api().as_ref(),
            "plugins.setting.set",
            &params,
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => None,
            Some(Ok(_)) => {
                self.refresh().await;
                None
            }
            Some(Err(error)) => Some(self.record(Sentence::of(&error))),
        }
    }

    /// One call that answers with the row.
    async fn call(&self, method: &str, name: &str) -> Option<Sentence> {
        let params = PluginNameParams {
            name: name.to_string(),
        };
        let issued = ask::<_, PluginRowResult>(
            self.settings.api().as_ref(),
            method,
            &params,
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => None,
            Some(Ok(_)) => {
                self.refresh().await;
                None
            }
            Some(Err(error)) => Some(self.record(Sentence::of(&error))),
        }
    }

    /// One call that answers with a job.
    async fn start_job(&self, method: &str, name: &str) -> Option<Sentence> {
        let params = PluginNameParams {
            name: name.to_string(),
        };
        let issued = ask::<_, JobView>(
            self.settings.api().as_ref(),
            method,
            &params,
            WRITE_DEADLINE,
        )
        .await;

        match accept(self.settings.api().as_ref(), issued) {
            None => None,
            Some(Ok(job)) => {
                self.job.adopt(job);
                self.observers.notify();
                None
            }
            Some(Err(error)) => Some(self.record(Sentence::of(&error))),
        }
    }

    fn record(&self, sentence: Sentence) -> Sentence {
        self.refusal.replace(Some(sentence.clone()));
        self.observers.notify();
        sentence
    }
}

fn row_of(plugin: &PluginRow) -> IntegrationRow {
    IntegrationRow {
        name: plugin.name.clone(),
        title: plugin.title.clone(),
        summary: plugin.summary.clone(),
        status: plugin.status_sentence.clone(),
        primary_verb: plugin.primary_verb.clone(),
        primary_action: plugin.primary_action,
        verbs: plugin.verbs.clone(),
        actions: plugin.actions.clone(),
        installed: plugin.installed,
        enabled: plugin.enabled,
        runtime_kind: plugin.runtime_kind,
        credential_present: plugin.credential_present,
        auth_provider: plugin.auth_provider.clone(),
        consent: plugin.consent_sentence.clone(),
        disclosure: plugin.remote_disclosure.clone(),
        settings: plugin.settings.clone(),
        access_profiles: plugin.access_profiles.clone(),
        workspaces: plugin.workspaces.clone(),
        workspace_label: plugin.workspace_label.clone(),
    }
}
