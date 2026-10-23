//! The owner-approval cards above the composer: each card's question, its detail, and Approve
//! and Deny, which send the card's own route. The token is never drawn. A card whose answer is
//! on its way keeps its buttons insensitive, so two clicks cannot send two answers.

use adw::prelude::*;
use fermix_client::chat_wire::timeline::{self, Chat};
use std::cell::RefCell;

/// One drawn card: its id, its words and whether its answer is on its way.
type Drawn = (String, String, Option<String>, bool);

pub struct Cards {
    /// The cards, as wide as the composer under them.
    pub widget: adw::Clamp,
    list: gtk::ListBox,
    drawn: RefCell<Vec<Drawn>>,
}

impl Cards {
    pub fn new() -> Cards {
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
        if *self.drawn.borrow() == cards {
            return;
        }
        self.list.remove_all();
        for card in &cards {
            self.list.append(&card_row(card));
        }
        self.widget.set_visible(!cards.is_empty());
        self.drawn.replace(cards);
    }
}

fn card_row((id, text, detail, answering): &Drawn) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(text);
    if let Some(detail) = detail {
        row.set_subtitle(detail);
    }
    row.set_title_lines(0);
    let deny = answer_button("Deny", "win.chat-deny", id, &[]);
    let approve = answer_button("Approve", "win.chat-approve", id, &["suggested-action"]);
    deny.set_sensitive(!answering);
    approve.set_sensitive(!answering);
    row.add_suffix(&deny);
    row.add_suffix(&approve);
    row
}

fn answer_button(label: &str, action: &str, id: &str, classes: &[&str]) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .action_name(action)
        .action_target(&id.to_variant())
        .build();
    for class in classes {
        button.add_css_class(class);
    }
    button
}
