//! Boot failed, and the refusals that come before anything is written.
//!
//! One shape for every named state: what happened, what is untouched, the one
//! next action, and the buttons that state offers. The title and the paragraph
//! are the catalogue's, because these are states this application names; the
//! sentence under them is always the command line's or the daemon's own, and
//! the evidence is the offline log tail the command line collected.
//!
//! A command a person has to run is shown as text they can select and copy,
//! because the command is the action.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::activation::{linger_command_row, Action, Refusal};
use crate::models::onboarding::{OnboardingModel, Snapshot};

use super::Screen;
use crate::ui::plain;

/// The Boot failed screen.
pub struct BootFailedScreen {
    root: gtk::Widget,
    model: Rc<OnboardingModel>,
    title: gtk::Label,
    body: gtk::Label,
    sentence: gtk::Label,
    next_action: gtk::Label,
    command_group: adw::PreferencesGroup,
    command_row: adw::ActionRow,
    command: gtk::Label,
    evidence: adw::PreferencesGroup,
    evidence_row: adw::ExpanderRow,
    evidence_lines: RefCell<Vec<adw::ActionRow>>,
    buttons: gtk::Box,
    drawn: RefCell<Option<Refusal>>,
}

impl BootFailedScreen {
    /// Build it over the assistant's model.
    pub fn new(model: &Rc<OnboardingModel>) -> Rc<Self> {
        let column = super::screen_column();

        let title = heading();
        let body = paragraph();
        let sentence = paragraph();
        let next_action = paragraph();
        let (command_group, command_row, command) = command_group();
        let (evidence, evidence_row) = evidence_group();
        let buttons = button_row();

        column.append(&title);
        column.append(&body);
        column.append(&sentence);
        column.append(&next_action);
        column.append(&command_group);
        column.append(&evidence);
        column.append(&buttons);

        Rc::new(Self {
            root: super::screen(&column),
            model: Rc::clone(model),
            title,
            body,
            sentence,
            next_action,
            command_group,
            command_row,
            command,
            evidence,
            evidence_row,
            evidence_lines: RefCell::new(Vec::new()),
            buttons,
            drawn: RefCell::new(None),
        })
    }

    /// Draw one refusal. The whole card is rebuilt only when the refusal
    /// itself changes, so a redraw does not take the focus off a button.
    fn draw_refusal(self: &Rc<Self>, refusal: &Refusal) {
        self.title.set_label(&copy::text(refusal.title));

        set_label(&self.body, Some(refusal.body.as_str()));
        set_label(&self.sentence, refusal.sentence.as_deref());
        set_label(
            &self.next_action,
            refusal.next_action.map(copy::text).as_deref(),
        );

        self.draw_command(refusal);
        self.draw_evidence(refusal);
        self.draw_buttons(refusal);
    }

    /// The command a person runs, with the account this process runs as in it
    /// where the design's own text names one.
    fn draw_command(&self, refusal: &Refusal) {
        let Some((label, command)) = refusal.command.as_ref() else {
            self.command_group.set_visible(false);
            return;
        };

        let row = if *label == Key::LingerDeniedCommandRow {
            linger_command_row()
        } else {
            copy::text(*label)
        };

        self.command_row.set_title(&row);
        // The sentence the design fixes for a linger refusal names the command
        // inside itself; the one for a system-scope refusal does not. The
        // command is drawn beside the row only where the row does not already
        // carry it, so nothing says it twice.
        self.command.set_label(command);
        self.command.set_visible(!row.contains(command));
        self.command_group.set_visible(true);
    }

    /// The offline boot evidence, where the command line could collect it.
    fn draw_evidence(&self, refusal: &Refusal) {
        for row in self.evidence_lines.borrow_mut().drain(..) {
            self.evidence_row.remove(&row);
        }

        self.evidence.set_visible(!refusal.evidence.is_empty());
        if refusal.evidence.is_empty() {
            return;
        }

        let mut drawn = Vec::with_capacity(refusal.evidence.len());
        for line in &refusal.evidence {
            let row = plain(adw::ActionRow::builder().activatable(false).build());
            row.add_prefix(&crate::ui::identifier_label(line));
            self.evidence_row.add_row(&row);
            drawn.push(row);
        }
        self.evidence_lines.replace(drawn);
    }

    fn draw_buttons(self: &Rc<Self>, refusal: &Refusal) {
        while let Some(child) = self.buttons.first_child() {
            self.buttons.remove(&child);
        }

        for action in &refusal.actions {
            self.buttons.append(&self.button(*action, refusal));
        }
    }

    fn button(self: &Rc<Self>, action: Action, refusal: &Refusal) -> gtk::Button {
        let button = gtk::Button::builder()
            .label(copy::text(action.key()))
            .build();
        if action == Action::TryAgain {
            button.add_css_class("suggested-action");
        }

        let screen = Rc::clone(self);
        let command = refusal.command.as_ref().map(|(_, value)| value.clone());
        button.connect_clicked(move |button| screen.run(action, command.as_deref(), button));

        button
    }

    /// What one button does. Every route here is one the window already has:
    /// nothing on this card reaches a surface the menus cannot.
    fn run(self: &Rc<Self>, action: Action, command: Option<&str>, anchor: &gtk::Button) {
        match action {
            Action::TryAgain => self.model.begin(),
            Action::Cancel => self.model.leave(),
            Action::CopyCommand | Action::CopyCommands => copy_to_clipboard(anchor, command),
            Action::ShowMeHow => open_website(anchor),
            Action::RunDoctor => {
                self.model.leave();
                let _ = WidgetExt::activate_action(anchor, "win.run-doctor", None);
            }
            Action::ViewLog => {
                self.model.leave();
                let _ = WidgetExt::activate_action(anchor, "win.logs", None);
            }
        }
    }
}

impl Screen for BootFailedScreen {
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    fn draw(self: Rc<Self>, snapshot: &Snapshot) {
        let Some(refusal) = snapshot.failure.as_ref() else {
            return;
        };
        if self.drawn.borrow().as_ref() == Some(refusal) {
            return;
        }

        self.draw_refusal(refusal);
        self.drawn.replace(Some(refusal.clone()));
    }
}

fn heading() -> gtk::Label {
    let label = gtk::Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .build();
    label.add_css_class("title-1");
    label
}

fn paragraph() -> gtk::Label {
    gtk::Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build()
}

/// The command row: the words that introduce it, and the command itself as text
/// a person can select.
fn command_group() -> (adw::PreferencesGroup, adw::ActionRow, gtk::Label) {
    let group = adw::PreferencesGroup::builder().visible(false).build();
    let row = plain(adw::ActionRow::builder().activatable(false).build());
    row.set_title_lines(0);

    let command = gtk::Label::builder()
        .label("")
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .build();
    command.add_css_class("monospace");

    row.add_suffix(&command);
    group.add(&row);
    (group, row, command)
}

/// The offline evidence, behind an expander: it is there to be read when it is
/// wanted and it is not what the card is about.
fn evidence_group() -> (adw::PreferencesGroup, adw::ExpanderRow) {
    let group = adw::PreferencesGroup::builder().visible(false).build();
    let row = plain(
        adw::ExpanderRow::builder()
            .title(copy::text(Key::ActivationLastLogLines))
            .expanded(false)
            .build(),
    );

    group.add(&row);
    (group, row)
}

fn button_row() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(crate::metrics::SPACE_HEADING)
        .halign(gtk::Align::Start)
        .build()
}

fn set_label(label: &gtk::Label, text: Option<&str>) {
    match text {
        Some(text) if !text.is_empty() => {
            label.set_label(text);
            label.set_visible(true);
        }
        _ => label.set_visible(false),
    }
}

fn copy_to_clipboard(anchor: &gtk::Button, command: Option<&str>) {
    let Some(command) = command else {
        return;
    };
    anchor.display().clipboard().set_text(command);
}

fn open_website(anchor: &gtk::Button) {
    let window = crate::ui::window_of(anchor);
    super::run(async move {
        let _ =
            crate::session::DesktopSession::open_url(window.as_ref(), crate::app::WEBSITE).await;
    });
}
