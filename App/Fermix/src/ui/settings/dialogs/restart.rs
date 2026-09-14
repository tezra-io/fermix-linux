//! The one restart confirmation.
//!
//! Every route to a restart opens this dialog: the Settings toolbar action, the
//! Home attention row, the Doctor remediation, the primary menu and the
//! accelerator. It carries the daemon's own reasons, what it knows about the
//! work in progress, and three responses. A refusal stays in the dialog.
//!
//! Restarting is a suggested action rather than a destructive one. The toolkit
//! reserves the destructive appearance for an action that loses something a
//! person cannot get back, and a restart keeps every setting, every credential
//! and every file: what it interrupts is named in the dialog by the daemon,
//! which is the honest size of it. Red here would have been the strongest word
//! the deck has, spent on the most ordinary maintenance action in the product.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::{spawn, SettingsModel};
use crate::service::types::Alignment;

/// The dialog that restarts Fermix.
pub struct RestartDialog;

impl RestartDialog {
    /// Ask, then restart.
    pub fn present(settings: Rc<SettingsModel>, parent: &impl IsA<gtk::Widget>) {
        let finishing_update = alignment(&settings) == Some(Alignment::PendingRestart);

        let dialog = adw::AlertDialog::new(
            Some(&copy::text(if finishing_update {
                Key::FinishUpdatingDialogTitle
            } else {
                Key::RestartDialogTitle
            })),
            Some(&body(&settings, finishing_update)),
        );

        let refusal = super::super::super::caption("");
        refusal.set_visible(false);
        dialog.set_extra_child(Some(&refusal));

        dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
        dialog.add_response(IDLE, &copy::text(Key::RestartWhenIdle));
        dialog.add_response(NOW, &copy::text(Key::RestartNow));
        dialog.set_response_appearance(NOW, adw::ResponseAppearance::Suggested);
        dialog.set_response_appearance(IDLE, adw::ResponseAppearance::Default);
        dialog.set_default_response(Some(CANCEL));
        dialog.set_close_response(CANCEL);

        dialog.connect_response(None, move |dialog, response| {
            let when_idle = match response {
                NOW => false,
                IDLE => true,
                _ => return,
            };

            let settings = Rc::clone(&settings);
            let dialog = dialog.clone();
            let refusal = refusal.clone();
            spawn(async move {
                match settings.restart(when_idle).await {
                    Ok(_) => {
                        dialog.close();
                    }
                    Err(sentence) => {
                        // The refusal stays where the decision was made, and
                        // the response that was refused stops being offered.
                        refusal.set_label(&sentence.text);
                        refusal.set_visible(true);
                        if when_idle {
                            dialog.set_response_enabled(IDLE, false);
                        }
                    }
                }
            });
        });

        dialog.present(Some(parent.as_ref()));
    }
}

/// What the dialog says: the daemon's own reasons, and what is in progress.
///
/// An unknown number of conversations is unknown, never zero.
fn body(settings: &SettingsModel, finishing_update: bool) -> String {
    let state = settings.state();

    let opening = copy::text(if finishing_update {
        Key::FinishUpdatingDialogBody
    } else {
        Key::RestartDialogBody
    });

    let reasons: Vec<String> = state
        .restart
        .reasons
        .iter()
        .map(|reason| reason.sentence.clone())
        .collect();

    let conversations = match state.overview.as_ref() {
        Some(overview) => copy::fill(
            Key::RestartConversationsKnown,
            &[(
                "{count}",
                &overview.agents.main.active_conversations.to_string(),
            )],
        ),
        None => copy::text(Key::RestartConversationsUnknown),
    };

    let mut lines = vec![opening];
    lines.extend(reasons);
    lines.push(conversations);
    lines.join("\n\n")
}

fn alignment(settings: &SettingsModel) -> Option<Alignment> {
    settings
        .state()
        .service
        .as_ref()
        .map(|status| status.alignment)
}

/// The three responses, by the ids the dialog answers with.
///
/// Public because the appearance each one carries is a decision rather than a
/// detail, and the widget gate reads them back: `tests/widgets.rs` asserts that
/// Restart now is suggested, that Restart when idle is plain, and that Cancel
/// is what Enter answers. They are this dialog's own strings and nothing else
/// routes on them.
pub const CANCEL: &str = "cancel";
pub const IDLE: &str = "idle";
pub const NOW: &str = "now";
