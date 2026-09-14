//! The one place a secret entry exists.
//!
//! A secret is never rendered, never read back and never sent blank. The entry
//! lives in this dialog and nowhere else, its contents are cleared when the
//! dialog closes, and the row that opened it only ever learns whether one is
//! stored.
//!
//! One refusal is answered here rather than under the row: a host with nowhere
//! to put a secret has not saved it, and that is a fact about the machine
//! rather than about the field. It gets the Linux moment the copy catalogue
//! owns, which says what happened, what is untouched and the one next action.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::vocabulary::Refusal;
use crate::models::settings_model::Sentence;
use crate::models::{spawn, SettingsModel};
use crate::session::DesktopSession;

/// The dialog that stores one secret.
pub struct SecretDialog;

impl SecretDialog {
    /// Ask for one secret and store it.
    ///
    /// `present` says whether one is already stored, which is the difference
    /// between adding and replacing, and nothing else.
    pub fn present(
        settings: Rc<SettingsModel>,
        section: &str,
        key: &str,
        present: bool,
        parent: &impl IsA<gtk::Widget>,
    ) {
        Self::present_then(settings, section, key, present, parent, || {});
    }

    /// The same dialog, with what to do once the daemon has taken the secret.
    ///
    /// One caller has something to do afterwards: the Setup assistant verifies
    /// a key it just stored with one metered call. Everywhere else stores it
    /// and stops, which is [`SecretDialog::present`] with nothing to run.
    pub fn present_then(
        settings: Rc<SettingsModel>,
        section: &str,
        key: &str,
        present: bool,
        parent: &impl IsA<gtk::Widget>,
        stored: impl Fn() + 'static,
    ) {
        let entry = password_entry();
        let dialog = dialog_for(present, &entry);

        {
            let dialog = dialog.clone();
            entry.connect_changed(move |entry| {
                // Blank is never sent: the one response that writes is
                // unavailable until there is something to write.
                dialog.set_response_enabled(STORE, !entry.text().is_empty());
            });
        }

        let section = section.to_string();
        let key = key.to_string();
        let stored: Rc<dyn Fn()> = Rc::new(stored);
        let owned = entry.clone();
        let anchor = parent.as_ref().clone();
        dialog.connect_response(None, move |_, response| {
            let secret = owned.text().to_string();
            // Cleared on every path out of the dialog, taken or cancelled.
            owned.set_text("");

            if response != STORE || secret.is_empty() {
                return;
            }

            let settings = Rc::clone(&settings);
            let section = section.clone();
            let key = key.clone();
            let anchor = anchor.clone();
            let stored = Rc::clone(&stored);
            spawn(async move {
                match settings.set_secret(&section, &key, secret).await {
                    Ok(()) => stored(),
                    Err(sentence) => present_store_failure(&sentence, &anchor),
                }
            });
        });

        dialog.present(Some(parent.as_ref()));
        entry.grab_focus();
    }
}

/// The host has nowhere to keep a secret, so nothing was saved.
///
/// The daemon's own code is what routes here, and the words are the
/// catalogue's: this moment has no macOS string and the design fixes its text.
/// Every other refusal is shown under the row that asked for the write, which
/// the model has already recorded.
fn present_store_failure(sentence: &Sentence, parent: &gtk::Widget) {
    let Some(code) = sentence.code.as_deref() else {
        return;
    };
    if Refusal::of_code(code) != Refusal::SecretStoreFailed {
        return;
    }

    let dialog = adw::AlertDialog::new(
        Some(&copy::text(Key::SecretStoreUnavailableTitle)),
        Some(&copy::text(Key::SecretStoreUnavailableBody)),
    );
    dialog.set_extra_child(Some(&crate::ui::caption(&copy::text(
        Key::SecretStoreUnavailableNextAction,
    ))));
    dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
    dialog.add_response(HELP, &copy::text(Key::ActionShowMeHow));
    dialog.set_default_response(Some(HELP));
    dialog.set_close_response(CANCEL);

    let window = crate::ui::window_of(parent);
    dialog.connect_response(None, move |_, response| {
        if response != HELP {
            return;
        }
        let window = window.clone();
        spawn(async move {
            let _ = DesktopSession::open_url(window.as_ref(), crate::app::WEBSITE).await;
        });
    });

    dialog.present(Some(parent));
}

/// The one secret entry in the product.
fn password_entry() -> gtk::PasswordEntry {
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .activates_default(true)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&copy::text(
        Key::SecretFieldLabel,
    ))]);
    entry
}

/// One task, one default response, Cancel always present, Escape cancels.
fn dialog_for(present: bool, entry: &gtk::PasswordEntry) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(
        Some(&copy::text(if present {
            Key::SecretDialogTitleReplace
        } else {
            Key::SecretDialogTitleAdd
        })),
        None,
    );

    dialog.set_extra_child(Some(entry));
    dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
    dialog.add_response(STORE, &copy::text(Key::SecretDialogConfirm));
    dialog.set_response_appearance(STORE, adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some(STORE));
    dialog.set_close_response(CANCEL);
    dialog.set_response_enabled(STORE, false);
    dialog
}

const CANCEL: &str = "cancel";
const STORE: &str = "store";
const HELP: &str = "help";
