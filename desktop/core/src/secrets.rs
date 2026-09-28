//! Where Fermix keeps secrets (engine 4ddea8b9): the login keyring, or one file
//! per secret in the Fermix folder. The file store is chosen, never fallen into,
//! so a save the keyring refuses is offered it once, and the `secrets` section
//! is where the choice is read and changed.

use crate::management::Refusal;
use crate::settings::SectionRows;

pub const SECTION: &str = "secrets";
pub const STORE_KEY: &str = "secret_store";
pub const KEYRING: &str = "keyring";
pub const FILE: &str = "file";

/// Whether the store itself refused the save: locked, absent or too slow.
pub fn store_refused(refusal: &Refusal) -> bool {
    refusal.code == "secret_store_failed"
}

/// What a refused `secret.set` says. When the store refused it, the daemon's own
/// message says only that nothing was stored, and its reason word says why.
pub fn refused_sentence(refusal: &Refusal) -> String {
    if !store_refused(refusal) {
        return refusal.sentence.clone();
    }
    let sentence = match refusal.reason.as_deref() {
        Some("locked") => {
            "Your keyring is locked and was not unlocked, so nothing was saved. \
             Signing in with a fingerprint or automatically leaves it locked."
        }
        Some("unavailable") => "No keyring answered, so nothing was saved.",
        Some("timeout") => "The keyring did not answer in time, so nothing was saved.",
        _ => return refusal.sentence.clone(),
    };
    sentence.to_owned()
}

/// The store the `secrets` section names, or `None` when it names neither.
pub fn configured_store(section: &SectionRows) -> Option<&'static str> {
    let row = section.rows.iter().find(|r| r.key == STORE_KEY)?;
    match row.value.as_str()? {
        KEYRING => Some(KEYRING),
        FILE => Some(FILE),
        _ => None,
    }
}
