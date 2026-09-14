//! The consent a plugin is installed under.
//!
//! Both sentences are the daemon's. `consent_sentence` names where the code
//! runs and is always present, and `remote_disclosure` names what leaves this
//! computer where anything does. Neither is composed here and neither has a
//! default: a hosted plugin rendering the local-process line is the one defect
//! those fields exist to prevent.

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::plugins::IntegrationRow;

/// The dialog a switch-on asks through.
pub struct ConsentDialog;

impl ConsentDialog {
    /// Ask for one plugin's published consent.
    ///
    /// `accepted` runs only on the one response that accepts, so a dialog
    /// dismissed any other way installs nothing.
    pub fn present(
        row: &IntegrationRow,
        parent: &impl IsA<gtk::Widget>,
        accepted: impl Fn() + 'static,
        declined: impl Fn() + 'static,
    ) {
        let dialog = adw::AlertDialog::new(
            Some(&copy::text(Key::IntegrationsConsentTitle)),
            Some(&row.consent),
        );

        if let Some(disclosure) = row.disclosure.as_ref() {
            dialog.set_extra_child(Some(&expander(disclosure)));
        }

        dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
        // The daemon's own word for what this row leads with, where it
        // published one. A verb this door invented would be a word the person
        // is agreeing to that nothing on the wire said.
        let accept_label = row
            .primary_verb
            .clone()
            .unwrap_or_else(|| copy::text(Key::ActionContinue));
        dialog.add_response(ACCEPT, &accept_label);
        dialog.set_response_appearance(ACCEPT, adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some(ACCEPT));
        dialog.set_close_response(CANCEL);

        dialog.connect_response(None, move |_, response| {
            if response == ACCEPT {
                accepted();
            } else {
                declined();
            }
        });

        dialog.present(Some(parent.as_ref()));
    }
}

/// What leaves this computer, folded away until it is asked for.
fn expander(disclosure: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let expander = adw::ExpanderRow::builder()
        .title(copy::text(Key::IntegrationsWhatLeaves))
        .expanded(false)
        .build();

    let row = adw::ActionRow::builder()
        .title(disclosure)
        .title_lines(0)
        .activatable(false)
        .build();
    row.add_css_class("caption");
    expander.add_row(&row);
    group.add(&expander);
    group
}

const CANCEL: &str = "cancel";
const ACCEPT: &str = "accept";
