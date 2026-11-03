//! The conversation's entries as widgets: the person's bubbles and Fermix's,
//! with a voice note's chip in its bubble over the words (its tile in the
//! lane once it cannot play), the cards of the places a reply links under it,
//! the card of the lists its turn read or changed, and what a row shows under
//! its words
//! (pictures, files, link cards), the tools it ran, a mandate's proposals
//! and the placeholder of those that could not be read, and failures. A
//! streamed entry grows in place, so a chunk never redraws what is already on
//! screen.

use super::activity::{Group, Live};
use super::blocks::Shown;
use super::lists::{Lister, ListsView};
use super::places::Places;
use super::proposals::{unshown_card, ProposalCard};
use super::strip::{Readers, StripView};
use super::voice_chip::Chip;
use super::Decider;
use adw::prelude::*;
use fermix_client::chat::{Entry, ListsCard};
use fermix_client::checklists::library::Library;
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
    /// Boxed, since it holds the reply's every part.
    Reply(Box<Reply>),
    /// Tools that ran in a row: live while the turn is on them, then one line.
    Tools(Group),
    /// A proposal's card, whose footer follows the card's state. Boxed, since it holds the whole
    /// card it draws.
    Proposal(Box<ProposalCard>),
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
    /// The card of the lists its turn read or changed, under the place cards; it comes with its
    /// row.
    lists: ListsView,
    /// What its row shows under the place cards.
    strip: SharedStrip,
    /// The row's voice note, over the words.
    voice: Option<Rc<Chip>>,
}

/// What a reply's row shows under its words: the place cards, the lists card, then its strip.
struct Under<'a> {
    places: &'a [Card],
    lists: Option<&'a ListsCard>,
    media: &'a Strip,
}

/// The person's words, and what their row shows under them.
struct Message {
    /// The label alone, or with a voice note's chip over it.
    bubble: gtk::Widget,
    label: gtk::Label,
    source: Source,
    strip: SharedStrip,
    voice: Option<Rc<Chip>>,
}

impl Drawn {
    /// `decider` takes what a proposal's buttons decide, and `lister` what a lists card asks.
    pub fn new(entry: &Entry, readers: &Readers, decider: &Decider, lister: &Lister) -> Drawn {
        if let Entry::Proposal(card) = entry {
            let card = ProposalCard::new(card, decider);
            return Drawn::own_row(entry, &card.widget.clone(), Parts::Proposal(Box::new(card)));
        }
        if let Entry::Unshown = entry {
            return Drawn::own_row(entry, &unshown_card(), Parts::Fixed);
        }
        let (side, body, source, parts) = draw_body(entry, readers, lister);
        let (row, time) = row(side, &body, source);
        let line = row.first_child();
        // Between the bubble's line and the time under it.
        let mut after = line.clone();
        for widget in parts.under_line() {
            row.insert_child_after(&widget, after.as_ref());
            after = Some(widget);
        }
        if let Some(line) = &line {
            // A row of files alone draws no empty bubble.
            line.set_visible(!blank(entry, parts.note_plays()));
            if let Some((chip, strip, words)) = parts.note() {
                when_unplayable(chip, line, strip, words);
            }
        }
        Drawn {
            entry: entry.clone(),
            row,
            time,
            parts,
        }
    }

    /// A card on a row of its own, apart from the messages around it, as wide as the
    /// conversation; it answers none of them.
    fn own_row(entry: &Entry, card: &gtk::Box, parts: Parts) -> Drawn {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(18)
            .margin_bottom(12)
            .css_classes(["chat-row"])
            .build();
        row.append(card);
        let time = gtk::Label::builder().visible(false).build();
        row.append(&time);
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
                    lists,
                },
            ) => {
                if !same_note(reply.voice.as_ref(), media) {
                    return false;
                }
                reply.grow(text);
                reply.places.show(places);
                reply.lists.set_card(lists.as_ref());
                reply.strip.borrow_mut().show(media);
            }
            (Parts::Message(message), Entry::User { text, media }) => {
                if !same_note(message.voice.as_ref(), media) {
                    return false;
                }
                message.grow(text, media);
            }
            (Parts::Tools(group), Entry::Tools(runs)) => {
                if !group.grow(runs) {
                    return false;
                }
            }
            (Parts::Proposal(card), Entry::Proposal(new)) => {
                if !card.grow(new) {
                    return false;
                }
            }
            _ => return false,
        }
        if let Some(line) = self.row.first_child() {
            line.set_visible(!blank(entry, self.parts.note_plays()));
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

    /// A reply's lists card, drawn as `lists` holds its lists now. `on_screen`: the entry was on
    /// screen at the page's last look.
    pub fn show_lists(&mut self, lists: &Library, on_screen: bool) {
        if let Parts::Reply(reply) = &mut self.parts {
            reply.lists.show(lists, on_screen);
        }
    }

    /// The entry is on screen: what its row shows is read, where it has not been. `came`: it came
    /// on screen since the page's last look, so a live lists card pulls its lists.
    pub fn on_screen(&self, came: bool) {
        let (strip, voice) = match &self.parts {
            Parts::Message(message) => (&message.strip, &message.voice),
            Parts::Reply(reply) => (&reply.strip, &reply.voice),
            Parts::Fixed | Parts::Tools(_) | Parts::Proposal(_) => return,
        };
        if let (Parts::Reply(reply), true) = (&self.parts, came) {
            reply.lists.came_on_screen();
        }
        strip.borrow().on_screen();
        if let Some(chip) = voice {
            chip.on_screen();
        }
    }
}

impl Parts {
    /// What stands under the entry's line, in order.
    fn under_line(&self) -> Vec<gtk::Widget> {
        match self {
            Parts::Message(message) => vec![message.strip.borrow().widget.clone().upcast()],
            Parts::Reply(reply) => vec![
                reply.places.widget.clone().upcast(),
                reply.lists.widget.clone().upcast(),
                reply.strip.borrow().widget.clone().upcast(),
            ],
            Parts::Fixed | Parts::Tools(_) | Parts::Proposal(_) => Vec::new(),
        }
    }

    /// The row's voice note, with its strip and its words, where it has one.
    fn note(&self) -> Option<(&Rc<Chip>, &SharedStrip, &Source)> {
        match self {
            Parts::Message(message) => message
                .voice
                .as_ref()
                .map(|chip| (chip, &message.strip, &message.source)),
            Parts::Reply(reply) => reply
                .voice
                .as_ref()
                .map(|chip| (chip, &reply.strip, &reply.source)),
            Parts::Fixed | Parts::Tools(_) | Parts::Proposal(_) => None,
        }
    }

    /// Whether the row has a voice note that can still play, in its bubble.
    fn note_plays(&self) -> bool {
        self.note().is_some_and(|(chip, _, _)| chip.plays())
    }
}

/// When the row's voice note cannot play, its tile leads the row's lane, and a line left with no
/// words goes, as a row of files alone draws none.
fn when_unplayable(chip: &Chip, line: &gtk::Widget, strip: &SharedStrip, words: &Source) {
    let line = line.downgrade();
    let strip = Rc::downgrade(strip);
    let words = Rc::downgrade(words);
    chip.when_unplayable(move || {
        if let Some(strip) = strip.upgrade() {
            strip.borrow_mut().unplayable();
        }
        if let (Some(line), Some(words)) = (line.upgrade(), words.upgrade()) {
            line.set_visible(!words.borrow().trim().is_empty());
        }
    });
}

/// Whether the entry is a message with nothing for its bubble: no words and no voice note that
/// plays, a row of files alone.
fn blank(entry: &Entry, note_plays: bool) -> bool {
    match entry {
        Entry::User { text, .. } | Entry::Assistant { text, .. } => {
            text.trim().is_empty() && !note_plays
        }
        Entry::Sending(_)
        | Entry::Tools(_)
        | Entry::Proposal(_)
        | Entry::Unshown
        | Entry::Failure(_) => false,
    }
}

/// Whether `media` has the voice note `drawn` shows, or none as it shows none. Another note is
/// another bubble, drawn anew.
fn same_note(drawn: Option<&Rc<Chip>>, media: &Strip) -> bool {
    drawn.map(|chip| chip.r#ref()) == media.voice.as_ref().map(|note| note.r#ref.as_str())
}

/// The entry's widget, its side, what copying it copies, and its parts.
fn draw_body(
    entry: &Entry,
    readers: &Readers,
    lister: &Lister,
) -> (Side, gtk::Widget, Option<Source>, Parts) {
    match entry {
        Entry::User { text, media } => {
            let message = Message::new(text, media, readers);
            let (bubble, source) = (message.bubble.clone(), message.source.clone());
            (Side::Person, bubble, Some(source), Parts::Message(message))
        }
        Entry::Assistant {
            text,
            places,
            media,
            lists,
        } => {
            let under = Under {
                places,
                lists: lists.as_ref(),
                media,
            };
            let reply = Reply::new(text, &under, readers, lister);
            let source = reply.source.clone();
            (
                Side::Fermix,
                reply.body.clone().upcast(),
                Some(source),
                Parts::Reply(Box::new(reply)),
            )
        }
        Entry::Sending(text) => (Side::Person, sending_bubble(text), None, Parts::Fixed),
        Entry::Tools(runs) => {
            let group = Group::new(runs);
            (Side::Fermix, group.widget(), None, Parts::Tools(group))
        }
        Entry::Failure(text) => (Side::Fermix, failure(text), None, Parts::Fixed),
        Entry::Proposal(_) | Entry::Unshown => {
            unreachable!("a card is drawn on a row of its own")
        }
    }
}

/// The Markdown or text a message's copy button and menu item copy.
type Source = Rc<RefCell<String>>;
/// What a row shows under its words, shared with its voice note's hook.
type SharedStrip = Rc<RefCell<StripView>>;

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

/// Shows `text` in the window's toast. A toast that cannot show is logged without its text,
/// which can name a row's file. Activating fails only where the action does not exist.
pub(super) fn toast(widget: &impl IsA<gtk::Widget>, text: &str) {
    let shown = widget.activate_action("win.toast", Some(&text.to_variant()));
    if shown.is_err() {
        glib::g_warning!(
            "fermix",
            "could not show a toast: no win.toast reaches this widget"
        );
    }
}

/// A GLib error as a log line names it: its domain and code, never its message, which can quote
/// a file's path or a link's address.
pub(super) fn error_code(e: &glib::Error) -> String {
    format!("{} {}", e.domain().as_str(), e.code())
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
        let strip = Rc::new(RefCell::new(StripView::new(gtk::Align::End, readers)));
        strip.borrow_mut().show(media);
        let label = user_bubble(text);
        let voice = media.voice.as_ref().map(|note| Chip::new(note, readers));
        let bubble = match &voice {
            Some(chip) => voice_bubble(chip, &label, text),
            None => label.clone().upcast(),
        };
        Message {
            bubble,
            label,
            source: Rc::new(RefCell::new(text.to_owned())),
            strip,
            voice,
        }
    }

    /// The row read again: its words as they stand now, and what it shows.
    fn grow(&mut self, text: &str, media: &Strip) {
        if self.label.label() != text {
            self.label.set_label(text);
            self.source.replace(text.to_owned());
        }
        if self.voice.is_some() {
            self.label.set_visible(!text.trim().is_empty());
        }
        self.strip.borrow_mut().show(media);
    }
}

/// The person's bubble for a voice note: its chip, and its transcript under it where there is one.
fn voice_bubble(chip: &Chip, label: &gtk::Label, text: &str) -> gtk::Widget {
    label.remove_css_class("chat-user");
    label.set_visible(!text.trim().is_empty());
    let bubble = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["chat-user"])
        .build();
    bubble.append(&chip.widget);
    bubble.append(label);
    bubble.upcast()
}

impl Reply {
    fn new(text: &str, under: &Under, readers: &Readers, lister: &Lister) -> Reply {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .accessible_role(gtk::AccessibleRole::Group)
            .css_classes(["chat-reply"])
            .build();
        body.update_property(&[gtk::accessible::Property::Label("Reply from Fermix")]);
        let voice = under
            .media
            .voice
            .as_ref()
            .map(|note| Chip::new(note, readers));
        if let Some(chip) = &voice {
            body.append(&chip.widget);
        }
        let mut reply = Reply {
            body,
            blocks: Vec::new(),
            draft: Draft::default(),
            source: Rc::default(),
            places: Places::new(),
            lists: ListsView::new(lister),
            strip: Rc::new(RefCell::new(StripView::new(gtk::Align::Start, readers))),
            voice,
        };
        reply.grow(text);
        reply.places.show(under.places);
        reply.lists.set_card(under.lists);
        reply.strip.borrow_mut().show(under.media);
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
