//! The Always On provisioner on the wire (M75 lean v1 §4.3): the seven `home.provision.*` calls
//! that create the server at a cloud provider from a token pasted once, and their answers. Which
//! providers there are, and every word their screens draw, is the engine's
//! (`home.provision.providers`): a provider is an id here, never a name, and a location is the
//! provider's own word for it. The token and the ledger stay on the computer that created the
//! server, so these are answered here even on a desktop whose home moved. Decoding and the calls
//! only; what a screen shows of it is `always_on.rs`'s.

use crate::management::{checked_job, CallError, Management};
use crate::model::JobView;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;

/// The methods the provisioner's screens need, every one of them listed by `hello` before they
/// are drawn.
pub const METHODS: [&str; 7] = [
    "home.provision.providers",
    "home.provision.connect",
    "home.provision.options",
    "home.provision.create",
    "home.provision.power",
    "home.provision.resources",
    "home.provision.delete",
];

/// One cloud provider Fermix creates the server on, in the engine's words: what its token screen
/// draws. `id` is what connect, options and create take as `provider`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Provider {
    pub id: String,
    /// How every sentence names it.
    pub name: String,
    /// The token field's label.
    pub token_label: String,
    /// The "How to get one" steps.
    pub steps: Vec<String>,
    /// The provider's console, which Open console opens.
    pub console_url: String,
    /// Its section of the Always On docs page.
    pub docs_anchor: String,
}

/// `home.provision.providers`' answer.
#[derive(Debug, Deserialize)]
struct Providers {
    providers: Vec<Provider>,
}

/// One of the places the provider sells in: its own word for it, and the city it names.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProvisionLocation {
    pub id: String,
    pub city: String,
    pub country: String,
}

/// One server type in one location. The prices are monthly and gross, as decimal strings in the
/// account's currency; `total_monthly` adds the IPv4 address.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Offer {
    #[serde(rename = "type")]
    pub size: String,
    pub description: String,
    pub cores: u32,
    pub memory_gb: f64,
    pub disk_gb: u32,
    pub dedicated: bool,
    pub location: String,
    pub monthly: String,
    pub ipv4_monthly: String,
    pub total_monthly: String,
}

/// A size and a location: what Create server makes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Pick {
    #[serde(rename = "type")]
    pub size: String,
    pub location: String,
}

/// `home.provision.options`: what the provider sells now, read live.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Options {
    pub currency: String,
    pub locations: Vec<ProvisionLocation>,
    pub offers: Vec<Offer>,
    pub recommended: Option<Pick>,
    pub more_room: Option<Pick>,
    /// The recommendation, or that nothing is in stock.
    pub sentence: String,
}

impl Options {
    /// The offer for `pick`, while the provider sells it.
    pub fn offer(&self, pick: &Pick) -> Option<&Offer> {
        self.offers
            .iter()
            .find(|o| o.size == pick.size && o.location == pick.location)
    }

    pub fn city(&self, location: &str) -> Option<&str> {
        self.locations
            .iter()
            .find(|l| l.id == location)
            .map(|l| l.city.as_str())
    }
}

/// `home.provision.*`'s view of what Fermix created, from this computer's ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionState {
    None,
    Connected,
    Creating,
    Created,
    Deleting,
    Gone,
    Failed,
    Unreadable,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenState {
    None,
    Accepted,
    /// The provider stopped accepting it.
    Refused,
    #[serde(other)]
    Unknown,
}

/// Where one resource Fermix created stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceStatus {
    Requested,
    Creating,
    Created,
    Deleting,
    Gone,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProvisionServer {
    pub name: String,
    #[serde(rename = "type")]
    pub size: String,
    pub location: String,
    pub status: ResourceStatus,
    /// The provider's own word for the server's power, as last read.
    pub power: Option<String>,
    pub ipv4: Option<String>,
    pub created_at: Option<String>,
    pub seen_at: Option<String>,
}

/// Why the last create stopped, or why deleting a replaced server stopped. `reason` is the
/// engine's word (`other` for the provider's own refusal); the sentence is what is shown.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Failure {
    pub reason: String,
    pub sentence: String,
    pub next_location: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplaceState {
    Creating,
    Waiting,
    Moving,
    Deleting,
    Done,
    Failed,
    Cancelling,
    #[serde(other)]
    Unknown,
}

/// Which server holds the home during a replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holder {
    Old,
    New,
    #[serde(other)]
    Unknown,
}

/// A replace of the server from the linked desktop.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Replace {
    pub state: ReplaceState,
    pub current: Holder,
    pub old: Option<ProvisionServer>,
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Server,
    Firewall,
    PrimaryIp,
    #[serde(other)]
    Unknown,
}

/// One thing Fermix created and the provider bills for until it is deleted.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Resource {
    pub kind: ResourceKind,
    pub name: Option<String>,
    /// The provider's own id for it, opaque here.
    pub id: Option<String>,
    pub status: ResourceStatus,
    pub replaced: bool,
}

/// `home.provision.connect` and `home.provision.resources`: the provisioner's view. The token is
/// never answered.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProvisionView {
    /// The provider in use: the one Fermix created on, else the one whose token was given last
    /// (connect's: the one connected).
    pub provider: String,
    pub state: ProvisionState,
    pub token: TokenState,
    pub server: Option<ProvisionServer>,
    pub failure: Option<Failure>,
    pub replace: Option<Replace>,
    pub resources: Vec<Resource>,
    pub sentence: Option<String>,
    /// `connect`: the account holds servers Fermix did not create.
    #[serde(default)]
    pub other_servers: Option<bool>,
    /// `resources`: whether the provider was read just now.
    #[serde(default)]
    pub checked: Option<bool>,
}

/// What `home.provision.power` does to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    On,
    Reset,
}

impl PowerAction {
    fn wire(self) -> &'static str {
        match self {
            PowerAction::On => "on",
            PowerAction::Reset => "reset",
        }
    }
}

/// A provider's API token as it was pasted, trimmed. It goes out on the wire and nowhere else: no
/// print of it spells it, and nothing keeps it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// The pasted text without the space around it; None when nothing but space was pasted. How
    /// a token must look is the daemon's rule, and its refusal says so.
    pub fn pasted(text: &str) -> Option<Token> {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| Token(trimmed.to_owned()))
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(withheld)")
    }
}

impl Management {
    /// Every provider Fermix creates the server on, in the order the screens offer them.
    pub fn provision_providers(&self) -> Result<Vec<Provider>, CallError> {
        let answer: Providers = decoded(
            "home.provision.providers",
            self.call("home.provision.providers", json!({}))?,
        )?;
        Ok(answer.providers)
    }

    /// Checks the token with `provider` and keeps it on this computer only. Its answer never
    /// holds the token, and no error built here quotes it.
    pub fn provision_connect(
        &self,
        provider: &str,
        token: &Token,
    ) -> Result<ProvisionView, CallError> {
        assert!(!provider.is_empty(), "a token is for a provider");
        let params = json!({ "provider": provider, "token": token.0 });
        decoded(
            "home.provision.connect",
            self.call("home.provision.connect", params)?,
        )
    }

    pub fn provision_options(&self, provider: &str) -> Result<Options, CallError> {
        assert!(!provider.is_empty(), "options are a provider's");
        decoded(
            "home.provision.options",
            self.call("home.provision.options", json!({ "provider": provider }))?,
        )
    }

    /// Creates the server at `provider`, as a job; `replace` creates the one that replaces the
    /// server the home lives on, from that home's linked desktop.
    pub fn provision_create(
        &self,
        provider: &str,
        pick: &Pick,
        replace: bool,
    ) -> Result<JobView, CallError> {
        assert!(!provider.is_empty(), "a server is created at a provider");
        assert!(!pick.size.is_empty(), "a server is created of a size");
        assert!(!pick.location.is_empty(), "a server is created somewhere");
        let mut params =
            json!({ "provider": provider, "type": pick.size, "location": pick.location });
        if replace {
            params["replace"] = json!(true);
        }
        checked_job(decoded(
            "home.provision.create",
            self.call("home.provision.create", params)?,
        )?)
    }

    pub fn provision_power(&self, action: PowerAction) -> Result<JobView, CallError> {
        let params = json!({ "action": action.wire() });
        checked_job(decoded(
            "home.provision.power",
            self.call("home.provision.power", params)?,
        )?)
    }

    /// The view read back from the provider in use first; the ledger as it stands when it could
    /// not be read (`checked` false).
    pub fn provision_resources(&self) -> Result<ProvisionView, CallError> {
        decoded(
            "home.provision.resources",
            self.call("home.provision.resources", json!({}))?,
        )
    }

    pub fn provision_delete(&self) -> Result<JobView, CallError> {
        checked_job(decoded(
            "home.provision.delete",
            self.call("home.provision.delete", json!({}))?,
        )?)
    }
}

fn decoded<T: DeserializeOwned>(method: &str, answer: Value) -> Result<T, CallError> {
    serde_json::from_value(answer)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}
