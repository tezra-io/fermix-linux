//! The consent a plugin is installed under.
//!
//! Both sentences are the daemon's. `consent_sentence` names where the code
//! runs and is always present, and `remote_disclosure` names what leaves this
//! computer where anything does. Neither is composed here and neither has a
//! default: a hosted plugin rendering the local-process line is the one defect
//! those fields exist to prevent.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::metrics;
use crate::models::plugins::IntegrationRow;

use super::form_dialog;
use crate::ui::plain;

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
        // The consent sentence is what is being agreed to, so it is the body
        // of the dialog rather than a subtitle squeezed above a button strip.
        let body = gtk::Box::new(gtk::Orientation::Vertical, metrics::SPACE_HEADING);
        let sentence = gtk::Label::builder()
            .label(&row.consent)
            .wrap(true)
            .xalign(0.0)
            .build();
        body.append(&sentence);
        if let Some(disclosure) = row.disclosure.as_ref() {
            body.append(&expander(disclosure));
        }

        // The daemon's own word for what this row leads with, where it
        // published one. A verb this door invented would be a word the person
        // is agreeing to that nothing on the wire said.
        let accept_label = row
            .primary_verb
            .clone()
            .unwrap_or_else(|| copy::text(Key::ActionContinue));

        let form = form_dialog(
            &copy::text(Key::IntegrationsConsentTitle),
            &body,
            &accept_label,
        );

        // Exactly one of the two runs, whichever way the dialog is left:
        // accepting closes the dialog, and every other way out, Cancel and
        // Escape alike, reaches the close handler having accepted nothing.
        let taken = Rc::new(Cell::new(false));
        let accepting = Rc::clone(&taken);
        let closing = form.dialog.clone();
        form.confirm.connect_clicked(move |_| {
            accepting.set(true);
            accepted();
            if !closing.close() {
                glib::g_warning!(
                    "fermix-desktop",
                    "consent was given and its dialog stayed open"
                );
            }
        });
        form.dialog.connect_closed(move |_| {
            if !taken.get() {
                declined();
            }
        });

        form.dialog.present(Some(parent.as_ref()));
    }
}

/// What leaves this computer, folded away until it is asked for.
fn expander(disclosure: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let expander = plain(
        adw::ExpanderRow::builder()
            .title(copy::text(Key::IntegrationsWhatLeaves))
            .expanded(false)
            .build(),
    );

    let row = plain(
        adw::ActionRow::builder()
            .title(disclosure)
            .title_lines(0)
            .activatable(false)
            .build(),
    );
    row.add_css_class("caption");
    expander.add_row(&row);
    group.add(&expander);
    group
}
