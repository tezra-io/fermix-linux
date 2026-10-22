//! The update dialog: a stock dialog under the app's icon that shows an
//! update, runs its install from Install Update to the window coming back,
//! and says what went wrong with a way to do it by hand. It draws whatever
//! `update::view::dialog_view` says; its buttons are window actions the
//! controller (`update_flow.rs`) answers. Text is shown as written: the notes'
//! only markup is inline code, and everything else is escaped or plain.

use adw::prelude::*;
use fermix_client::update::state::Manual;
use fermix_client::update::view::{manual_intro, DialogAction, DialogButton, DialogView, Span};
use gtk::{gio, glib};
use std::cell::RefCell;

const APP_ICON: &str = "io.tezra.Fermix";
const WIDTH: i32 = 460;
/// The notes scroll past this height, so the buttons stay on screen.
const NOTES_HEIGHT: i32 = 220;
/// Long lines wrap here, so every state of the dialog is as wide as the last.
const LINE_CHARS: i32 = 46;

pub struct UpdateDialog {
    pub dialog: adw::Dialog,
    header: adw::HeaderBar,
    toasts: adw::ToastOverlay,
    title: gtk::Label,
    subtitle: gtk::Label,
    notice: gtk::Label,
    notes: gtk::Box,
    notes_scroller: gtk::ScrolledWindow,
    progress: gtk::ProgressBar,
    progress_text: gtk::Label,
    spinner: adw::Spinner,
    failure: gtk::Label,
    manual: gtk::Box,
    buttons: gtk::Box,
    shown: RefCell<Option<DialogView>>,
}

fn label(classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .justify(gtk::Justification::Center)
        .max_width_chars(LINE_CHARS)
        .css_classes(classes.to_vec())
        .build()
}

impl UpdateDialog {
    pub fn new() -> UpdateDialog {
        let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let notes_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(NOTES_HEIGHT)
            .propagate_natural_height(true)
            .child(&notes)
            .build();
        let toasts = adw::ToastOverlay::new();
        let dialog = adw::Dialog::builder()
            .content_width(WIDTH)
            .title("Software Update")
            .child(&toasts)
            .build();
        let parts = UpdateDialog {
            dialog,
            header: adw::HeaderBar::builder().show_title(false).build(),
            toasts,
            title: label(&["title-2"]),
            subtitle: label(&["dimmed"]),
            notice: label(&[]),
            notes,
            notes_scroller,
            progress: gtk::ProgressBar::new(),
            progress_text: label(&["dimmed", "caption", "numeric"]),
            spinner: adw::Spinner::builder().height_request(32).build(),
            failure: label(&["error"]),
            manual: gtk::Box::new(gtk::Orientation::Vertical, 6),
            buttons: gtk::Box::builder()
                .spacing(12)
                .halign(gtk::Align::Center)
                .margin_top(12)
                .build(),
            shown: RefCell::default(),
        };
        parts.lay_out();
        parts
    }

    /// Every part in one column under the app's icon, below a header that
    /// holds only the close button.
    fn lay_out(&self) {
        let icon = gtk::Image::builder()
            .icon_name(APP_ICON)
            .pixel_size(96)
            .build();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(24)
            .margin_end(24)
            .margin_bottom(24)
            .build();
        for widget in [
            icon.upcast_ref::<gtk::Widget>(),
            self.title.upcast_ref(),
            self.subtitle.upcast_ref(),
            self.notice.upcast_ref(),
            self.notes_scroller.upcast_ref(),
            self.spinner.upcast_ref(),
            self.progress.upcast_ref(),
            self.progress_text.upcast_ref(),
            self.failure.upcast_ref(),
            self.manual.upcast_ref(),
            self.buttons.upcast_ref(),
        ] {
            body.append(widget);
        }
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&self.header);
        toolbar.set_content(Some(&body));
        self.toasts.set_child(Some(&toolbar));
    }

    /// Draws `view`. Only what changed is redrawn, so the buttons and the
    /// command never move under the pointer between progress updates.
    pub fn show(&self, view: &DialogView) {
        let before = self.shown.borrow_mut().replace(view.clone());
        if before.as_ref() == Some(view) {
            return;
        }
        self.title.set_text(&view.title);
        show_text(&self.subtitle, view.subtitle.as_deref());
        show_text(&self.notice, view.notice.as_deref());
        show_text(&self.failure, view.failure.as_deref());
        self.show_progress(view);
        // PackageKit that cannot say how far it is shows the spinner instead of a bar.
        let unmeasured = view.progress.as_ref().is_some_and(|p| p.fraction.is_none());
        self.spinner.set_visible(view.spinner || unmeasured);
        // Past the point of no return there is no way to close it.
        self.dialog.set_can_close(view.closable);
        self.header.set_show_end_title_buttons(view.closable);
        if before.as_ref().map(|b| &b.notes) != Some(&view.notes) {
            self.show_notes(&view.notes);
        }
        if before.as_ref().map(|b| &b.manual) != Some(&view.manual) {
            self.show_manual(view.manual.as_ref());
        }
        if before.as_ref().map(|b| &b.buttons) != Some(&view.buttons) {
            self.show_buttons(&view.buttons);
        }
    }

    fn show_progress(&self, view: &DialogView) {
        let Some(progress) = &view.progress else {
            self.progress.set_visible(false);
            self.progress_text.set_visible(false);
            return;
        };
        self.progress.set_visible(progress.fraction.is_some());
        self.progress.set_fraction(progress.fraction.unwrap_or(0.0));
        show_text(&self.progress_text, Some(&progress.text));
    }

    fn show_notes(&self, notes: &[Vec<Span>]) {
        while let Some(child) = self.notes.first_child() {
            self.notes.remove(&child);
        }
        for note in notes {
            // The bullet sits apart, so a wrapped note's lines align under its text.
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let bullet = gtk::Label::builder()
                .label("•")
                .valign(gtk::Align::Start)
                .build();
            let text = gtk::Label::builder()
                .use_markup(true)
                .label(note_markup(note))
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .max_width_chars(LINE_CHARS)
                .hexpand(true)
                .xalign(0.0)
                .build();
            line.append(&bullet);
            line.append(&text);
            self.notes.append(&line);
        }
        self.notes_scroller.set_visible(!notes.is_empty());
    }

    fn show_manual(&self, manual: Option<&Manual>) {
        while let Some(child) = self.manual.first_child() {
            self.manual.remove(&child);
        }
        let Some(manual) = manual else {
            self.manual.set_visible(false);
            return;
        };
        self.manual.append(&label_text(manual_intro(manual), &[]));
        match manual {
            Manual::Command { command, file } => {
                self.manual.append(&label_text(command, &["monospace"]));
                self.manual.append(&self.command_buttons(command, file));
            }
            Manual::Link { url } => {
                let link = gtk::LinkButton::with_label(url, "Download the Package");
                link.set_halign(gtk::Align::Center);
                self.manual.append(&link);
            }
            Manual::Reopen => {}
        }
        self.manual.set_visible(true);
    }

    /// Copy puts the command on the clipboard; Show in Files opens the
    /// folder that holds the package.
    fn command_buttons(&self, command: &str, file: &std::path::Path) -> gtk::Box {
        let row = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        let copy = gtk::Button::with_label("Copy");
        let (text, toasts) = (command.to_owned(), self.toasts.clone());
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&text);
            toasts.add_toast(crate::shell::plain_toast("Copied"));
        });
        let open = gtk::Button::with_label("Show in Files");
        let package = gio::File::for_path(file);
        open.connect_clicked(move |button| show_in_files(button, &package));
        row.append(&copy);
        row.append(&open);
        row
    }

    fn show_buttons(&self, buttons: &[DialogButton]) {
        while let Some(child) = self.buttons.first_child() {
            self.buttons.remove(&child);
        }
        for spec in buttons {
            let button = self.button(spec);
            self.buttons.append(&button);
            // Focus starts on the suggested button, never on a label whose
            // text it would select.
            if spec.suggested {
                self.dialog.set_focus(Some(&button));
            }
        }
        self.buttons.set_visible(!buttons.is_empty());
    }

    /// A button that fires its window action, or only closes the dialog.
    fn button(&self, spec: &DialogButton) -> gtk::Button {
        let button = gtk::Button::with_label(spec.label);
        if spec.suggested {
            button.add_css_class("suggested-action");
        }
        if let Some(action) = window_action(spec.action) {
            button.set_action_name(Some(action));
            return button;
        }
        let dialog = self.dialog.clone();
        button.connect_clicked(move |_| {
            dialog.close();
        });
        button
    }
}

fn show_text(label: &gtk::Label, text: Option<&str>) {
    label.set_text(text.unwrap_or(""));
    label.set_visible(text.is_some());
}

/// A label that shows `text` as written.
fn label_text(text: &str, classes: &[&str]) -> gtk::Label {
    let shown = label(classes);
    shown.set_selectable(classes.contains(&"monospace"));
    shown.set_text(text);
    shown
}

/// A note as Pango markup: its text escaped, its inline code in `<tt>`.
pub fn note_markup(note: &[Span]) -> String {
    note.iter()
        .map(|span| match span {
            Span::Text(text) => glib::markup_escape_text(text).to_string(),
            Span::Code(code) => format!("<tt>{}</tt>", glib::markup_escape_text(code)),
        })
        .collect()
}

/// The window action a button fires; `None` for those that only close the dialog.
pub fn window_action(action: DialogAction) -> Option<&'static str> {
    match action {
        DialogAction::Install => Some("win.updates-install"),
        DialogAction::Skip => Some("win.updates-skip"),
        DialogAction::Cancel => Some("win.updates-cancel"),
        DialogAction::TryAgain => Some("win.updates-try-again"),
        DialogAction::CheckAgain => Some("win.updates-check"),
        DialogAction::Restart => Some("win.updates-restart"),
        DialogAction::Later | DialogAction::Close => None,
    }
}

/// Opens the folder that holds `package`, with the package selected.
fn show_in_files(button: &gtk::Button, package: &gio::File) {
    let window = button.root().and_downcast::<gtk::Window>();
    let launcher = gtk::FileLauncher::new(Some(package));
    launcher.open_containing_folder(window.as_ref(), gio::Cancellable::NONE, |opened| {
        if let Err(e) = opened {
            glib::g_warning!("fermix", "the package's folder did not open: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_s_text_is_escaped_and_only_its_code_is_markup() {
        let note = [
            Span::Text("Run <b>this</b> & ".into()),
            Span::Code("fermix <status>".into()),
            Span::Text(".".into()),
        ];
        assert_eq!(
            note_markup(&note),
            "Run &lt;b&gt;this&lt;/b&gt; &amp; <tt>fermix &lt;status&gt;</tt>."
        );
    }

    #[test]
    fn later_and_close_only_close_the_dialog() {
        assert_eq!(window_action(DialogAction::Later), None);
        assert_eq!(window_action(DialogAction::Close), None);
        assert_eq!(
            window_action(DialogAction::Install),
            Some("win.updates-install")
        );
    }
}
