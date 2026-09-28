//! Storing a secret, with the one question the terminal setup also asks: when
//! the keyring refuses a save, may Fermix keep its secrets in a file instead?
//! The file store is chosen, never fallen into, so nothing switches without a yes.

use crate::daemon::Daemon;
use crate::dialogs::confirm;
use adw::prelude::*;
use fermix_client::management::CallError;
use fermix_client::model::SecretSetResult;
use fermix_client::secrets::{configured_store, store_refused, FILE, KEYRING, SECTION, STORE_KEY};
use gtk::glib;
use serde_json::{json, Map};

const HEADING: &str = "Keep secrets in a file?";
const BODY: &str = "Your keyring did not take this key. Fermix can keep each secret in a \
    private file in your Fermix home instead: no password, readable only by your account, and \
    not encrypted. Keys already saved stay where they are. You can change this later in \
    Settings, under Secrets.";
const VERB: &str = "Keep in a File";

/// `secret.set`. When the keyring refuses it, offers the file store once; on a
/// yes, switches the store and tries the same save one more time.
pub async fn set_secret(
    daemon: &Daemon,
    parent: &impl IsA<gtk::Widget>,
    id: String,
    value: String,
) -> Result<SecretSetResult, CallError> {
    assert!(!id.is_empty(), "a secret needs an id");
    let (first_id, first_value) = (id.clone(), value.clone());
    let answer = daemon
        .call(move |m| m.secret_set(&first_id, &first_value))
        .await;
    let Err(CallError::Refused(refusal)) = &answer else {
        return answer;
    };
    if !store_refused(refusal) || !keyring_in_use(daemon).await {
        return answer;
    }
    if !confirm(parent, HEADING, BODY, VERB, false).await {
        return answer;
    }
    switch_to_file(daemon).await?;
    daemon.call(move |m| m.secret_set(&id, &value)).await
}

/// Whether the keyring is the configured store. A daemon without the `secrets`
/// section cannot switch, so it is not offered one.
async fn keyring_in_use(daemon: &Daemon) -> bool {
    match daemon.call(|m| m.settings_get(SECTION)).await {
        Ok(section) => configured_store(&section) == Some(KEYRING),
        Err(e) => {
            glib::g_warning!("fermix", "where secrets are kept could not be read: {e:?}");
            false
        }
    }
}

async fn switch_to_file(daemon: &Daemon) -> Result<(), CallError> {
    let mut values = Map::new();
    values.insert(STORE_KEY.into(), json!(FILE));
    daemon
        .call(move |m| m.settings_apply(SECTION, values))
        .await
        .map(|_| ())
}
