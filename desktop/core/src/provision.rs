//! The Always On provisioner on the wire (M75 lean v1 §4.3): the six `home.provision.*` calls
//! that create the server on Hetzner from a token pasted once, and their answers. The token and
//! the ledger stay on the computer that created the server, so these are answered here even on a
//! desktop whose home moved. Decoding and the calls only; what a screen shows of it is
//! `always_on.rs`'s.

use crate::management::{checked_job, CallError, Management};
use crate::model::JobView;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;

/// The methods the Hetzner path needs, every one of them listed by `hello` before it is drawn.
pub const METHODS: [&str; 6] = [
    "home.provision.connect",
    "home.provision.options",
    "home.provision.create",
    "home.provision.power",
    "home.provision.resources",
    "home.provision.delete",
];

/// A Hetzner location Fermix creates a server in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    Fsn1,
    Nbg1,
    Hel1,
    /// A location this app does not know: decoded, never sent.
    #[serde(other)]
    Unknown,
}

impl Location {
    /// The word the protocol takes, for a location this app knows.
    pub fn wire(self) -> Option<&'static str> {
        match self {
            Location::Fsn1 => Some("fsn1"),
            Location::Nbg1 => Some("nbg1"),
            Location::Hel1 => Some("hel1"),
            Location::Unknown => None,
        }
    }
}

/// One of the locations Hetzner sells in, with its city as Hetzner names it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProvisionLocation {
    pub id: Location,
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
    pub location: Location,
    pub monthly: String,
    pub ipv4_monthly: String,
    pub total_monthly: String,
}

/// A size and a location: what Create server makes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Pick {
    #[serde(rename = "type")]
    pub size: String,
    pub location: Location,
}

/// `home.provision.options`: what Hetzner sells now, read live.
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
    /// The offer for `pick`, while Hetzner sells it.
    pub fn offer(&self, pick: &Pick) -> Option<&Offer> {
        self.offers
            .iter()
            .find(|o| o.size == pick.size && o.location == pick.location)
    }

    pub fn city(&self, location: Location) -> Option<&str> {
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
    /// Hetzner stopped accepting it.
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
    pub location: Location,
    pub status: ResourceStatus,
    /// Hetzner's own word for the server's power, as last read.
    pub power: Option<String>,
    pub ipv4: Option<String>,
    pub created_at: Option<String>,
    pub seen_at: Option<String>,
}

/// Why the last create stopped, or why deleting a replaced server stopped.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Failure {
    pub reason: String,
    pub sentence: String,
    pub next_location: Option<Location>,
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

/// One thing Fermix created and Hetzner bills for until it is deleted.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Resource {
    pub kind: ResourceKind,
    pub name: Option<String>,
    pub id: Option<u64>,
    pub status: ResourceStatus,
    pub replaced: bool,
}

/// `home.provision.connect` and `home.provision.resources`: the provisioner's view. The token is
/// never answered.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProvisionView {
    pub state: ProvisionState,
    pub token: TokenState,
    pub server: Option<ProvisionServer>,
    pub failure: Option<Failure>,
    pub replace: Option<Replace>,
    pub resources: Vec<Resource>,
    pub sentence: Option<String>,
    /// `connect`: the project holds servers Fermix did not create.
    #[serde(default)]
    pub other_servers: Option<bool>,
    /// `resources`: whether Hetzner was read just now.
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

/// A Hetzner API token as it was pasted, trimmed. It goes out on the wire and nowhere else: no
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
    /// Checks the token with Hetzner and keeps it on this computer only. Its answer never holds
    /// the token, and no error built here quotes it.
    pub fn provision_connect(&self, token: &Token) -> Result<ProvisionView, CallError> {
        let params = json!({ "token": token.0 });
        decoded(
            "home.provision.connect",
            self.call("home.provision.connect", params)?,
        )
    }

    pub fn provision_options(&self) -> Result<Options, CallError> {
        decoded(
            "home.provision.options",
            self.call("home.provision.options", json!({}))?,
        )
    }

    /// Creates the server, as a job; `replace` creates the one that replaces the server the
    /// home lives on, from that home's linked desktop.
    pub fn provision_create(&self, pick: &Pick, replace: bool) -> Result<JobView, CallError> {
        let location = pick
            .location
            .wire()
            .expect("a server is created only in a location this app knows");
        assert!(!pick.size.is_empty(), "a server is created of a size");
        let mut params = json!({ "type": pick.size, "location": location });
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

    /// The view read back from Hetzner first; the ledger as it stands when Hetzner could not
    /// be read (`checked` false).
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
