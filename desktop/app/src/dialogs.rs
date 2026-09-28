//! The two dialogs, paste a secret and confirm an action, and the pages a form
//! is drawn on. A form is a page so that one opened from the setup assistant
//! becomes its next page, not a second popup over it; alone, it sits in a
//! dialog of its own.

use adw::prelude::*;
use gtk::glib;
use std::future::Future;
use std::rc::{Rc, Weak};

pub const SECRET_WIDTH: i32 = 420;

pub struct SecretPrompt<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub entry_title: &'a str,
}

/// Asks for one secret in a dialog of its own.
pub fn secret_dialog<F, Fut>(parent: &impl IsA<gtk::Widget>, prompt: SecretPrompt<'_>, submit: F)
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    form_dialog(&secret_page(prompt, submit), SECRET_WIDTH, None).present(Some(parent));
}

/// A dialog around one form page, which `leave` closes.
pub fn form_dialog(page: &adw::NavigationPage, width: i32, height: Option<i32>) -> adw::Dialog {
    let nav = adw::NavigationView::new();
    nav.add(page);
    let dialog = adw::Dialog::builder()
        .title(page.title())
        .content_width(width)
        .child(&nav)
        .build();
    if let Some(height) = height {
        dialog.set_content_height(height);
    }
    dialog
}

/// One secret, as a page: Cancel and Add over the entry. `submit` stores it
/// and answers with the daemon's sentence when it did not land; the page then
/// stays, with that sentence under the entry. It leaves once the value is
/// stored or on Cancel, and the entry is cleared whenever the page is hidden.
pub fn secret_page<F, Fut>(prompt: SecretPrompt<'_>, submit: F) -> adw::NavigationPage
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let (content, entry, error) = secret_body(&prompt);
    let (header, cancel, add) = secret_header();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::builder()
        .title(prompt.title)
        .child(&toolbar)
        .build();
    let widgets = Rc::new(SecretWidgets {
        entry,
        cancel,
        add,
        error,
    });
    wire_secret_page(&page, &widgets, Rc::new(submit));
    keep_with(&page, widgets);
    page
}

/// The entry, what it is for, and the line a refusal is shown on, held to a
/// readable width on a page as wide as the assistant's.
fn secret_body(prompt: &SecretPrompt<'_>) -> (adw::Clamp, adw::PasswordEntryRow, gtk::Label) {
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
    let clamp = adw::Clamp::builder()
        .maximum_size(SECRET_WIDTH + 60)
        .child(&content)
        .build();
    (clamp, entry, error)
}

/// Cancel and Add in the header, where GNOME puts a dialog's answer. Cancel
/// stands for Back too, so a page pushed onto the assistant has one way back.
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
        .show_back_button(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&add);
    (header, cancel, add)
}

struct SecretWidgets {
    entry: adw::PasswordEntryRow,
    cancel: gtk::Button,
    add: gtk::Button,
    error: gtk::Label,
}

fn wire_secret_page<F, Fut>(page: &adw::NavigationPage, w: &Rc<SecretWidgets>, submit: Rc<F>)
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let add = w.add.downgrade();
    w.entry.connect_changed(move |entry| {
        if let Some(add) = add.upgrade() {
            add.set_sensitive(!entry.text().trim().is_empty());
        }
    });
    w.cancel.connect_clicked(leave);
    // The entry takes the focus as it comes on screen, in a dialog of its own
    // or pushed onto the assistant; on the next idle turn, once it can.
    w.entry.connect_map(|entry| {
        let entry = entry.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(entry) = entry.upgrade() {
                entry.grab_focus();
            }
        });
    });
    let hidden = w.entry.downgrade();
    page.connect_hidden(move |_| {
        if let Some(entry) = hidden.upgrade() {
            entry.set_text("");
        }
    });
    let (on_click, on_enter) = (Rc::downgrade(w), Rc::downgrade(w));
    let submit_enter = submit.clone();
    w.add
        .connect_clicked(move |_| send_secret(&on_click, submit.clone()));
    w.entry
        .connect_entry_activated(move |_| send_secret(&on_enter, submit_enter.clone()));
}

/// Sends what was typed, once: Add is insensitive until the answer comes, and
/// Enter in the entry goes through the same check. Cancel waits too, so a
/// refusal always has its page to be shown on.
fn send_secret<F, Fut>(w: &Weak<SecretWidgets>, submit: Rc<F>)
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let Some(w) = w.upgrade() else {
        return;
    };
    let value = w.entry.text().trim().to_owned();
    if value.is_empty() || !w.add.is_sensitive() {
        return;
    }
    w.add.set_sensitive(false);
    w.cancel.set_sensitive(false);
    w.entry.set_sensitive(false);
    w.error.set_visible(false);
    glib::spawn_future_local(async move {
        let outcome = submit(value).await;
        w.entry.set_sensitive(true);
        w.cancel.set_sensitive(true);
        match outcome {
            Ok(()) => leave(&w.add),
            Err(sentence) => {
                w.error.set_text(&sentence);
                w.error.set_visible(true);
                w.add.set_sensitive(true);
            }
        }
    });
}

/// The page owns its form. The form's own widgets hold it only weakly, so
/// when the page goes, its handlers go, and the form with them.
pub fn keep_with<T: 'static>(page: &adw::NavigationPage, form: Rc<T>) {
    page.connect_destroy(move |_| {
        let _ = &form;
    });
}

/// Goes back from the page holding `widget`, or closes the dialog it is the
/// only page of. A page already left stays left.
pub fn leave(widget: &impl IsA<gtk::Widget>) {
    let page = widget
        .ancestor(adw::NavigationPage::static_type())
        .and_downcast::<adw::NavigationPage>();
    let nav = widget
        .ancestor(adw::NavigationView::static_type())
        .and_downcast::<adw::NavigationView>();
    let (Some(page), Some(nav)) = (page, nav) else {
        return;
    };
    if nav.visible_page().as_ref() != Some(&page) || nav.pop() {
        return;
    }
    let dialog = nav
        .ancestor(adw::Dialog::static_type())
        .and_downcast::<adw::Dialog>();
    if let Some(dialog) = dialog {
        dialog.close();
    }
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
