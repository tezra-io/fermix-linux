//! The dialogs Settings opens.
//!
//! One task each, one default response, Cancel always present, Escape cancels,
//! and a secret entry that exists in exactly one of them.

pub mod consent;
pub mod model_picker;
pub mod oauth_client;
pub mod restart;
pub mod secret;
pub mod sign_in;
pub mod workspace;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::metrics;

/// A dialog built around a form, with its two ways out on the header bar.
///
/// The caller keeps the buttons because it decides what each one does and
/// when the writing one becomes available; the dialog is what it presents.
pub struct FormDialog {
    pub dialog: adw::Dialog,
    pub confirm: gtk::Button,
    pub cancel: gtk::Button,
}

/// Build a form dialog: `content` under `title`, Cancel and `confirm_label`.
///
/// `adw::AlertDialog` is kept for what it is for, a message and a choice of
/// replies. Anything with a field to fill in comes through here instead, so
/// it opens at a width the field can be read in rather than at the width of
/// a sentence.
///
/// # Panics
///
/// If `title` or `confirm_label` is blank: a dialog with no name, or a
/// response with no word on it, is a dialog nobody can answer.
pub fn form_dialog(
    title: &str,
    content: &impl IsA<gtk::Widget>,
    confirm_label: &str,
) -> FormDialog {
    assert!(!title.trim().is_empty(), "a form dialog needs a title");
    assert!(
        !confirm_label.trim().is_empty(),
        "the response that writes needs a word on it"
    );

    let cancel = gtk::Button::with_label(&copy::text(Key::ActionCancel));
    let confirm = gtk::Button::with_label(confirm_label);
    confirm.add_css_class("suggested-action");

    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&confirm);

    let clamped = gtk::Box::new(gtk::Orientation::Vertical, metrics::SPACE_HEADING);
    clamped.set_margin_top(metrics::SPACE_GUTTER);
    clamped.set_margin_bottom(metrics::SPACE_GUTTER);
    clamped.set_margin_start(metrics::SPACE_GUTTER);
    clamped.set_margin_end(metrics::SPACE_GUTTER);
    clamped.append(content.as_ref());

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&clamped)
        .build();

    let toolbar = adw::ToolbarView::builder().content(&scroller).build();
    toolbar.add_top_bar(&header);

    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(metrics::DIALOG_FORM_WIDTH)
        .child(&toolbar)
        .build();

    // The response strip used to own the default response. Without it, Enter
    // in a field would reach nothing, so the writing button takes that role.
    confirm.set_receives_default(true);
    dialog.set_default_widget(Some(&confirm));

    // `close` answers whether it closed: a dialog held open by something
    // that refused the close would otherwise strand the person on a form
    // whose Cancel appears to do nothing.
    let closing = dialog.clone();
    cancel.connect_clicked(move |_| {
        if !closing.close() {
            gtk::glib::g_warning!(
                "fermix-desktop",
                "cancel did not close the form dialog: something refused the close"
            );
        }
    });

    FormDialog {
        dialog,
        confirm,
        cancel,
    }
}

/// Close a dialog only if it is still on screen.
///
/// A write spawned from a dialog outlives it: the person can cancel while the
/// call is in flight, and the success arm then closes a dialog that is
/// already gone. The toolkit answers that with "Trying to close
/// AdwAlertDialog … that's not presented", which the owner's journal carried
/// twice. Measured in tests/widgets.rs: a second close complains, a close in
/// the same turn as its presentation does not.
///
/// Returns whether it closed, so a caller that expected to can say it did not.
pub fn close_if_open(dialog: &impl IsA<adw::Dialog>) -> bool {
    if !dialog.as_ref().is_visible() {
        return false;
    }
    dialog.as_ref().close()
}
