//! Settings → Updates: this version and the engine's, what the last check
//! found and when, Check Now, and the switch for automatic checks. The app's
//! own pane: it reads nothing from the daemon, so it shows while Fermix is
//! down. Its words are core's (`update::view::settings_view`).

use adw::prelude::*;
use fermix_client::update::view::{RowAction, SettingsView, AUTOMATIC, AUTOMATIC_DETAIL};
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;

pub struct UpdatesPane {
    pub page: adw::PreferencesPage,
    version: adw::ActionRow,
    status: adw::ActionRow,
    spinner: adw::Spinner,
    button: gtk::Button,
    automatic: adw::SwitchRow,
    shown: RefCell<Option<SettingsView>>,
}

/// A row that shows its text as written, markup off before any is set.
fn plain_row() -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row
}

impl UpdatesPane {
    pub fn new() -> UpdatesPane {
        let version = plain_row();
        version.set_title("This version");
        version.add_css_class("property");
        version.set_subtitle_selectable(true);
        let status = plain_row();
        let spinner = adw::Spinner::new();
        let button = gtk::Button::builder().valign(gtk::Align::Center).build();
        status.add_suffix(&spinner);
        status.add_suffix(&button);
        let automatic = adw::SwitchRow::new();
        automatic.set_use_markup(false);
        automatic.set_title(AUTOMATIC);
        automatic.set_subtitle(AUTOMATIC_DETAIL);
        automatic.connect_active_notify(|row| {
            let wanted = row.is_active().to_variant();
            if let Err(e) = row.activate_action("win.updates-automatic", Some(&wanted)) {
                glib::g_warning!("fermix", "win.updates-automatic could not be sent: {e}");
            }
        });
        let group = adw::PreferencesGroup::new();
        group.add(&version);
        group.add(&status);
        group.add(&automatic);
        let page = adw::PreferencesPage::new();
        page.add(&group);
        UpdatesPane {
            page,
            version,
            status,
            spinner,
            button,
            automatic,
            shown: RefCell::default(),
        }
    }

    /// Redraws only when what the pane says changed, so a refresh never moves
    /// a button under the pointer.
    pub fn render(&self, view: &SettingsView) {
        if self.shown.borrow().as_ref() == Some(view) {
            return;
        }
        self.version.set_subtitle(&view.version);
        self.status.set_title(&view.status);
        self.status
            .set_subtitle(view.detail.as_deref().unwrap_or(""));
        self.spinner.set_visible(view.checking);
        match view.button {
            Some((label, action)) => {
                self.button.set_label(label);
                self.button.set_action_name(Some(row_action(action)));
                self.button.set_visible(true);
            }
            None => self.button.set_visible(false),
        }
        self.automatic.set_sensitive(view.automatic_sensitive);
        self.automatic.set_active(view.automatic);
        *self.shown.borrow_mut() = Some(view.clone());
    }
}

/// The window action a row's button fires. Show and Restart both open the
/// dialog, which shows the update or the restart that finishes one.
pub fn row_action(action: RowAction) -> &'static str {
    match action {
        RowAction::CheckNow => "win.updates-check",
        RowAction::Show | RowAction::Restart => "win.updates-open",
    }
}
