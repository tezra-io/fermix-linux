//! What the cloud provider sells, on the wire (Always On setup §3): `home.provision.options`, the
//! one provisioner call left to a surface, read where an action needs a choice of size and place.
//! A provider is an id here, never a name, and a location is the provider's own word for it.
//! Decoding and the call only; how a choice is drawn is the window's.

use crate::management::{CallError, Management};
use serde::Deserialize;
use serde_json::json;

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

/// A size and a location: what a create makes.
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

impl Management {
    /// What `provider` sells now; the provider is read live.
    pub fn provision_options(&self, provider: &str) -> Result<Options, CallError> {
        assert!(!provider.is_empty(), "options are a provider's");
        let answer = self.call("home.provision.options", json!({ "provider": provider }))?;
        serde_json::from_value(answer).map_err(|e| {
            CallError::Protocol(format!(
                "home.provision.options answered an unexpected shape: {e}"
            ))
        })
    }
}
