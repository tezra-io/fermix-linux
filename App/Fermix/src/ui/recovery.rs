//! Recovery.
//!
//! The cause first, in somebody else's words, then the offline evidence, then
//! the three things there are to do: try the same owned transaction again,
//! write the bundle out, or read the journal by hand. It never calls the
//! socket.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::recovery::RecoveryModel;
use crate::models::{spawn, Change, SettingsModel};

use super::{caption, identifier_label};

/// The Recovery surface.
pub struct RecoveryPage {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    recovery: Rc<RecoveryModel>,
    status: adw::StatusPage,
    evidence: adw::ExpanderRow,
    evidence_rows: RefCell<Vec<adw::ActionRow>>,
    evidence_group: adw::PreferencesGroup,
    retry: gtk::Button,
    export: gtk::Button,
    refusal: gtk::Label,
}

impl RecoveryPage {
    /// Build Recovery over the one settings model.
    pub fn new(settings: Rc<SettingsModel>, recovery: Rc<RecoveryModel>) -> Rc<Self> {
        let evidence = adw::ExpanderRow::builder()
            .title(copy::text(Key::RecoveryEvidence))
            .expanded(false)
            .build();
        let evidence_group = adw::PreferencesGroup::new();
        evidence_group.add(&evidence);

        let retry = action_button(Key::RecoveryRetry, true);
        let export = action_button(Key::RecoveryExportDiagnostics, false);
        let journal = action_button(Key::RecoveryShowJournalCommand, false);

        // The three wrap rather than hold the window open: two of these words
        // are phrases, and a row of them that cannot shrink is 601 px of
        // minimum width against the 424 the narrowest window has to give.
        let buttons = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(3)
            .min_children_per_line(1)
            .column_spacing(crate::metrics::SPACE_HEADING as u32)
            .row_spacing(crate::metrics::SPACE_HEADING as u32)
            .homogeneous(false)
            .halign(gtk::Align::Center)
            .build();
        buttons.append(&retry);
        buttons.append(&export);
        buttons.append(&journal);

        let refusal = caption("");
        refusal.set_visible(false);
        refusal.set_halign(gtk::Align::Center);

        let column = super::column();
        column.append(&evidence_group);
        column.append(&buttons);
        column.append(&refusal);

        let status = adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .child(&column)
            .build();

        let page = Rc::new(Self {
            root: super::scrolled(&super::clamp(&status)).upcast(),
            settings,
            recovery,
            status,
            evidence,
            evidence_rows: RefCell::new(Vec::new()),
            evidence_group,
            retry,
            export,
            refusal,
        });

        page.connect(&journal);
        page.draw();
        page
    }

    /// The widget to put in the window.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    fn connect(self: &Rc<Self>, journal: &gtk::Button) {
        {
            let page = Rc::downgrade(self);
            self.recovery.observe(move || {
                if let Some(page) = page.upgrade() {
                    page.draw();
                }
            });
        }

        {
            let page = Rc::downgrade(self);
            self.settings.observe(move |change| {
                if !matches!(change, Change::Setup | Change::Service) {
                    return;
                }
                if let Some(page) = page.upgrade() {
                    page.draw();
                }
            });
        }

        {
            let page = Rc::clone(self);
            self.retry.connect_clicked(move |button| {
                button.set_sensitive(false);
                let page = Rc::clone(&page);
                let button = button.clone();
                spawn(async move {
                    let outcome = page.recovery.retry().await;
                    button.set_sensitive(true);
                    if let Err(refusal) = outcome {
                        page.refusal.set_label(&refusal.text);
                        page.refusal.set_visible(true);
                    }
                });
            });
        }

        {
            let page = Rc::clone(self);
            self.export.connect_clicked(move |_| page.export());
        }

        {
            let page = Rc::clone(self);
            journal.connect_clicked(move |button| page.show_journal_command(button));
        }

        let page = Rc::clone(self);
        super::on_shown(&self.root, move || {
            let recovery = Rc::clone(&page.recovery);
            spawn(async move {
                recovery.collect().await;
            });
        });
    }

    /// Draw the cause and its evidence.
    pub fn draw(&self) {
        match self.recovery.cause() {
            Some(cause) => {
                self.status.set_title(&copy::text(cause.title()));
                // The sentence is always somebody else's, so an empty one means
                // nobody has said anything yet rather than a blank line.
                let sentence = cause.sentence();
                self.status
                    .set_description((!sentence.is_empty()).then_some(sentence));
            }
            None => {
                self.status.set_title(&copy::text(Key::PageRecovery));
                self.status.set_description(None);
            }
        }

        let lines = self.recovery.evidence();
        self.evidence_group.set_visible(!lines.is_empty());

        for row in self.evidence_rows.borrow_mut().drain(..) {
            self.evidence.remove(&row);
        }
        let mut drawn = Vec::with_capacity(lines.len());
        for line in &lines {
            let row = adw::ActionRow::builder().activatable(false).build();
            row.add_prefix(&identifier_label(line));
            self.evidence.add_row(&row);
            drawn.push(row);
        }
        self.evidence_rows.replace(drawn);

        self.export.set_sensitive(self.recovery.can_export());
        match self.recovery.refusal() {
            Some(refusal) => {
                self.refusal.set_label(&refusal.text);
                self.refusal.set_visible(true);
            }
            None if !self.recovery.can_export() => {
                self.refusal
                    .set_label(&copy::text(Key::RecoveryExportUnavailable));
                self.refusal.set_visible(true);
            }
            None => self.refusal.set_visible(false),
        }
    }

    /// The command that shows what the service wrote, as copyable text.
    fn show_journal_command(&self, widget: &impl IsA<gtk::Widget>) {
        let command = copy::text(Key::LogsJournalCommand);

        let dialog = adw::AlertDialog::new(
            Some(&copy::text(Key::RecoveryJournalDialogTitle)),
            Some(&copy::text(Key::LogsCaptionJournal)),
        );

        let label = identifier_label(&command);
        label.set_selectable(true);
        label.set_halign(gtk::Align::Center);
        dialog.set_extra_child(Some(&label));

        dialog.add_response("copy", &copy::text(Key::ActionCopyCommand));
        dialog.add_response("close", &copy::text(Key::ActionClose));
        dialog.set_default_response(Some("copy"));
        dialog.set_close_response("close");

        let command = command.clone();
        dialog.connect_response(None, move |dialog, response| {
            if response != "copy" {
                return;
            }
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&command);
            }
            dialog.close();
        });

        dialog.present(Some(widget.as_ref()));
    }

    /// Write the offline bundle where a person says.
    fn export(self: &Rc<Self>) {
        let Some(bytes) = self.recovery.export_bytes() else {
            return;
        };

        let dialog = gtk::FileDialog::builder()
            .title(copy::text(Key::RecoveryExportDiagnostics))
            .initial_name(EXPORT_NAME)
            .build();
        let window = super::window_of(&self.root);

        spawn(async move {
            let Ok(file) = dialog.save_future(window.as_ref()).await else {
                return;
            };
            if let Err(error) = file
                .replace_contents_future(
                    bytes,
                    None,
                    false,
                    gio::FileCreateFlags::REPLACE_DESTINATION,
                )
                .await
            {
                glib::g_warning!("fermix-desktop", "the bundle was not written: {error:?}");
            }
        });
    }
}

/// One of the three things there are to do, in the shape a status page's own
/// actions take.
fn action_button(label: Key, suggested: bool) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(copy::text(label))
        .halign(gtk::Align::Center)
        .build();

    if suggested {
        button.add_css_class("suggested-action");
        button.add_css_class("pill");
    }
    button
}

/// The name the offline bundle is offered under.
const EXPORT_NAME: &str = "fermix-diagnostics-offline.json";
