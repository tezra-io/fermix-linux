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
use crate::models::secret_store::StoreRefusal;
use crate::models::settings_model::Sentence;
use crate::models::{spawn, SettingsModel};

use super::{close_if_open, form_dialog, FormDialog};

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
        let form = dialog_for(present, &entry);
        let dialog = form.dialog.clone();

        {
            let store = form.confirm.clone();
            entry.connect_changed(move |entry| {
                // Blank is never sent: the one response that writes is
                // unavailable until there is something to write.
                store.set_sensitive(!entry.text().is_empty());
            });
        }

        // The value is cleared on every way out, taken or abandoned. Cancel
        // and Escape both land here, so no path leaves a typed key sitting in
        // a widget after the dialog is gone.
        let emptied = entry.clone();
        dialog.connect_closed(move |_| emptied.set_text(""));

        let section = section.to_string();
        let key = key.to_string();
        let stored: Rc<dyn Fn()> = Rc::new(stored);
        let owned = entry.clone();
        let anchor = parent.as_ref().clone();
        let closing = dialog.clone();
        form.confirm.connect_clicked(move |_| {
            let secret = owned.text().to_string();
            owned.set_text("");
            close_if_open(&closing);

            if secret.is_empty() {
                return;
            }

            let settings = Rc::clone(&settings);
            let section = section.clone();
            let key = key.clone();
            let anchor = anchor.clone();
            let stored = Rc::clone(&stored);
            spawn(async move {
                let retry = Retry {
                    settings: Rc::clone(&settings),
                    section: section.clone(),
                    key: key.clone(),
                    secret: secret.clone(),
                    stored: Rc::clone(&stored),
                };
                match settings.set_secret(&section, &key, secret).await {
                    Ok(()) => stored(),
                    Err(sentence) => present_store_failure(&sentence, retry, &anchor),
                }
            });
        });

        dialog.present(Some(parent.as_ref()));
        entry.grab_focus();
    }
}

/// Everything needed to send the same secret again.
///
/// The value is held only while one of these dialogs is open, because every
/// offer on them is a second attempt at the write the owner already asked for
/// and this application cannot read a value back out of any store. It goes out
/// of scope with the dialog.
#[derive(Clone)]
struct Retry {
    settings: Rc<SettingsModel>,
    section: String,
    key: String,
    secret: String,
    stored: Rc<dyn Fn()>,
}

/// The dialog for one store refusal, with only the actions that can work on it.
///
/// Built apart from what the buttons do so the shape can be tested: which
/// actions a refusal offers is the whole of what the owner's complaint was
/// about. The old dialog offered one next action for three situations, and it
/// was wrong for all three.
pub fn store_dialog(refusal: StoreRefusal, gave_up: bool, sentence: &Sentence) -> adw::AlertDialog {
    match refusal {
        StoreRefusal::KeyringLocked => locked_dialog(gave_up),
        StoreRefusal::NoKeyring => absent_dialog(),
        // Nothing is locked, so there is nothing to unlock and no reason to
        // reach for the file store: the helper may simply answer next time.
        // The words are the daemon's own, as with every other refusal.
        StoreRefusal::HelperDidNotAnswer => {
            let dialog = adw::AlertDialog::new(None, Some(&sentence.text));
            dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
            dialog.add_response(RETRY, &copy::text(Key::ActionTryAgain));
            dialog.set_default_response(Some(RETRY));
            dialog.set_close_response(CANCEL);
            dialog
        }
    }
}

/// A keyring is here and locked, which is the one case with a way through.
///
/// Both actions stay on it after the wait is given up, because an owner who
/// walked away from the prompt is in exactly the situation it opened on.
fn locked_dialog(gave_up: bool) -> adw::AlertDialog {
    let body = if gave_up {
        Key::SecretKeyringUnlockGaveUp
    } else {
        Key::SecretKeyringLockedBody
    };

    let dialog = adw::AlertDialog::new(
        Some(&copy::text(Key::SecretKeyringLockedTitle)),
        Some(&copy::text(body)),
    );
    // Each action carries its own words directly beneath it. Stacked above
    // the buttons they read as conditions of the whole dialog, and "the key
    // will be saved in a file" is then a thing that happens whichever button
    // is pressed, which is the ambiguity this screen exists to remove.
    dialog.set_extra_child(Some(&offers(&[
        Offer {
            response: UNLOCK,
            label: Key::ActionUnlockKeyring,
            caption: Key::SecretKeyringUnlockHint,
            suggested: true,
        },
        Offer {
            response: STORE_ON_THIS_COMPUTER,
            label: Key::ActionStoreOnThisComputer,
            caption: Key::SecretStoreFileTradeoff,
            suggested: false,
        },
    ])));
    dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
    dialog.set_close_response(CANCEL);
    dialog
}

/// No keyring at all, so the file store is the whole of what can be offered.
fn absent_dialog() -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(
        Some(&copy::text(Key::SecretStoreUnavailableTitle)),
        Some(&copy::text(Key::SecretStoreUnavailableBody)),
    );
    // Under the button, not above it: the press stores the key in one click,
    // so what that costs belongs with the thing that does it. Suggested
    // because it is the only thing this dialog can do, not because a private
    // file is better than a keyring; the caption under it says which.
    dialog.set_extra_child(Some(&offers(&[Offer {
        response: STORE_ON_THIS_COMPUTER,
        label: Key::ActionStoreOnThisComputer,
        caption: Key::SecretStoreFileTradeoff,
        suggested: true,
    }])));
    dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
    dialog.set_close_response(CANCEL);
    dialog
}

/// One action a store refusal offers, with the words that belong to it.
struct Offer {
    /// The response this button reports, which is also its widget name.
    response: &'static str,
    label: Key,
    caption: Key,
    suggested: bool,
}

/// The offered actions, each with its caption directly beneath it.
///
/// These are buttons in the dialog's extra child rather than dialog
/// responses, because a response area cannot put words between its buttons
/// and a caption that does not touch its own action is ambiguous however true
/// it is. Cancel stays a response: it has nothing to explain.
fn offers(offers: &[Offer]) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    for offer in offers {
        let group = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let button = gtk::Button::with_label(&copy::text(offer.label));
        button.set_widget_name(offer.response);
        // No shape class: an offer and Cancel are the same kind of control and
        // sit in one column, so they take the dialog's own response-button
        // shape rather than reading as two different sorts of thing.
        if offer.suggested {
            button.add_css_class("suggested-action");
        }
        group.append(&button);
        group.append(&crate::ui::caption(&copy::text(offer.caption)));
        column.append(&group);
    }
    column
}

/// Move every file-stored secret back into the keyring, waiting if asked.
///
/// The same wait and the same giving-up as a save, because it is the same
/// unlock: an owner who meets one account of a locked keyring should not meet
/// a second one worded differently. Nothing is typed and nothing is lost, so
/// a refusal simply leaves every value where it was.
pub fn migrate_to_keyring(settings: Rc<SettingsModel>, parent: &gtk::Widget) {
    let waiting = adw::AlertDialog::new(None, Some(&copy::text(Key::SecretKeyringUnlockWaiting)));
    waiting.set_extra_child(Some(&spinner()));
    waiting.present(Some(parent));

    let parent = parent.clone();
    spawn(async move {
        let outcome = settings.migrate_to_keyring(true).await;
        close_if_open(&waiting);

        let Err(sentence) = outcome else {
            return;
        };
        // Every refusal is reported, not only the one this screen knows how to
        // name. An engine without the verb at all answers something this build
        // has no words for, and a button that spins and then silently does
        // nothing is worse than one that says what went wrong: the owner is
        // left unable to tell a refusal from a success.
        let told = match StoreRefusal::of(&sentence) {
            // The keyring is still not taking values. The same words a refused
            // save would use, minus the offers: there is no secret in hand to
            // put anywhere else, so what happened is the whole of it.
            Some(StoreRefusal::KeyringLocked) => adw::AlertDialog::new(
                Some(&copy::text(Key::SecretKeyringLockedTitle)),
                Some(&copy::text(Key::SecretKeyringUnlockGaveUp)),
            ),
            // Anything else in the daemon's own words, as every other refusal
            // in this application is shown.
            _ => adw::AlertDialog::new(None, Some(&sentence.text)),
        };
        told.add_response(CANCEL, &copy::text(Key::ActionCancel));
        told.set_close_response(CANCEL);
        told.present(Some(&parent));
    });
}

/// What a refused migration says, whatever the refusal was.
///
/// It returns a dialog rather than an option on purpose: there is no refusal
/// this screen may answer with silence. A button that spins and then does
/// nothing leaves the owner unable to tell a refusal from a success, and the
/// case that produced that was an engine without the verb at all, answering
/// something this build has no words for.
pub fn migration_refusal(sentence: &Sentence) -> adw::AlertDialog {
    let dialog = match StoreRefusal::of(sentence) {
        // The keyring is still not taking values. The same words a refused
        // save would use, minus the offers: there is no secret in hand to put
        // anywhere else, so what happened is the whole of it.
        Some(StoreRefusal::KeyringLocked) => adw::AlertDialog::new(
            Some(&copy::text(Key::SecretKeyringLockedTitle)),
            Some(&copy::text(Key::SecretKeyringUnlockGaveUp)),
        ),
        // Anything else in the daemon's own words, as every other refusal in
        // this application is shown.
        _ => adw::AlertDialog::new(None, Some(&sentence.text)),
    };
    dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
    dialog.set_close_response(CANCEL);
    dialog
}

/// What a refused store says, whatever the refusal was.
///
/// It returns a dialog rather than an option on purpose, for the reason the
/// migration one does: there is no refusal a button press may be answered with
/// silence. The owner pressed "Store on this computer", and an engine that
/// predates the `store` parameter refuses that as invalid parameters — not a
/// store refusal at all, so the code that knew only store refusals drew
/// nothing and the press vanished. A dialog that quotes the daemon is worth
/// little; a press that does nothing at all is worth less, because the owner
/// cannot tell it from success.
pub fn store_refusal_dialog(sentence: &Sentence, gave_up: bool) -> adw::AlertDialog {
    match StoreRefusal::of(sentence) {
        // One this build understands, with the offers that can work on it.
        Some(refusal) => store_dialog(refusal, gave_up, sentence),
        // Anything else in the daemon's own words, with no offers: an action
        // for a state nobody here understands would be a guess.
        None => {
            let dialog = adw::AlertDialog::new(None, Some(&sentence.text));
            dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
            dialog.set_close_response(CANCEL);
            dialog
        }
    }
}

/// Every offered action's button, by the response it stands for.
///
/// The builder names each button after its response, so the wiring here and
/// the test that pins the pairing read the same thing.
fn offered_buttons(dialog: &adw::AlertDialog) -> Vec<(String, gtk::Button)> {
    let Some(extra) = dialog.extra_child() else {
        return Vec::new();
    };

    let mut found = Vec::new();
    let mut group = extra.first_child();
    while let Some(row) = group {
        if let Some(button) = row
            .first_child()
            .and_then(|child| child.downcast::<gtk::Button>().ok())
        {
            found.push((button.widget_name().to_string(), button));
        }
        group = row.next_sibling();
    }
    found
}

/// The store refused the write, so say which of the three refusals it was.
///
/// Every other refusal is shown under the row that asked for the write, which
/// the model has already recorded.
fn present_store_failure(sentence: &Sentence, retry: Retry, parent: &gtk::Widget) {
    present_store_refusal(sentence, retry, parent, false);
}

/// The same, knowing whether the wait for an unlock has already been given up.
fn present_store_refusal(sentence: &Sentence, retry: Retry, parent: &gtk::Widget, gave_up: bool) {
    let dialog = store_refusal_dialog(sentence, gave_up);

    // The offered actions are buttons in the extra child, so each one closes
    // the dialog itself. Cancel and Try again are dialog responses and arrive
    // below.
    for (response, button) in offered_buttons(&dialog) {
        let dialog = dialog.clone();
        let parent = parent.clone();
        let retry = retry.clone();
        button.connect_clicked(move |_| {
            dialog.close();
            match response.as_str() {
                UNLOCK => wait_for_unlock(retry.clone(), &parent),
                STORE_ON_THIS_COMPUTER => store_on_this_computer(retry.clone(), &parent),
                _ => {}
            }
        });
    }

    let anchor = parent.clone();
    dialog.connect_response(None, move |_, response| {
        let parent = anchor.clone();
        let retry = retry.clone();
        if response == RETRY {
            send_again(retry, &parent);
        }
    });

    dialog.present(Some(parent));
}

/// Ask the engine to wait for the owner to finish unlocking, and say so.
///
/// The wait is the owner's, not a machine's, so there is nothing to count
/// down: the engine answers when they finish or when its cap passes. An
/// unlock that succeeds closes this; one that does not brings the same dialog
/// back with both its actions.
fn wait_for_unlock(retry: Retry, parent: &gtk::Widget) {
    let waiting = adw::AlertDialog::new(None, Some(&copy::text(Key::SecretKeyringUnlockWaiting)));
    waiting.set_extra_child(Some(&spinner()));
    waiting.present(Some(parent));

    let parent = parent.clone();
    spawn(async move {
        let outcome = retry
            .settings
            .retry_secret_after_unlock(&retry.section, &retry.key, retry.secret.clone())
            .await;
        close_if_open(&waiting);

        match outcome {
            Ok(()) => (retry.stored)(),
            Err(sentence) => present_store_refusal(&sentence, retry, &parent, true),
        }
    });
}

/// Store it in the private file, because the owner pressed the button saying so.
fn store_on_this_computer(retry: Retry, parent: &gtk::Widget) {
    let parent = parent.clone();
    spawn(async move {
        let outcome = retry
            .settings
            .store_secret_on_this_computer(&retry.section, &retry.key, retry.secret.clone())
            .await;
        match outcome {
            Ok(()) => (retry.stored)(),
            Err(sentence) => present_store_failure(&sentence, retry, &parent),
        }
    });
}

/// Send the same value again, for a helper that did not answer.
fn send_again(retry: Retry, parent: &gtk::Widget) {
    let parent = parent.clone();
    spawn(async move {
        let outcome = retry
            .settings
            .set_secret(&retry.section, &retry.key, retry.secret.clone())
            .await;
        match outcome {
            Ok(()) => (retry.stored)(),
            Err(sentence) => present_store_failure(&sentence, retry, &parent),
        }
    });
}

/// The one indeterminate spinner in this file.
fn spinner() -> gtk::Spinner {
    let spinner = gtk::Spinner::new();
    spinner.start();
    spinner
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
fn dialog_for(present: bool, entry: &gtk::PasswordEntry) -> FormDialog {
    let form = form_dialog(
        &copy::text(if present {
            Key::SecretDialogTitleReplace
        } else {
            Key::SecretDialogTitleAdd
        }),
        entry,
        &copy::text(Key::SecretDialogConfirm),
    );
    form.confirm.set_sensitive(false);
    form
}

/// The way out without choosing either store.
pub const CANCEL: &str = "cancel";
/// Wait for the owner to unlock the keyring, then write again.
pub const UNLOCK: &str = "unlock";
/// Write to the private file store instead, which is the consent itself.
pub const STORE_ON_THIS_COMPUTER: &str = "store-on-this-computer";
/// Send the same value again, for a helper that did not answer.
pub const RETRY: &str = "retry";
