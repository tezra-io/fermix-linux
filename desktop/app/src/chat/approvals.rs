//! The owner-approval cards above the composer: each card's question, its detail, and Approve
//! and Deny, which send the card's own route. The token is never drawn. A card whose answer is
//! on its way keeps its buttons insensitive, so two clicks cannot send two answers. Each button
//! decides by its own click, holding the card's id, never through an action (M60 cues §6 rule 3).
//! An approval the daemon sent that could not be read is one row of words, with no buttons
//! (`chat_wire::unshown`).

use super::{decide_on_click, Decider, Decision};
use adw::prelude::*;
use fermix_client::chat_wire::timeline::{self, Chat};
use fermix_client::chat_wire::unshown::{self, Asks, APPROVAL};
use std::cell::RefCell;

/// One drawn card: its id, its words and whether its answer is on its way.
type Drawn = (String, String, Option<String>, bool);

pub struct Cards {
    /// The cards, as wide as the composer under them.
    pub widget: adw::Clamp,
    list: gtk::ListBox,
    /// The cards drawn, and whether the placeholder of unreadable ones is.
    drawn: RefCell<(Vec<Drawn>, bool)>,
    decider: Decider,
}

impl Cards {
    pub fn new(decider: &Decider) -> Cards {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .margin_start(18)
            .margin_end(18)
            .margin_bottom(12)
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Waiting for your answer")]);
        let widget = adw::Clamp::builder()
            .maximum_size(760)
            .child(&list)
            .visible(false)
            .build();
        Cards {
            widget,
            list,
            drawn: RefCell::default(),
            decider: decider.clone(),
        }
    }

    /// Redraws the cards when they changed; deltas, tens a second, leave them alone.
    pub fn show(&self, chat: &Chat) {
        let cards: Vec<Drawn> = chat
            .approvals
            .iter()
            .map(|held| {
                let card = &held.approval;
                let answering = timeline::answering(chat, &card.approval_id);
                (
                    card.approval_id.clone(),
                    card.text.clone(),
                    card.detail.clone(),
                    answering,
                )
            })
            .collect();
        let unreadable = unshown::waiting(&chat.unshown, Asks::Approval);
        let drawn = (cards, unreadable);
        if *self.drawn.borrow() == drawn {
            return;
        }
        self.list.remove_all();
        for card in &drawn.0 {
            self.list.append(&card_row(card, &self.decider));
        }
        if unreadable {
            self.list.append(&unshown_row());
        }
        self.widget.set_visible(!drawn.0.is_empty() || unreadable);
        self.drawn.replace(drawn);
    }
}

/// The approvals this app could not read, said in one row: nothing here can answer them.
fn unshown_row() -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(APPROVAL);
    row.set_title_lines(0);
    row.set_activatable(false);
    row
}

fn card_row((id, text, detail, answering): &Drawn, decider: &Decider) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(text);
    if let Some(detail) = detail {
        row.set_subtitle(detail);
    }
    row.set_title_lines(0);
    let deny = answer_button("Deny", &[]);
    let approve = answer_button("Approve", &["suggested-action"]);
    deny.set_sensitive(!answering);
    approve.set_sensitive(!answering);
    let answer = |approve| Decision::Approval {
        id: id.clone(),
        approve,
    };
    decide_on_click(&deny, decider, answer(false));
    decide_on_click(&approve, decider, answer(true));
    row.add_suffix(&deny);
    row.add_suffix(&approve);
    row
}

fn answer_button(label: &str, classes: &[&str]) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build();
    for class in classes {
        button.add_css_class(class);
    }
    button
}
