//! The sign-in dialog.
//!
//! The browser is where a sign-in happens, so this dialog reports the step and
//! offers the address again. It is open exactly while a sign-in is in flight,
//! which is the one state no daemon field can report: the browser hop happens
//! outside the daemon, and the job is what says when it is over.
//!
//! The address is shown as copyable text whenever the browser did not open,
//! because a sign-in nobody can reach is a dead end otherwise. `auth.start`
//! returns it once, so it is held here for as long as this dialog is open and
//! never asked for again.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::jobs::{phase_word, JobRunner};
use crate::models::providers::{ProvidersModel, SignIn};
use crate::models::spawn;
use crate::session::DesktopSession;
use crate::ui::CaptionRow;

/// The dialog one sign-in runs under.
pub struct SignInDialog {
    dialog: adw::AlertDialog,
    phase: CaptionRow,
    address: gtk::Label,
    copy_button: gtk::Button,
    runner: Rc<JobRunner>,
    providers: Rc<ProvidersModel>,
    started: RefCell<Option<SignIn>>,
}

impl SignInDialog {
    /// Open the dialog over one sign-in that has already started.
    pub fn present(providers: Rc<ProvidersModel>, started: SignIn, parent: &impl IsA<gtk::Widget>) {
        let dialog = Rc::new(Self::build(Rc::clone(&providers), &started));
        dialog.started.replace(Some(started.clone()));
        dialog.connect();
        dialog.draw();
        dialog.dialog.present(Some(parent.as_ref()));

        // The browser is opened once, by the gesture that started the sign-in.
        // Where it did not open, the address is on screen to be copied.
        if let Some(url) = started.authorize_url.clone() {
            let dialog = Rc::clone(&dialog);
            let window = crate::ui::window_of(parent);
            spawn(async move {
                // The address on screen is the owner's way out, but it is not
                // an explanation: a browser that never opened looks identical
                // to one the owner was too slow to notice. The reason goes to
                // the journal, because the last time this failed it took a day
                // to find and the cause was a missing file the log never
                // mentioned.
                if let Err(error) = DesktopSession::open_url(window.as_ref(), &url).await {
                    glib::g_warning!(
                        "fermix-desktop",
                        "the browser was not opened for the sign-in: {error}"
                    );
                    dialog.show_address();
                }
            });
        }
    }

    fn build(providers: Rc<ProvidersModel>, started: &SignIn) -> Self {
        let phase = CaptionRow::new();
        let address = gtk::Label::builder()
            .label("")
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .visible(false)
            .build();
        address.add_css_class("monospace");

        let label = crate::ui::caption(&copy::text(Key::ProviderSignInUrlLabel));
        label.set_visible(false);

        let copy_button = gtk::Button::builder()
            .label(copy::text(Key::ProviderSignInCopyUrl))
            .halign(gtk::Align::Start)
            .visible(false)
            .build();

        let column = crate::ui::column();
        let group = adw::PreferencesGroup::new();
        group.add(phase.row());
        column.append(&group);
        column.append(&label);
        column.append(&address);
        column.append(&copy_button);

        let dialog = adw::AlertDialog::new(
            Some(&copy::text(Key::ProviderSignInDialogTitle)),
            Some(&copy::text(if started.imported {
                Key::ProviderImportBody
            } else {
                Key::ProviderSignInBody
            })),
        );
        dialog.set_extra_child(Some(&column));
        dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
        dialog.set_close_response(CANCEL);
        dialog.set_default_response(Some(CANCEL));

        // The label rides with the address: both appear together or not at all.
        address.connect_visible_notify(move |address| label.set_visible(address.is_visible()));

        Self {
            dialog,
            phase,
            address,
            copy_button,
            runner: providers.sign_in_job(),
            providers,
            started: RefCell::new(None),
        }
    }

    fn connect(self: &Rc<Self>) {
        {
            let dialog = Rc::clone(self);
            self.runner.observe(move || dialog.draw());
        }
        {
            let dialog = Rc::clone(self);
            self.copy_button.connect_clicked(move |button| {
                let text = dialog.address.label().to_string();
                if let Some(clipboard) = button.display().clipboard().into() {
                    let clipboard: gtk::gdk::Clipboard = clipboard;
                    clipboard.set_text(&text);
                }
            });
        }

        // Every way out of this dialog ends the sign-in: the job keeps running
        // where the daemon is still waiting on a browser, and the rows are read
        // again either way.
        let providers = Rc::clone(&self.providers);
        let runner = Rc::clone(&self.runner);
        self.dialog.connect_closed(move |_| {
            let providers = Rc::clone(&providers);
            let runner = Rc::clone(&runner);
            spawn(async move {
                runner.detach();
                providers.sign_in_finished().await;
            });
        });
    }

    /// Show the address, which is what a browser that did not open leaves.
    fn show_address(self: &Rc<Self>) {
        let Some(url) = self
            .started
            .borrow()
            .as_ref()
            .and_then(|started| started.authorize_url.clone())
        else {
            return;
        };

        self.address.set_label(&url);
        self.address.set_visible(true);
        self.copy_button.set_visible(true);
    }

    /// The step the job is on, and nothing more: the daemon owns the sentence
    /// for what happened, and the phase is the only thing that moves here.
    fn draw(self: &Rc<Self>) {
        let Some(job) = self.runner.job() else {
            self.phase.set(None);
            return;
        };

        if let Some(failure) = job.failure.as_ref() {
            self.phase.set(Some(&failure.sentence));
            return;
        }

        match job.phase.as_deref().and_then(phase_word) {
            Some(word) => self.phase.set(Some(&copy::text(word))),
            None => self.phase.set(None),
        }

        if self.runner.is_terminal() {
            self.dialog.close();
        }
    }
}

const CANCEL: &str = "cancel";
