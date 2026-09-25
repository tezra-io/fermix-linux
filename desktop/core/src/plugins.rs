//! Integrations: the plugins the daemon publishes, and the rules the pane draws
//! them by. Every word on a row is the daemon's; this module only decides which
//! rows a filter shows, which method a button runs, and what follows a
//! switch-on. Rows decode tolerantly, so a newer daemon's fields never break it.

use crate::management::{CallError, Management};
use crate::model::{Features, JobStatus, JobView};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The runtimes that make a plugin an MCP server: a local process or a remote one.
const MCP_RUNTIMES: [&str; 2] = ["local_stdio", "remote_mcp"];

/// Job kinds whose end moves the plugin rows, so the list is read again.
const PLUGIN_JOBS: [&str; 4] = [
    "plugin_install",
    "plugin_check",
    "plugin_workspaces_discover",
    "plugin_workspace_select",
];

pub const PORT_INVALID: &str =
    "Enter a port from 1 to 65535, or leave it blank to use the default.";
pub const REGION_MISSING: &str = "Choose the account's region.";
pub const CLIENT_ID_MISSING: &str = "Enter the client ID.";
pub const SECRET_FIRST: &str = "Add the client secret first.";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PluginList {
    pub plugins: Vec<PluginRow>,
    #[serde(default)]
    pub oauth_clients: Vec<OAuthClient>,
}

/// One plugin. `status` is for logs only: nothing is drawn or decided from it.
/// `account_label` is personal: it is shown to its owner and never logged.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PluginRow {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub runtime_kind: Option<String>,
    #[serde(default)]
    pub auth_kind: Option<String>,
    #[serde(default)]
    pub auth_provider: Option<String>,
    pub installed: bool,
    pub enabled: bool,
    #[serde(default)]
    pub status: Option<String>,
    pub status_sentence: String,
    #[serde(default)]
    pub primary_verb: Option<String>,
    #[serde(default)]
    pub primary_action: Option<String>,
    #[serde(default)]
    pub verbs: Vec<String>,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub settings: Vec<PluginSetting>,
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub credential_present: bool,
    pub consent_sentence: String,
    #[serde(default)]
    pub remote_disclosure: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub access_profiles: Vec<AccessProfile>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_label: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    #[default]
    Text,
    Boolean,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PluginSetting {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub kind: SettingKind,
}

impl PluginSetting {
    /// A boolean setting travels as the text "true" or "false".
    pub fn is_on(&self) -> bool {
        self.value.as_deref() == Some("true")
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AccessProfile {
    pub id: String,
    pub label: String,
    /// This level can change data in the workspace, so choosing it is warned about.
    #[serde(default)]
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub label: String,
}

/// A sign-in client: the app registration a family of plugins signs in through.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OAuthClient {
    pub provider: String,
    pub configured: bool,
    /// `None` means the daemon's own default is in force.
    #[serde(default)]
    pub redirect_port: Option<u16>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub secret_present: bool,
    #[serde(default)]
    pub region: Option<String>,
    /// Non-empty means a region is required; empty means sending one is refused.
    #[serde(default)]
    pub regions: Vec<Region>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Region {
    pub id: String,
    pub label: String,
}

/// What `plugins.oauth_client.set` is sent. Blank keys are left out, never zeroed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientAnswer {
    pub provider: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// The daemon's closed set of action ids. A button routes on these, never on its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginAction {
    Install,
    Enable,
    Disable,
    SignIn,
    AddToken,
    ReplaceToken,
    SetUpClient,
    ChooseWorkspace,
    Check,
    Disconnect,
}

impl PluginAction {
    /// The action for a wire id, or `None` for one this build does not know.
    pub fn parse(wire: &str) -> Option<PluginAction> {
        let action = match wire {
            "install" => PluginAction::Install,
            "enable" => PluginAction::Enable,
            "disable" => PluginAction::Disable,
            "sign_in" => PluginAction::SignIn,
            "add_token" => PluginAction::AddToken,
            "replace_token" => PluginAction::ReplaceToken,
            "set_up_client" => PluginAction::SetUpClient,
            "choose_workspace" => PluginAction::ChooseWorkspace,
            "check" => PluginAction::Check,
            "disconnect" => PluginAction::Disconnect,
            _ => return None,
        };
        Some(action)
    }
}

/// One button: the daemon's word painted on it, and the action it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verb {
    pub label: String,
    pub action: PluginAction,
    /// The row's leading button.
    pub primary: bool,
}

/// The row's buttons in the daemon's order: `verbs[i]` painted, `actions[i]`
/// routed. An action this build does not know draws no button.
pub fn verbs(row: &PluginRow) -> Vec<Verb> {
    row.verbs
        .iter()
        .zip(&row.actions)
        .filter_map(|(label, wire)| {
            let action = PluginAction::parse(wire)?;
            let primary = row.primary_action.as_deref() == Some(wire.as_str())
                && row.primary_verb.as_deref() == Some(label.as_str());
            Some(Verb {
                label: label.clone(),
                action,
                primary,
            })
        })
        .collect()
}

/// The four counted filters over the one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Installed,
    Available,
    Mcps,
    Features,
}

impl Filter {
    pub const ALL: [Filter; 4] = [
        Filter::Installed,
        Filter::Available,
        Filter::Mcps,
        Filter::Features,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Filter::Installed => "Installed",
            Filter::Available => "Available",
            Filter::Mcps => "MCPs",
            Filter::Features => "Features",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Filter::Installed => "installed",
            Filter::Available => "available",
            Filter::Mcps => "mcps",
            Filter::Features => "features",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Filter> {
        Filter::ALL.into_iter().find(|f| f.slug() == slug)
    }

    /// Whether a plugin row belongs under this filter. Features hold no plugins.
    pub fn admits(self, row: &PluginRow) -> bool {
        match self {
            Filter::Installed => row.installed,
            Filter::Available => !row.installed,
            Filter::Mcps => row
                .runtime_kind
                .as_deref()
                .is_some_and(|kind| MCP_RUNTIMES.contains(&kind)),
            Filter::Features => false,
        }
    }
}

pub fn count(filter: Filter, rows: &[PluginRow]) -> usize {
    match filter {
        Filter::Features => FEATURES.len(),
        _ => rows.iter().filter(|r| filter.admits(r)).count(),
    }
}

/// Whether a row's name or description holds the query. A blank query holds everything.
pub fn matches(query: &str, title: &str, summary: Option<&str>) -> bool {
    let query = query.trim().to_lowercase();
    let holds = |text: &str| text.to_lowercase().contains(&query);
    holds(title) || summary.is_some_and(holds)
}

pub fn visible<'a>(rows: &'a [PluginRow], filter: Filter, query: &str) -> Vec<&'a PluginRow> {
    rows.iter()
        .filter(|r| filter.admits(r) && matches(query, &r.title, r.summary.as_deref()))
        .collect()
}

/// The one line under a row's name: where an installed plugin stands, or what
/// an available one does.
pub fn line(row: &PluginRow) -> &str {
    let summary = row.summary.as_deref().filter(|s| !s.trim().is_empty());
    match (row.installed, summary) {
        (false, Some(summary)) => summary,
        _ => &row.status_sentence,
    }
}

/// Whether, after a switch-on, the re-read row leads with a step only the
/// person can take. The pane then opens the detail; it never starts the step.
pub fn needs_operator(row: &PluginRow) -> bool {
    let action = row.primary_action.as_deref().and_then(PluginAction::parse);
    matches!(
        action,
        Some(
            PluginAction::SignIn
                | PluginAction::AddToken
                | PluginAction::SetUpClient
                | PluginAction::ChooseWorkspace
        )
    )
}

/// What the person agrees to before an install: where the plugin runs, and
/// what leaves the machine when it runs elsewhere. The daemon asks nothing itself.
pub fn consent_body(row: &PluginRow) -> String {
    match &row.remote_disclosure {
        Some(disclosure) => format!("{}\n\n{disclosure}", row.consent_sentence),
        None => row.consent_sentence.clone(),
    }
}

/// The `auth.start` provider for a plugin's own sign-in.
pub fn sign_in_provider(name: &str) -> String {
    assert!(!name.is_empty(), "a plugin sign-in names its plugin");
    format!("plugin:{name}")
}

/// The `secret.set` id for a plugin's own token.
pub fn plugin_secret_id(name: &str) -> String {
    assert!(!name.is_empty(), "a plugin token names its plugin");
    format!("plugin:{name}")
}

/// The `secret.set` id for a sign-in client's secret.
pub fn client_secret_id(provider: &str) -> String {
    assert!(!provider.is_empty(), "a client secret names its provider");
    format!("oauth_client:{provider}")
}

/// Checks what the client editor holds and turns it into what the daemon takes.
/// The secret must already be stored; a blank port is left out; a region is
/// sent exactly when the provider offers regions, and must be one of them.
pub fn client_answer(
    client: &OAuthClient,
    client_id: &str,
    port: &str,
    region: Option<&str>,
) -> Result<ClientAnswer, &'static str> {
    if !client.secret_present {
        return Err(SECRET_FIRST);
    }
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err(CLIENT_ID_MISSING);
    }
    let redirect_port = match port.trim() {
        "" => None,
        typed => Some(
            typed
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or(PORT_INVALID)?,
        ),
    };
    let region = if client.regions.is_empty() {
        None
    } else {
        let offered = region.filter(|r| client.regions.iter().any(|o| o.id == *r));
        Some(offered.ok_or(REGION_MISSING)?.to_owned())
    };
    Ok(ClientAnswer {
        provider: client.provider.clone(),
        client_id: client_id.to_owned(),
        redirect_port,
        region,
    })
}

/// A client's state in words, with the account's region where one is chosen.
pub fn client_state(client: &OAuthClient) -> String {
    let state = if client.configured {
        "Configured"
    } else {
        "Not configured"
    };
    let region = client
        .region
        .as_deref()
        .and_then(|id| client.regions.iter().find(|r| r.id == id));
    match region {
        Some(region) => format!("{state}, {}", region.label),
        None => state.to_owned(),
    }
}

/// A client's name: the daemon's spelling where a plugin's title is the
/// provider's own name ("GitHub"), else the provider id capitalised ("Google").
pub fn client_title(list: &PluginList, provider: &str) -> String {
    let spelled = list
        .plugins
        .iter()
        .find(|p| p.title.eq_ignore_ascii_case(provider));
    if let Some(plugin) = spelled {
        return plugin.title.clone();
    }
    let mut chars = provider.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The sign-in client a plugin signs in through, if it has one.
pub fn client_for<'a>(list: &'a PluginList, row: &PluginRow) -> Option<&'a OAuthClient> {
    let provider = row.auth_provider.as_deref()?;
    list.oauth_clients.iter().find(|c| c.provider == provider)
}

/// Plugin jobs still running that the pane is not following yet. Jobs do not
/// name their plugin, so these are followed only to read the list when they end.
pub fn reattachable(jobs: &[JobView], followed: &[String]) -> Vec<JobView> {
    jobs.iter()
        .filter(|job| {
            job.status == JobStatus::Running
                && PLUGIN_JOBS.contains(&job.kind.as_str())
                && !followed.contains(&job.job_id)
        })
        .cloned()
        .collect()
}

/// What a running job is doing, by its kind. The phase is a wire atom, not copy.
pub fn job_words(kind: &str) -> &'static str {
    match kind {
        "plugin_install" => "Installing…",
        "plugin_check" => "Checking…",
        "plugin_workspaces_discover" => "Finding workspaces…",
        "plugin_workspace_select" => "Saving the workspace…",
        "auth" => "Waiting for the browser…",
        _ => "Working…",
    }
}

/// A native feature the Features filter lists. It has no switch here: its row
/// opens the pane that owns it.
pub struct Feature {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    /// The Settings pane slug that owns the feature's switch.
    pub pane: &'static str,
}

/// Computer history is macOS-only, so Linux lists two.
pub const FEATURES: [Feature; 2] = [
    Feature {
        id: "computer_use",
        title: "Computer use",
        summary: "Lets Fermix see the screen and use the pointer, through a separate helper.",
        pane: "computer",
    },
    Feature {
        id: "meetings",
        title: "Meeting notetaker",
        summary: "Joins a meeting as a notetaker and writes the notes up afterwards.",
        pane: "meetings",
    },
];

/// Where a feature stands. Unread is not Off: nobody has asked the daemon yet.
pub fn feature_state(feature: &Feature, features: Option<&Features>) -> &'static str {
    let on = features.and_then(|f| match feature.id {
        "computer_use" => Some(f.computer_use),
        "meetings" => Some(f.meetings),
        _ => None,
    });
    match on {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Not reported",
    }
}

pub fn visible_features(query: &str) -> Vec<&'static Feature> {
    FEATURES
        .iter()
        .filter(|f| matches(query, f.title, Some(f.summary)))
        .collect()
}

/// The writers' answer: the plugin row as it stands after the write.
#[derive(Deserialize)]
struct RowAnswer {
    plugin: PluginRow,
}

#[derive(Deserialize)]
struct ClientSetAnswer {
    oauth_client: OAuthClient,
}

impl Management {
    /// Every plugin and sign-in client. Input-free: the daemon refuses any params key.
    pub fn plugins_list(&self) -> Result<PluginList, CallError> {
        let list: PluginList = typed("plugins.list", self.call("plugins.list", json!({}))?)?;
        let unpaired = list
            .plugins
            .iter()
            .find(|p| p.verbs.len() != p.actions.len());
        if let Some(plugin) = unpaired {
            return Err(CallError::Protocol(format!(
                "plugin {} publishes {} verbs for {} actions",
                plugin.name,
                plugin.verbs.len(),
                plugin.actions.len()
            )));
        }
        Ok(list)
    }

    /// Starts an install. The consent that precedes it is the app's to ask.
    pub fn plugins_install_start(&self, name: &str) -> Result<JobView, CallError> {
        self.plugin_job("plugins.install.start", json!({ "name": named(name) }))
    }

    pub fn plugins_enable(&self, name: &str) -> Result<PluginRow, CallError> {
        self.plugin_row("plugins.enable", json!({ "name": named(name) }))
    }

    pub fn plugins_disable(&self, name: &str) -> Result<PluginRow, CallError> {
        self.plugin_row("plugins.disable", json!({ "name": named(name) }))
    }

    /// Forgets the plugin's credential on this machine. Nothing is revoked upstream.
    pub fn plugins_disconnect(&self, name: &str) -> Result<PluginRow, CallError> {
        self.plugin_row("plugins.disconnect", json!({ "name": named(name) }))
    }

    /// Writes one plugin setting; a switch's value is "true" or "false".
    pub fn plugins_setting_set(
        &self,
        name: &str,
        key: &str,
        value: &str,
    ) -> Result<PluginRow, CallError> {
        assert!(!key.is_empty(), "a plugin setting names its key");
        let params = json!({ "name": named(name), "key": key, "value": value });
        self.plugin_row("plugins.setting.set", params)
    }

    /// Registers a sign-in client. Its secret must be stored with `secret.set` first.
    pub fn plugins_oauth_client_set(
        &self,
        answer: &ClientAnswer,
    ) -> Result<OAuthClient, CallError> {
        assert!(
            !answer.provider.is_empty(),
            "a sign-in client names its provider"
        );
        let params = serde_json::to_value(answer).expect("a client answer always serializes");
        let method = "plugins.oauth_client.set";
        let set: ClientSetAnswer = typed(method, self.call(method, params)?)?;
        Ok(set.oauth_client)
    }

    pub fn plugins_check_start(&self, name: &str) -> Result<JobView, CallError> {
        self.plugin_job("plugins.check.start", json!({ "name": named(name) }))
    }

    /// Lists the workspaces the account can reach. They land on the plugin row, not the job.
    pub fn plugins_workspaces_discover_start(&self, name: &str) -> Result<JobView, CallError> {
        let method = "plugins.workspaces.discover.start";
        self.plugin_job(method, json!({ "name": named(name) }))
    }

    /// Binds the plugin to one workspace at one access level. `label` may be empty.
    pub fn plugins_workspace_select_start(
        &self,
        name: &str,
        profile: &str,
        workspace_id: &str,
        label: &str,
    ) -> Result<JobView, CallError> {
        assert!(
            !profile.is_empty(),
            "a workspace choice names its access level"
        );
        assert!(
            !workspace_id.is_empty(),
            "a workspace choice names its workspace"
        );
        let params = json!({
            "name": named(name), "profile": profile, "workspace_id": workspace_id, "label": label,
        });
        self.plugin_job("plugins.workspace.select.start", params)
    }

    fn plugin_row(&self, method: &str, params: Value) -> Result<PluginRow, CallError> {
        let answer: RowAnswer = typed(method, self.call(method, params)?)?;
        Ok(answer.plugin)
    }

    fn plugin_job(&self, method: &str, params: Value) -> Result<JobView, CallError> {
        let job: JobView = typed(method, self.call(method, params)?)?;
        // The rule `management.rs` holds every job to: one that ended badly says why.
        let ended_badly = matches!(job.status, JobStatus::Failed | JobStatus::TimedOut);
        if ended_badly && job.failure.is_none() {
            return Err(CallError::Protocol(format!(
                "job {} ended badly without a reason",
                job.job_id
            )));
        }
        Ok(job)
    }
}

fn named(name: &str) -> &str {
    assert!(!name.is_empty(), "a plugin call names its plugin");
    name
}

fn typed<T: DeserializeOwned>(method: &str, result: Value) -> Result<T, CallError> {
    serde_json::from_value(result)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}
