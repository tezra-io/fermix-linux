//! The Settings controller: reads sections when their pane opens, writes one
//! row at a time, and re-reads what a write may have moved (spec §3.3). A
//! refusal lands under the row it names; nothing here retries.

use crate::app::App;
use crate::channels::{section_id, switch_key};
use crate::descriptor::{Keep, SectionView};
use crate::dialogs::{confirm, secret_dialog, SecretPrompt};
use adw::prelude::*;
use fermix_client::management::CallError;
use fermix_client::settings::{matching_panes, pane, sections_for, Kind};
use gtk::glib;
use serde_json::{Map, Value};
use std::rc::Rc;

/// What a row shows when its write never reached a verdict.
const NOT_SAVED: &str = "Fermix did not answer, so this was not saved.";

impl App {
    /// Reads every section of `pane` not read yet. Sections already on screen
    /// stay while they are read again elsewhere; only a first read shows a spinner.
    pub async fn open_pane(&self, pane: String) {
        if !self.ensure_sections().await {
            return;
        }
        let unread: Vec<String> = {
            let data = self.settings_data.borrow();
            let sections = data.sections.as_deref().unwrap_or_default();
            sections_for(&pane, sections)
                .into_iter()
                .map(|s| s.id.clone())
                .filter(|id| !data.rows.contains_key(id) && !data.reading.contains(id))
                .collect()
        };
        for id in unread {
            self.read_section(&id).await;
        }
        // What the daemon reports about a helper is read as its pane opens (M38 §5.7).
        match pane.as_str() {
            "meetings" => self.detect_meetbot().await,
            "computer" => self.probe_computer().await,
            "integrations" => self.integrations.shown(),
            _ => {}
        }
    }

    async fn ensure_sections(&self) -> bool {
        if self.settings_data.borrow().sections.is_some() {
            return true;
        }
        match self.daemon.call(|m| m.settings_sections()).await {
            Ok(answer) => {
                self.settings_data.borrow_mut().sections = Some(answer.sections);
                self.render();
                true
            }
            Err(e) => {
                // The idle refresh notices a daemon that went away; this pane just stays empty.
                glib::g_warning!("fermix", "settings.sections failed: {e:?}");
                false
            }
        }
    }

    pub async fn read_section(&self, id: &str) {
        self.settings_data
            .borrow_mut()
            .reading
            .insert(id.to_owned());
        let section = id.to_owned();
        let answer = self.daemon.call(move |m| m.settings_get(&section)).await;
        let mut data = self.settings_data.borrow_mut();
        data.reading.remove(id);
        match answer {
            Ok(rows) => {
                data.rows.insert(id.to_owned(), rows);
            }
            Err(e) => glib::g_warning!("fermix", "settings.get {id} failed: {e:?}"),
        }
        drop(data);
        self.render();
    }

    /// Writes one row. On success the section is read again, since rows come and
    /// go with other values; on refusal the sentence shows under the row.
    pub async fn apply_setting(self: Rc<Self>, section: String, key: String, value: Value) {
        let mut values = Map::new();
        values.insert(key.clone(), value);
        let target = section.clone();
        let answer = self
            .daemon
            .call(move |m| m.settings_apply(&target, values))
            .await;
        let applied = match answer {
            Ok(applied) => applied,
            Err(e) => return self.write_refused(&section, &key, e).await,
        };
        self.settings_data
            .borrow_mut()
            .errors
            .remove(&(section.clone(), key));
        for sentence in &applied.side_effects {
            self.shell.toast(sentence);
        }
        self.read_section(&section).await;
        self.refresh().await;
    }

    async fn write_refused(&self, section: &str, key: &str, e: CallError) {
        glib::g_warning!("fermix", "a write to {section}/{key} failed: {e:?}");
        let sentence = match &e {
            CallError::Refused(r) if r.code == "config_unreadable" => {
                self.settings_data.borrow_mut().unreadable = Some(r.sentence.clone());
                r.sentence.clone()
            }
            CallError::Refused(r) => r.sentence.clone(),
            _ => NOT_SAVED.to_owned(),
        };
        self.settings_data
            .borrow_mut()
            .errors
            .insert((section.to_owned(), key.to_owned()), sentence);
        self.render();
        // An external change or a broken file shows as the banner, read from the daemon.
        self.refresh().await;
    }

    /// Asks for a secret in a dialog and stores it under the row's key.
    pub fn add_secret(self: Rc<Self>, section: String, key: String) {
        let label = self.row_label(&section, &key);
        let title = format!("Add {label}");
        let prompt = SecretPrompt {
            title: &title,
            description: "The value goes to Fermix's secret store and is never shown again.",
            entry_title: &label,
        };
        let app = self.clone();
        secret_dialog(&self.shell.window, prompt, move |value| {
            let (app, section, key) = (app.clone(), section.clone(), key.clone());
            async move { app.store_secret(section, key, value).await }
        });
    }

    async fn store_secret(
        &self,
        section: String,
        key: String,
        value: String,
    ) -> Result<(), String> {
        let id = key.clone();
        let answer = self.daemon.call(move |m| m.secret_set(&id, &value)).await;
        match answer {
            Ok(_) => {
                self.settings_data
                    .borrow_mut()
                    .errors
                    .remove(&(section, key.clone()));
                self.secret_changed(&key).await;
                Ok(())
            }
            Err(CallError::Refused(r)) => Err(r.sentence),
            Err(e) => {
                glib::g_warning!("fermix", "secret.set {key} failed: {e:?}");
                Err(NOT_SAVED.to_owned())
            }
        }
    }

    pub async fn remove_secret(self: Rc<Self>, section: String, key: String) {
        let label = self.row_label(&section, &key);
        let heading = format!("Remove {label}?");
        let body =
            "Fermix forgets it. Anything that needs it stops working until you add it again.";
        if !confirm(&self.shell.window, &heading, body, "Remove", true).await {
            return;
        }
        let id = key.clone();
        match self.daemon.call(move |m| m.secret_clear(&id)).await {
            Ok(_) => self.secret_changed(&key).await,
            Err(e) => self.write_refused(&section, &key, e).await,
        }
    }

    /// One secret can sit in several sections (spec G4): re-read each that shows it.
    async fn secret_changed(&self, key: &str) {
        let holders: Vec<String> = self
            .settings_data
            .borrow()
            .rows
            .values()
            .filter(|s| {
                s.rows
                    .iter()
                    .any(|r| r.kind == Kind::Secret && r.key == key)
            })
            .map(|s| s.id.clone())
            .collect();
        for id in holders {
            self.read_section(&id).await;
        }
        self.refresh().await;
    }

    fn row_label(&self, section: &str, key: &str) -> String {
        self.settings_data
            .borrow()
            .rows
            .get(section)
            .and_then(|s| s.rows.iter().find(|r| r.key == key))
            .map_or_else(|| key.to_owned(), |r| r.label.clone())
    }

    /// Takes in a settings file changed outside Fermix, then reads everything again.
    pub async fn reload_settings(self: Rc<Self>) {
        match self.daemon.call(|m| m.settings_reload()).await {
            Ok(_) => {
                self.forget_settings();
                self.refresh().await;
                if let Some(pane) = self.shell.visible_pane() {
                    self.open_pane(pane).await;
                }
            }
            Err(CallError::Refused(r)) => self.shell.toast(&r.sentence),
            Err(e) => {
                glib::g_warning!("fermix", "settings.reload failed: {e:?}");
                self.shell.toast(NOT_SAVED);
            }
        }
    }

    pub fn forget_settings(&self) {
        let mut data = self.settings_data.borrow_mut();
        data.sections = None;
        data.rows.clear();
        data.errors.clear();
        data.unreadable = None;
    }

    /// A restarted daemon may publish different rows: drop what the old one
    /// said, and read the pane on screen again.
    pub async fn follow_daemon_process(&self, pid: Option<String>) {
        if self.settings_data.borrow().pid == pid {
            return;
        }
        self.forget_settings();
        self.settings_data.borrow_mut().pid = pid;
        if let Some(pane) = self.shell.visible_pane() {
            self.open_pane(pane).await;
        }
    }

    /// Shows a page or pane; a pane reads its sections as it opens.
    pub fn show_page(self: &Rc<Self>, name: &str) {
        self.shell.show_page(name);
        if pane(name).is_none() {
            return;
        }
        // Settings reads the daemon's current state as it opens, so a banner is never stale.
        let (app, pane) = (self.clone(), name.to_owned());
        glib::spawn_future_local(async move {
            app.refresh().await;
            app.open_pane(pane).await;
        });
    }

    pub fn search_settings(&self, query: &str) {
        let data = self.settings_data.borrow();
        let sections = data.sections.as_deref().unwrap_or_default();
        let matches = matching_panes(query, sections, &data.rows);
        drop(data);
        self.settings.filter(matches);
    }

    /// Opens one daemon section in a dialog. The dialog's rows follow the
    /// section like any pane's and stop when the dialog closes.
    pub fn section_dialog(self: &Rc<Self>, title: &str, section: &str, keep: Keep, intro: &str) {
        let view = SectionView::keeping(section, None, keep);
        let page = adw::PreferencesPage::new();
        let lead = adw::PreferencesGroup::new();
        lead.set_description(Some(&glib::markup_escape_text(intro)));
        page.add(&lead);
        page.add(&view.group);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&page));
        let dialog = adw::Dialog::builder()
            .title(title)
            .content_width(560)
            .content_height(560)
            .child(&toolbar)
            .build();
        self.settings.dialog_views.borrow_mut().push(view.clone());
        let views = self.settings.dialog_views.clone();
        dialog.connect_closed(move |_| views.borrow_mut().retain(|v| !Rc::ptr_eq(v, &view)));
        self.render();
        dialog.present(Some(&self.shell.window));
        glib::spawn_future_local({
            let (app, section) = (self.clone(), section.to_owned());
            async move { app.read_section(&section).await }
        });
    }

    pub fn channel_setup(self: &Rc<Self>, channel: &str) {
        let section = section_id(channel);
        let data = self.settings_data.borrow();
        let title = data
            .sections
            .iter()
            .flatten()
            .find(|s| s.id == section)
            .map_or_else(|| channel.to_owned(), |s| s.title.clone());
        let switch = switch_key(&data, channel);
        drop(data);
        // The list row's switch owns the channel's on/off key; the dialog draws the rest.
        let keep: Keep = Box::new(move |key| Some(key) != switch.as_deref());
        let intro = "Each field saves when you press Enter or leave it. Turn the channel on \
                     with the switch beside it when you are done.";
        self.section_dialog(&title, &section, keep, intro);
    }

    pub fn provider_settings(self: &Rc<Self>, provider: &str) {
        let title = self.state.borrow().label(provider);
        let intro = "The model and how hard it thinks. Changes take effect when Fermix restarts.";
        let section = format!("providers.{provider}");
        self.section_dialog(&title, &section, Box::new(|_| true), intro);
    }
}
