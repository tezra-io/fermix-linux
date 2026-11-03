//! A proposal in the conversation (M60 cues §5.3): a task under a mandate in approve mode wants
//! to run one call, or a watcher wants to run its follow-up (PROTOCOL.md "Version 5"), and it
//! runs only if the owner says so. Fermix's lines (the summary, the task, the watcher, the
//! expiry, the daemon's sentences, and a follow-up's instructions, tools and where it reports)
//! are drawn as Fermix's; the call's arguments, the filter's reason and the item that led to it
//! (`outside_parts`), and what the tool answered (the resolved footer's part), only by
//! `outside_text`, each part handed over whole.
//!
//! The buttons (a call's Deny and Confirm, a follow-up's Skip and Run, with Always allow where
//! its card offers it) look alike and none is the default widget, so Enter on the card does
//! nothing; each decides by its own click, holding the card's id and its word (§6 rule 3). A
//! press disables every button, and an answer that runs says it is running; the daemon's
//! resolution replaces the buttons. A screen reader hears the expiry as a card that asks
//! appears, and again a minute before it.
//!
//! The proposals the daemon sent that this app could not read stand as one card of words alone,
//! in the same box, saying where they can be answered (`chat_wire::unshown`).

use super::outside_text;
use super::{clock, decide_on_click, Decider, Decision};
use adw::prelude::*;
use fermix_client::chat_wire::proposals::{
    arrival, facts, last_minute_in, outside_parts, Card, CardAsk, Footer, EXPIRED, LAST_MINUTE,
    RUNNING, TITLE,
};
use fermix_client::chat_wire::unshown;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// A card on screen: the card it draws, its box, and the footer that follows the card's state.
pub struct ProposalCard {
    pub widget: gtk::Box,
    card: Card,
    footer: RefCell<gtk::Box>,
    decider: Decider,
    /// Whether the card still asks, for the last minute's announcement.
    asking: Rc<Cell<bool>>,
    _last_minute: LastMinute,
}

impl ProposalCard {
    pub fn new(card: &Card, decider: &Decider) -> ProposalCard {
        let (widget, inner) = shell(TITLE);
        inner.append(&lead(card));
        asked(card, &inner);
        let footer = footer(card, decider);
        inner.append(&footer);
        let asks = card.footer == Footer::Ask;
        if asks {
            widget.announce(
                &arrival(card, clock),
                gtk::AccessibleAnnouncementPriority::Medium,
            );
        }
        let asking = Rc::new(Cell::new(asks));
        let last_minute = LastMinute::start(card, &widget, &asking);
        ProposalCard {
            widget,
            card: card.clone(),
            footer: RefCell::new(footer),
            decider: decider.clone(),
            asking,
            _last_minute: last_minute,
        }
    }

    /// Takes `card`'s new state in place: the same card with another footer swaps the footer.
    /// False for anything else, which is drawn anew.
    pub fn grow(&mut self, card: &Card) -> bool {
        let same = Card {
            footer: card.footer.clone(),
            ..self.card.clone()
        };
        if same != *card {
            return false;
        }
        if self.card.footer != card.footer {
            let fresh = footer(card, &self.decider);
            let old = self.footer.replace(fresh.clone());
            let_go_of_focus(&old);
            let parent = old.parent().and_downcast::<gtk::Box>();
            if let Some(parent) = parent {
                parent.insert_child_after(&fresh, Some(&old));
                parent.remove(&old);
            }
            self.asking.set(card.footer == Footer::Ask);
            self.card = card.clone();
        }
        true
    }
}

/// The placeholder of the proposals that could not be read: its heading and where to answer
/// them, Fermix's words, with no buttons. A screen reader hears it as it appears.
pub fn unshown_card() -> gtk::Box {
    let (widget, inner) = shell(unshown::TITLE);
    let title = gtk::Label::builder()
        .label(unshown::TITLE)
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    inner.append(&title);
    inner.append(&line(unshown::PROPOSAL, &[]));
    widget.announce(
        unshown::PROPOSAL,
        gtk::AccessibleAnnouncementPriority::Medium,
    );
    widget
}

/// A card's box, a group named `name`, and the column inside it that its parts go in.
fn shell(name: &str) -> (gtk::Box, gtk::Box) {
    let widget = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .hexpand(true)
        .accessible_role(gtk::AccessibleRole::Group)
        .css_classes(["card"])
        .build();
    widget.update_property(&[gtk::accessible::Property::Label(name)]);
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(14)
        .margin_end(14)
        .build();
    widget.append(&inner);
    (widget, inner)
}

/// A pressed button goes with the footer it stood in. Its focus would move, at the next frame,
/// to the card's first selectable label, selecting it and scrolling the conversation there, so
/// the window lets go of it first.
fn let_go_of_focus(footer: &gtk::Box) {
    let Some(root) = footer.root() else {
        return;
    };
    if root.focus().is_some_and(|focus| focus.is_ancestor(footer)) {
        root.set_focus(None::<&gtk::Widget>);
    }
}

/// What the card asks, under its lead: a follow-up in Fermix's and the owner's words; then the
/// card's outside text as `outside_parts` lists it (a call's arguments, the filter's reason, the
/// item), each part handed whole to `outside_text`.
fn asked(card: &Card, inner: &gtk::Box) {
    if let CardAsk::FollowUp {
        instructions,
        facts,
    } = &card.ask
    {
        inner.append(&follow_up(instructions, facts));
    }
    for part in outside_parts(card, clock) {
        inner.append(&outside_text::block(&part));
    }
}

/// A follow-up's lines: what it will do, then where it reports and its tools.
fn follow_up(instructions: &str, facts: &str) -> gtk::Box {
    let lines = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    lines.append(&line(instructions, &[]));
    lines.append(&line(facts, &["caption", "dimmed"]));
    lines
}

/// The title, the summary, and the task, the watcher and the expiry under them: Fermix's lines.
fn lead(card: &Card) -> gtk::Box {
    let lead = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    let title = gtk::Label::builder()
        .label(TITLE)
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    lead.append(&title);
    lead.append(&line(&card.summary, &[]));
    lead.append(&line(&facts(card, clock), &["caption", "dimmed"]));
    lead
}

/// One of Fermix's sentences, as written: wrapping, inside a word too where one is wider than the
/// card (a hash, a path), selectable, with no markup read.
fn line(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.set_text(text);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_selectable(true);
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// Under the outside text: the buttons while the card asks or its answer is on its way, the
/// daemon's line for a card it does not say waits, the expiry, or the daemon's last word.
fn footer(card: &Card, decider: &Decider) -> gtk::Box {
    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    match &card.footer {
        Footer::Ask => footer.append(&buttons(card, decider, None)),
        Footer::Answered { running } => {
            footer.append(&buttons(card, decider, Some(*running)));
        }
        Footer::Status(sentence) => footer.append(&line(sentence, &["dimmed"])),
        Footer::Expired => footer.append(&line(EXPIRED, &["dimmed"])),
        Footer::Resolved { sentence, result } => {
            footer.append(&line(sentence, &[]));
            if let Some(answered) = result {
                footer.append(&outside_text::block(answered));
            }
        }
    }
    footer
}

/// The card's answers, alike, at its end. `answered` is `None` while the card asks, else whether
/// the answer runs: every button then waits, and one that runs says it is running.
fn buttons(card: &Card, decider: &Decider, answered: Option<bool>) -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::End)
        .build();
    if answered == Some(true) {
        let running = gtk::Label::builder()
            .label(RUNNING)
            .css_classes(["caption", "dimmed"])
            .build();
        row.append(&running);
    }
    for (label, word) in &card.answers {
        let button = gtk::Button::builder()
            .label(*label)
            .sensitive(answered.is_none())
            .receives_default(false)
            .build();
        let decision = Decision::Proposal {
            id: card.id.clone(),
            decision: *word,
        };
        decide_on_click(&button, decider, decision);
        row.append(&button);
    }
    row
}

/// The announcement a minute before the card's expiry, while it still asks. Its timer goes
/// with the card.
struct LastMinute(Rc<RefCell<Option<glib::SourceId>>>);

impl LastMinute {
    fn start(card: &Card, widget: &gtk::Box, asking: &Rc<Cell<bool>>) -> LastMinute {
        let slot: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let now = glib::real_time() / 1_000_000;
        let Some(wait) = last_minute_in(card, now).filter(|_| asking.get()) else {
            return LastMinute(slot);
        };
        let (fired, widget, asking) = (slot.clone(), widget.downgrade(), asking.clone());
        let source = glib::timeout_add_local_once(Duration::from_secs(wait), move || {
            fired.take();
            let Some(widget) = widget.upgrade().filter(|_| asking.get()) else {
                return;
            };
            widget.announce(LAST_MINUTE, gtk::AccessibleAnnouncementPriority::High);
        });
        slot.replace(Some(source));
        LastMinute(slot)
    }
}

impl Drop for LastMinute {
    fn drop(&mut self) {
        if let Some(source) = self.0.take() {
            source.remove();
        }
    }
}
