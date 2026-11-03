//! What the cue dialogs share (M60 cues §4.4–§4.6): a dialog of pages the integration detail's
//! size, which lets go of every page as it closes; a page's header bar; the lines under a form;
//! a page of one line; a button that names its row; and a redraw after the signal that asked.

use crate::cues_rows::note_row;
use adw::prelude::*;
use fermix_client::cues::words;
use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;

const DIALOG_WIDTH: i32 = 480;
const DIALOG_HEIGHT: i32 = 620;

/// A dialog of pages. Each page goes in by `first_page` or a push, so the view removes it once
/// it is left; closing the dialog leaves them all, and with them each form, what it held, a
/// token shown once and a copy still on the clipboard.
pub fn page_dialog(title: &str) -> (adw::Dialog, adw::NavigationView) {
    let nav = adw::NavigationView::new();
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(DIALOG_WIDTH)
        .content_height(DIALOG_HEIGHT)
        .child(&nav)
        .build();
    dialog.connect_closed(|dialog| {
        if let Some(nav) = dialog.child().and_downcast::<adw::NavigationView>() {
            nav.replace(&[]);
        }
    });
    (dialog, nav)
}

/// Shows `page` as the dialog's only page, in place of any before it. A page added rather than
/// pushed would stay in the view after it was left, and keep alive all it holds.
pub fn first_page(nav: &adw::NavigationView, page: &adw::NavigationPage) {
    nav.replace(std::slice::from_ref(page));
}

/// A page of the dialog: its title in a header bar, an action at its end, and its content.
pub fn nav_page(
    title: &str,
    content: &impl IsA<gtk::Widget>,
    action: Option<&gtk::Button>,
) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    if let Some(action) = action {
        header.pack_end(action);
    }
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(content));
    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
}

/// A line under a form or its pickers, hidden while it has nothing to say.
pub fn hidden_line() -> adw::ActionRow {
    let row = note_row("", false, true);
    row.set_visible(false);
    row.add_css_class("setting-refused");
    row
}

pub fn show_line(row: &adw::ActionRow, text: Option<&str>) {
    row.set_title(&glib::markup_escape_text(text.unwrap_or("")));
    row.set_visible(text.is_some());
}

/// A page saying it is reading.
pub fn reading() -> adw::PreferencesPage {
    alone(&note_row(words::READING, true, false))
}

/// A page of one row.
pub fn alone(row: &impl IsA<gtk::Widget>) -> adw::PreferencesPage {
    let group = adw::PreferencesGroup::new();
    group.add(row);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    page
}

/// A button read to a screen reader with the row it acts on: "Edit · gh-hooks".
pub fn named_button(label: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build();
    let spoken = words::middot(&[label.to_owned(), name.to_owned()]);
    button.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    button
}

/// A redraw asked from inside a row's own signal, run once that signal is over: a row must not
/// rebuild itself while it speaks. Asks made before it runs make one redraw.
#[derive(Default)]
pub struct Soon {
    queued: Rc<Cell<bool>>,
}

impl Soon {
    pub fn redraw<T: 'static>(&self, owner: &Rc<T>, redraw: fn(&T)) {
        if self.queued.replace(true) {
            return;
        }
        let (queued, owner) = (self.queued.clone(), owner.clone());
        glib::idle_add_local_once(move || {
            queued.set(false);
            redraw(&owner);
        });
    }
}
