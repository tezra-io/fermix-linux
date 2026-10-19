//! The conversation's entries as widgets: the person's bubbles and Fermix's,
//! with the cards of the places a reply links under it and what a row shows
//! under its words (pictures, files, link cards), the tools it ran, and
//! failures. A streamed entry grows in place, so a chunk never redraws what is
//! already on screen.

use super::activity::{Group, Live};
use super::blocks::Shown;
use super::places::Places;
use super::strip::{Readers, StripView};
use adw::prelude::*;
use fermix_client::chat::Entry;
use fermix_client::markdown::{Block, Draft};
use fermix_client::media::Strip;
use fermix_client::places::Card;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Person,
    Fermix,
}

/// One drawn entry: its row in the list, the time under it, and the parts
/// that change after it is drawn.
pub struct Drawn {
    pub entry: Entry,
    pub row: gtk::Box,
    time: gtk::Label,
    parts: Parts,
}

enum Parts {
    Fixed,
    Message(Message),
    Reply(Reply),
    /// Tools that ran in a row: live while the turn is on them, then one line.
    Tools(Group),
}

struct Reply {
    body: gtk::Box,
    blocks: Vec<(Block, Shown)>,
    /// The reply's Markdown parsed so far, so a streamed piece re-parses only
    /// the part still being written.
    draft: Draft,
    /// The Markdown the copy button copies, kept current while the reply grows.
    source: Rc<RefCell<String>>,
    /// The cards of the places it links, under the bubble; they come with its row.
    places: Places,
    /// What its row shows under the place cards.
    strip: StripView,
}

/// The person's words, and what their row shows under them.
struct Message {
    label: gtk::Label,
    source: Source,
    strip: StripView,
}

impl Drawn {
    pub fn new(entry: &Entry, readers: &Readers) -> Drawn {
        let (side, body, source, parts) = draw_body(entry, readers);
        let (row, time) = row(side, &body, source);
        let line = row.first_child();
        // Between the bubble's line and the time under it.
        let mut after = line.clone();
        for widget in parts.under_line() {
            row.insert_child_after(&widget, after.as_ref());
            after = Some(widget);
        }
        // A row of files alone draws no empty bubble.
        if let Some(line) = line {
            line.set_visible(!blank(entry));
        }
        Drawn {
            entry: entry.clone(),
            row,
            time,
            parts,
        }
    }

    /// Grows this entry into `entry`, the same entry with more streamed in or
    /// a new status; false when `entry` is a different one and has to be
    /// drawn anew. Words coming to a row of files alone show its bubble, and
    /// what the row shows stays as it is, read.
    pub fn grow(&mut self, entry: &Entry) -> bool {
        match (&mut self.parts, entry) {
            (
                Parts::Reply(reply),
                Entry::Assistant {
                    text,
                    places,
                    media,
                },
            ) => {
                reply.grow(text);
                reply.places.show(places);
                reply.strip.show(media);
            }
            (Parts::Message(message), Entry::User { text, media }) => message.grow(text, media),
            (Parts::Tools(group), Entry::Tools(runs)) => {
                if !group.grow(runs) {
                    return false;
                }
            }
            _ => return false,
        }
        if let Some(line) = self.row.first_child() {
            line.set_visible(!blank(entry));
        }
        self.entry = entry.clone();
        true
    }

    /// What depends on the rest of the conversation: the time under the entry,
    /// and whether the turn is still on it.
    pub fn decorate(&self, time: Option<&str>, live: Option<&Live>) {
        self.time.set_visible(time.is_some());
        self.time.set_text(time.unwrap_or_default());
        self.set_live(live);
    }

    /// The parts that follow the running turn, which the page's clock also
    /// updates every second.
    pub fn set_live(&self, live: Option<&Live>) {
        if let (Parts::Tools(group), Entry::Tools(runs)) = (&self.parts, &self.entry) {
            group.set_live(runs, live);
        }
    }

    /// The entry is on screen: what its row shows is read, where it has not been.
    pub fn on_screen(&self) {
        match &self.parts {
            Parts::Message(message) => message.strip.on_screen(),
            Parts::Reply(reply) => reply.strip.on_screen(),
            Parts::Fixed | Parts::Tools(_) => {}
        }
    }
}

impl Parts {
    /// What stands under the entry's line, in order.
    fn under_line(&self) -> Vec<gtk::Widget> {
        match self {
            Parts::Message(message) => vec![message.strip.widget.clone().upcast()],
            Parts::Reply(reply) => vec![
                reply.places.widget.clone().upcast(),
                reply.strip.widget.clone().upcast(),
            ],
            Parts::Fixed | Parts::Tools(_) => Vec::new(),
        }
    }
}

/// Whether the entry is a message without words: a row of files alone.
fn blank(entry: &Entry) -> bool {
    match entry {
        Entry::User { text, .. } | Entry::Assistant { text, .. } => text.trim().is_empty(),
        Entry::Sending(_) | Entry::Tools(_) | Entry::Failure(_) => false,
    }
}

/// The entry's widget, its side, what copying it copies, and its parts.
fn draw_body(entry: &Entry, readers: &Readers) -> (Side, gtk::Widget, Option<Source>, Parts) {
    match entry {
        Entry::User { text, media } => {
            let message = Message::new(text, media, readers);
            let (label, source) = (message.label.clone(), message.source.clone());
            (
                Side::Person,
                label.upcast(),
                Some(source),
                Parts::Message(message),
            )
        }
        Entry::Assistant {
            text,
            places,
            media,
        } => {
            let reply = Reply::new(text, places, media, readers);
            let source = reply.source.clone();
            (
                Side::Fermix,
                reply.body.clone().upcast(),
                Some(source),
                Parts::Reply(reply),
            )
        }
        Entry::Sending(text) => (Side::Person, sending_bubble(text), None, Parts::Fixed),
        Entry::Tools(runs) => {
            let group = Group::new(runs);
            (Side::Fermix, group.widget(), None, Parts::Tools(group))
        }
        Entry::Failure(text) => (Side::Fermix, failure(text), None, Parts::Fixed),
    }
}

/// The Markdown or text a message's copy button and menu item copy.
type Source = Rc<RefCell<String>>;

/// The entry on its side of the conversation, its copy button on the inside
/// edge, and the time under it, hidden until there is one. A message's row
/// also carries `message.copy`, which its labels' right-click menu offers.
fn row(side: Side, body: &gtk::Widget, source: Option<Source>) -> (gtk::Box, gtk::Label) {
    let align = match side {
        Side::Person => gtk::Align::End,
        Side::Fermix => gtk::Align::Start,
    };
    let line = gtk::Box::builder().spacing(6).halign(align).build();
    if let Some(source) = &source {
        line.insert_action_group("message", Some(&message_actions(&line, source.clone())));
    }
    match source.map(copy_button) {
        Some(copy) if side == Side::Person => {
            line.append(&copy);
            line.append(body);
        }
        Some(copy) => {
            line.append(body);
            line.append(&copy);
        }
        None => line.append(body),
    }
    let time = gtk::Label::builder()
        .css_classes(["dimmed", "caption", "numeric"])
        .halign(align)
        .margin_start(10)
        .margin_end(10)
        .visible(false)
        .build();
    let side_class = match side {
        Side::Person => "person",
        Side::Fermix => "fermix",
    };
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(["chat-row", side_class])
        .build();
    row.append(&line);
    row.append(&time);
    (row, time)
}

/// `message.copy`, for the right-click menu of the labels inside `line`.
fn message_actions(line: &gtk::Box, source: Source) -> gio::SimpleActionGroup {
    let copy = gio::SimpleAction::new("copy", None);
    let line = line.downgrade();
    copy.connect_activate(move |_, _| {
        if let Some(line) = line.upgrade() {
            copy_text(&line, &source.borrow());
        }
    });
    let group = gio::SimpleActionGroup::new();
    group.add_action(&copy);
    group
}

/// Added to a message label's own right-click menu (Copy, Select All).
pub(super) fn message_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Copy Message"), Some("message.copy"));
    menu
}

fn copy_button(source: Source) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy")
        .valign(gtk::Align::End)
        .css_classes(["flat", "circular", "chat-hover"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Copy message")]);
    button.connect_clicked(move |button| copy_text(button, &source.borrow()));
    button
}

pub(super) fn copy_text(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.clipboard().set_text(text);
    toast(widget, "Copied");
}

pub(super) fn toast(widget: &impl IsA<gtk::Widget>, text: &str) {
    if let Err(e) = widget.activate_action("win.toast", Some(&text.to_variant())) {
        glib::g_warning!("fermix", "could not show the toast {text:?}: {e}");
    }
}

fn user_bubble(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .max_width_chars(60)
        .extra_menu(&message_menu())
        .css_classes(["chat-user"])
        .build();
    label.update_property(&[gtk::accessible::Property::Description("Your message")]);
    label
}

/// A question on its way: the person's bubble, dimmed, until the daemon accepts it.
fn sending_bubble(text: &str) -> gtk::Widget {
    let bubble = user_bubble(text);
    bubble.set_opacity(0.6);
    bubble.set_tooltip_text(Some("Sending"));
    bubble.upcast()
}

impl Message {
    fn new(text: &str, media: &Strip, readers: &Readers) -> Message {
        let mut strip = StripView::new(gtk::Align::End, readers);
        strip.show(media);
        Message {
            label: user_bubble(text),
            source: Rc::new(RefCell::new(text.to_owned())),
            strip,
        }
    }

    /// The row read again: its words as they stand now, and what it shows.
    fn grow(&mut self, text: &str, media: &Strip) {
        if self.label.label() != text {
            self.label.set_label(text);
            self.source.replace(text.to_owned());
        }
        self.strip.show(media);
    }
}

impl Reply {
    fn new(text: &str, places: &[Card], media: &Strip, readers: &Readers) -> Reply {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .accessible_role(gtk::AccessibleRole::Group)
            .css_classes(["chat-reply"])
            .build();
        body.update_property(&[gtk::accessible::Property::Label("Reply from Fermix")]);
        let mut reply = Reply {
            body,
            blocks: Vec::new(),
            draft: Draft::default(),
            source: Rc::default(),
            places: Places::new(),
            strip: StripView::new(gtk::Align::Start, readers),
        };
        reply.grow(text);
        reply.places.show(places);
        reply.strip.show(media);
        reply
    }

    /// Unchanged blocks stay, the first changed one is updated in place when
    /// it is the same kind, and anything after it is redrawn.
    fn grow(&mut self, text: &str) {
        let tail = self.draft.update(text);
        let fresh: Vec<&Block> = self.draft.settled().iter().chain(&tail).collect();
        let same = self
            .blocks
            .iter()
            .zip(&fresh)
            .take_while(|((old, _), new)| old == **new)
            .count();
        let mut next = same;
        if let (Some((old, shown)), Some(new)) = (self.blocks.get_mut(same), fresh.get(same)) {
            if shown.update(old, new) {
                *old = (*new).clone();
                next += 1;
            }
        }
        for (_, shown) in self.blocks.drain(next..) {
            self.body.remove(&shown.widget);
        }
        for block in fresh.into_iter().skip(next) {
            let shown = Shown::new(block);
            self.body.append(&shown.widget);
            self.blocks.push((block.clone(), shown));
        }
        self.source.replace(text.to_owned());
    }
}

/// Why the reply did not come, and where the reason is.
fn failure(text: &str) -> gtk::Widget {
    let icon = gtk::Image::from_icon_name("dialog-error-symbolic");
    icon.add_css_class("error");
    icon.set_valign(gtk::Align::Start);
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .build();
    // The reason for any failure is in the daemon's log, which Logs shows.
    let logs = gtk::Button::builder()
        .label("Open Logs")
        .css_classes(["flat"])
        .action_name("win.page")
        .action_target(&"logs".to_variant())
        .build();
    let actions = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Start)
        .build();
    actions.append(&logs);
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    column.append(&label);
    column.append(&actions);
    let bubble = gtk::Box::builder()
        .spacing(10)
        .css_classes(["chat-failure"])
        .build();
    bubble.append(&icon);
    bubble.append(&column);
    bubble.upcast()
}
