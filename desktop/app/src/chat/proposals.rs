//! A mandate's proposal in the conversation (M60 cues §5.3): a task under a mandate in approve
//! mode wants to run one call, and runs it only if the owner confirms. Fermix's lines (the
//! summary, the task, the watcher, the expiry and the daemon's sentences) are drawn as Fermix's;
//! the call's arguments, the item that led to it and what the tool answered only by
//! `outside_text`.
//!
//! Deny and Confirm look alike and neither is the default widget, so Enter on the card does
//! nothing; each decides by its own click, holding the card's id (§6 rule 3). A press disables
//! both, and a confirm says it is running; the daemon's resolution replaces the buttons. A screen
//! reader hears the expiry as a card that asks appears, and again a minute before it.

use super::outside_text;
use super::{clock, decide_on_click, Decider, Decision};
use adw::prelude::*;
use fermix_client::chat_wire::proposals::{
    arrival, facts, item_header, joined_facts, last_minute_in, Card, Footer, CALL, CONFIRM, DENY,
    EXPIRED, LAST_MINUTE, RESULT, RUNNING, TITLE,
};
use fermix_client::chat_wire::protocol::Decision as Word;
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
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .hexpand(true)
            .accessible_role(gtk::AccessibleRole::Group)
            .css_classes(["card"])
            .build();
        widget.update_property(&[gtk::accessible::Property::Label(TITLE)]);
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(14)
            .margin_end(14)
            .build();
        inner.append(&lead(card));
        inner.append(&outside_text::block(CALL, None, &card.call, true));
        if let Some(item) = &card.item {
            let header = item_header(item, clock);
            let facts = joined_facts(item);
            inner.append(&outside_text::block(
                &header,
                facts.as_deref(),
                &item.text,
                false,
            ));
        }
        let footer = footer(card, decider);
        inner.append(&footer);
        widget.append(&inner);
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

/// One of Fermix's sentences, as written: wrapping, selectable, with no markup read.
fn line(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.set_text(text);
    label.set_wrap(true);
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
            if let Some(result) = result {
                footer.append(&outside_text::block(RESULT, None, result, true));
            }
        }
    }
    footer
}

/// Deny and Confirm, alike, at the card's end. `answered` is `None` while the card asks, else
/// whether the answer was a confirm: both buttons then wait, and a confirm says it is running.
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
    for (label, word) in [(DENY, Word::Deny), (CONFIRM, Word::Confirm)] {
        let button = gtk::Button::builder()
            .label(label)
            .sensitive(answered.is_none())
            .receives_default(false)
            .build();
        let decision = Decision::Proposal {
            id: card.id.clone(),
            decision: word,
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
