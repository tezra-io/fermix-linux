//! The two dialogs: paste a secret, and confirm an action.

use adw::prelude::*;
use gtk::glib;
use std::future::Future;
use std::rc::Rc;

pub struct SecretPrompt<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub entry_title: &'a str,
}

/// Asks for one secret. `submit` stores it and answers with the daemon's sentence
/// when it did not land; the dialog then stays open with that sentence under the
/// entry. The entry is cleared whenever the dialog closes.
pub fn secret_dialog<F, Fut>(parent: &impl IsA<gtk::Widget>, prompt: SecretPrompt<'_>, submit: F)
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let (content, entry, error) = secret_body(&prompt);
    let (header, cancel, add) = secret_header();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title(prompt.title)
        .content_width(420)
        .child(&toolbar)
        .build();
    dialog.set_default_widget(Some(&add));

    let widgets = Rc::new(SecretWidgets {
        dialog: dialog.clone(),
        entry: entry.clone(),
        add,
        error,
    });
    wire_secret_dialog(&widgets, &cancel, Rc::new(submit));
    dialog.present(Some(parent));
    entry.grab_focus();
}

/// The entry, what it is for, and the line a refusal is shown on.
fn secret_body(prompt: &SecretPrompt<'_>) -> (gtk::Box, adw::PasswordEntryRow, gtk::Label) {
    let entry = adw::PasswordEntryRow::builder()
        .title(prompt.entry_title)
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(prompt.description)
        .build();
    group.add(&entry);
    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .visible(false)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&group);
    content.append(&error);
    (content, entry, error)
}

/// Cancel and Add in the header, where GNOME puts a dialog's answer.
fn secret_header() -> (adw::HeaderBar, gtk::Button, gtk::Button) {
    let cancel = gtk::Button::with_label("Cancel");
    let add = gtk::Button::builder()
        .label("Add")
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&add);
    (header, cancel, add)
}

struct SecretWidgets {
    dialog: adw::Dialog,
    entry: adw::PasswordEntryRow,
    add: gtk::Button,
    error: gtk::Label,
}

fn wire_secret_dialog<F, Fut>(w: &Rc<SecretWidgets>, cancel: &gtk::Button, submit: Rc<F>)
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let add = w.add.clone();
    w.entry
        .connect_changed(move |entry| add.set_sensitive(!entry.text().trim().is_empty()));
    let dialog = w.dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog.close();
    });
    let entry = w.entry.clone();
    w.dialog.connect_closed(move |_| entry.set_text(""));

    let (on_click, on_enter) = (w.clone(), w.clone());
    let (submit_click, submit_enter) = (submit.clone(), submit);
    w.add
        .connect_clicked(move |_| send_secret(&on_click, submit_click.clone()));
    w.entry
        .connect_entry_activated(move |_| send_secret(&on_enter, submit_enter.clone()));
}

fn send_secret<F, Fut>(w: &Rc<SecretWidgets>, submit: Rc<F>)
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let value = w.entry.text().trim().to_owned();
    if value.is_empty() {
        return;
    }
    w.add.set_sensitive(false);
    w.entry.set_sensitive(false);
    w.error.set_visible(false);
    let w = w.clone();
    glib::spawn_future_local(async move {
        let outcome = submit(value).await;
        w.entry.set_sensitive(true);
        match outcome {
            Ok(()) => {
                w.dialog.close();
            }
            Err(sentence) => {
                w.error.set_text(&sentence);
                w.error.set_visible(true);
                w.add.set_sensitive(true);
            }
        }
    });
}

/// A yes/no question. A destructive verb is never the default response.
pub async fn confirm(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    verb: &str,
    destructive: bool,
) -> bool {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_responses(&[("cancel", "Cancel"), ("ok", verb)]);
    let appearance = if destructive {
        adw::ResponseAppearance::Destructive
    } else {
        adw::ResponseAppearance::Suggested
    };
    dialog.set_response_appearance("ok", appearance);
    dialog.set_default_response(Some(if destructive { "cancel" } else { "ok" }));
    dialog.set_close_response("cancel");
    dialog.choose_future(Some(parent)).await == "ok"
}
