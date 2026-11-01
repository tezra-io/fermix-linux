//! What the cloud provider sells, on the wire (Always On setup §3): `home.provision.options`, the
//! one provisioner call left to a surface, read where an action needs a choice of size and place.
//! It takes no params: the engine answers for the provider in use and names it. Each offer comes
//! with the engine's own words for it, its label and its price, and a location is the provider's
//! own word for it. Decoding and the call only; how a choice is drawn is the window's.

use crate::management::{CallError, Management};
use serde::Deserialize;
use serde_json::json;

/// The provider the offers are of, the one Always On speaks of.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OptionsProvider {
    pub id: String,
    pub name: String,
}

/// One of the places the provider sells in: its own word for it, and the city it names.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProvisionLocation {
    pub id: String,
    pub city: String,
}

/// One server type in one place, as the engine words it: its label (the type's name and shape)
/// and its price, drawn as they are.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Offer {
    #[serde(rename = "type")]
    pub size: String,
    pub location: String,
    pub label: String,
    pub price: String,
}

/// A size and a location: what a create makes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Pick {
    #[serde(rename = "type")]
    pub size: String,
    pub location: String,
}

/// `home.provision.options`: what the provider in use sells now, read live.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Options {
    pub provider: OptionsProvider,
    /// In the order the engine recommends them.
    pub locations: Vec<ProvisionLocation>,
    /// Cheapest first.
    pub offers: Vec<Offer>,
    pub recommended: Option<Pick>,
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
    /// What the provider in use sells now; the provider is read live.
    pub fn provision_options(&self) -> Result<Options, CallError> {
        let answer = self.call("home.provision.options", json!({}))?;
        serde_json::from_value(answer).map_err(|e| {
            CallError::Protocol(format!(
                "home.provision.options answered an unexpected shape: {e}"
            ))
        })
    }
}
