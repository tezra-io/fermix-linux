//! The Voice pane.
//!
//! The daemon's own rows, under two statements the platform owes a person
//! before they touch a control that opens a microphone.
//!
//! The first is where voice is used from, because this door configures a
//! capability it does not itself provide. The second is the microphone
//! statement, which is above every control that enables capture rather than
//! beneath it, so it is read before it is relevant rather than after. Both are
//! the copy catalogue's, rendered verbatim: the Permissions ledger and this
//! pane render the same row rather than a variant of it.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::SettingsPane;
use crate::models::ledger::{PermissionLedger, Right};
use crate::models::SettingsModel;
use crate::ui::settings::descriptor_form::DescriptorForm;

/// The Voice pane.
pub struct VoicePane {
    root: gtk::Widget,
    form: Rc<DescriptorForm>,
}

impl VoicePane {
    /// Build the pane over the one settings model and the one ledger.
    pub fn new(settings: Rc<SettingsModel>, ledger: Rc<PermissionLedger>) -> Rc<Self> {
        let form = DescriptorForm::restricted(
            settings,
            SettingsPane::Voice,
            vec![REALTIME.to_string(), TRANSCRIPTION.to_string()],
            None,
        );

        let statements = adw::PreferencesGroup::new();
        statements.add(&statement(Key::VoiceCompanionStatement));
        statements.add(&statement(Key::VoiceMicrophoneStatement));

        // The microphone row of the one ledger, which is what makes this pane
        // and Permissions incapable of disagreeing about it.
        let microphone = ledger.row(Right::Microphone);
        statements.add(
            &adw::ActionRow::builder()
                .title(copy::text(microphone.right.title()))
                .subtitle(copy::text(microphone.right.principal()))
                .subtitle_lines(0)
                .activatable(false)
                .build(),
        );

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&statements);
        column.append(&form.widget());

        Rc::new(Self {
            root: crate::ui::scrolled(&crate::ui::clamp(&column)).upcast(),
            form,
        })
    }

    /// The widget the pane stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// Read the two sections this pane draws.
    pub fn load(self: &Rc<Self>) {
        self.form.load();
    }

    /// The form, for the pane list's search index.
    pub fn form(&self) -> Rc<DescriptorForm> {
        Rc::clone(&self.form)
    }
}

/// One statement, in the shape every statement the platform owes takes.
fn statement(key: Key) -> adw::ActionRow {
    crate::ui::statement_row(&copy::text(key))
}

/// The two sections the daemon publishes under this pane.
const REALTIME: &str = "realtime";
const TRANSCRIPTION: &str = "transcription";
