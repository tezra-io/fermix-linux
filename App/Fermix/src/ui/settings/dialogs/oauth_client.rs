//! The sign-in client dialog.
//!
//! One client: the identifier, the port the browser comes back to, and the
//! region where the daemon published regions to choose from. The secret is not
//! a parameter of that call and never passes through here: it goes through the
//! one secret dialog, under the id the contract spells for a client.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::PluginOAuthClient;
use crate::management::vocabulary::oauth_client_id;
use crate::models::plugins::PluginsModel;
use crate::models::{spawn, SettingsModel};
use crate::ui::CaptionRow;

use super::{close_if_open, form_dialog};

use super::secret::SecretDialog;
use crate::ui::plain;

/// The dialog one sign-in client is registered through.
pub struct OAuthClientDialog;

impl OAuthClientDialog {
    /// Open the dialog over one client row.
    pub fn present(
        plugins: Rc<PluginsModel>,
        settings: Rc<SettingsModel>,
        client: PluginOAuthClient,
        parent: &impl IsA<gtk::Widget>,
    ) {
        let identifier = plain(
            adw::EntryRow::builder()
                .title(copy::text(Key::OAuthClientId))
                .text(client.client_id.clone().unwrap_or_default())
                .build(),
        );

        let port = plain(adw::SpinRow::with_range(MINIMUM_PORT, MAXIMUM_PORT, 1.0));
        port.set_title(&copy::text(Key::OAuthClientRedirectPort));
        port.set_value(f64::from(client.redirect_port.unwrap_or(0)));

        let group = adw::PreferencesGroup::new();
        group.add(&identifier);
        group.add(&port);

        // A region is offered exactly where the daemon published regions: the
        // call refuses one for a provider that serves a single region.
        let regions = (!client.regions.is_empty()).then(|| {
            let words: Vec<&str> = client
                .regions
                .iter()
                .map(|region| region.label.as_str())
                .collect();
            let combo = plain(
                adw::ComboRow::builder()
                    .title(copy::text(Key::OAuthClientRegion))
                    .model(&gtk::StringList::new(&words))
                    .build(),
            );
            if let Some(at) = client
                .regions
                .iter()
                .position(|region| Some(&region.id) == client.region.as_ref())
            {
                combo.set_selected(at as u32);
            }
            group.add(&combo);
            combo
        });

        let secret = secret_row(&client, settings, parent);
        group.add(&secret);

        let notice = CaptionRow::new();
        group.add(notice.row());

        let form = form_dialog(
            &copy::text(Key::OAuthClientDialogTitle),
            &group,
            &copy::text(Key::ActionContinue),
        );
        let dialog = form.dialog.clone();

        // Blank is never sent: the one response that writes is unavailable
        // until the identifier says something.
        {
            let save = form.confirm.clone();
            save.set_sensitive(!identifier.text().trim().is_empty());
            identifier.connect_changed(move |identifier| {
                save.set_sensitive(!identifier.text().trim().is_empty());
            });
        }

        let typed = identifier.clone();
        let dialog_for_save = dialog.clone();
        form.confirm.connect_clicked(move |_| {
            let dialog = dialog_for_save.clone();

            let client_id = typed.text().trim().to_string();
            if client_id.is_empty() {
                return;
            }

            let region = regions.as_ref().and_then(|combo| {
                client
                    .regions
                    .get(combo.selected() as usize)
                    .map(|region| region.id.clone())
            });

            let plugins = Rc::clone(&plugins);
            let provider = client.provider.clone();
            let port = port.value().round().max(0.0) as u32;
            let dialog = dialog.clone();
            let notice = notice.clone();

            spawn(async move {
                match plugins
                    .set_oauth_client(&provider, &client_id, port, region)
                    .await
                {
                    None => {
                        // The person may have cancelled while this was in
                        // flight; closing what is already gone is what the
                        // owner's journal complained about.
                        close_if_open(&dialog);
                    }
                    // The daemon's own sentence, beside the fields it refused.
                    Some(sentence) => notice.set(Some(&sentence.text)),
                }
            });
        });

        dialog.present(Some(parent.as_ref()));
        identifier.grab_focus();
    }
}

/// The client's own secret, which goes through the one dialog that takes one.
fn secret_row(
    client: &PluginOAuthClient,
    settings: Rc<SettingsModel>,
    parent: &impl IsA<gtk::Widget>,
) -> adw::ActionRow {
    let row = plain(
        adw::ActionRow::builder()
            .title(copy::text(Key::OAuthClientSecret))
            .subtitle(if client.secret_present {
                copy::text(Key::SecretStored)
            } else {
                String::new()
            })
            .activatable(false)
            .build(),
    );

    let button = gtk::Button::builder()
        .label(copy::text(if client.secret_present {
            Key::SecretReplace
        } else {
            Key::SecretAdd
        }))
        .valign(gtk::Align::Center)
        .build();

    let id = oauth_client_id(&client.provider);
    let present = client.secret_present;
    let parent = parent.as_ref().clone();
    button.connect_clicked(move |_| {
        SecretDialog::present(Rc::clone(&settings), CLIENT_SECTION, &id, present, &parent);
    });

    row.add_suffix(&button);
    row
}

/// A client secret belongs to no settings section: the id is the whole address,
/// and this is the name the refusal is filed under while the dialog is open.
const CLIENT_SECTION: &str = "";

/// The ports a loopback redirect can come back on.
const MINIMUM_PORT: f64 = 0.0;
const MAXIMUM_PORT: f64 = 65_535.0;
