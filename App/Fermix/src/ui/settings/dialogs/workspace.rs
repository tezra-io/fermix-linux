//! The workspace dialog.
//!
//! One hosted plugin binds to one workspace under one access profile. Both
//! lists are the daemon's, read off the live row rather than off the row this
//! dialog opened on: a discovery that finished while the dialog was open
//! republishes the workspaces, and the label a row shows afterwards is the
//! daemon's answer rather than the one this dialog sent.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::plugins::PluginsModel;
use crate::models::spawn;
use crate::ui::CaptionRow;

use super::{close_if_open, form_dialog};
use crate::ui::plain;

/// The dialog one workspace is chosen in.
pub struct WorkspaceDialog;

impl WorkspaceDialog {
    /// Open the dialog over one plugin, by name.
    pub fn present(plugins: Rc<PluginsModel>, name: &str, parent: &impl IsA<gtk::Widget>) {
        let Some(row) = plugins.row(name) else {
            return;
        };

        let group = adw::PreferencesGroup::new();

        let profiles: Vec<String> = row
            .access_profiles
            .iter()
            .map(|profile| profile.label.clone())
            .collect();
        let profile = plain(
            adw::ComboRow::builder()
                .title(copy::text(Key::WorkspaceAccessProfile))
                .model(&gtk::StringList::new(
                    &profiles.iter().map(String::as_str).collect::<Vec<&str>>(),
                ))
                .build(),
        );
        group.add(&profile);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("boxed-list");
        for workspace in &row.workspaces {
            list.append(&plain(
                adw::ActionRow::builder()
                    .title(workspace.label.as_str())
                    .subtitle(workspace.id.as_str())
                    .build(),
            ));
        }
        if let Some(first) = list.row_at_index(0) {
            list.select_row(Some(&first));
        }

        let notice = CaptionRow::new();
        let notice_group = crate::ui::caption_group(&notice);

        let column = crate::ui::column();
        column.append(&group);
        column.append(&list);
        column.append(&notice_group);

        let form = form_dialog(
            &copy::text(Key::WorkspaceDialogTitle),
            &column,
            &copy::text(Key::ActionContinue),
        );
        let dialog = form.dialog.clone();
        // Nothing to choose from is nothing to confirm.
        form.confirm.set_sensitive(!row.workspaces.is_empty());

        let name = name.to_string();
        let dialog_for_choose = dialog.clone();
        form.confirm.connect_clicked(move |_| {
            let dialog = dialog_for_choose.clone();

            let Some(selected) = list.selected_row().map(|row| row.index().max(0) as usize) else {
                return;
            };
            let Some(workspace) = row.workspaces.get(selected).cloned() else {
                return;
            };
            let Some(profile) = row
                .access_profiles
                .get(profile.selected() as usize)
                .map(|profile| profile.id.clone())
            else {
                return;
            };

            let plugins = Rc::clone(&plugins);
            let name = name.clone();
            let dialog = dialog.clone();
            let notice = notice.clone();

            spawn(async move {
                match plugins.select_workspace(&name, &profile, &workspace).await {
                    None => {
                        // Cancel may have got here first; see close_if_open.
                        close_if_open(&dialog);
                    }
                    Some(sentence) => notice.set(Some(&sentence.text)),
                }
            });
        });

        dialog.present(Some(parent.as_ref()));
    }
}
