//! The text pill.
//!
//! Status is never colour alone, so a Doctor row leads with a letter and says
//! the word beside it. The pill is a label in the toolkit's own card style: the
//! shape, the ground and the radius are libadwaita's, and the letter and the
//! accessible name are the catalogue's.

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::copy::{self, Key};
use crate::management::types::CheckStatus;
use crate::models::doctor::{status_pill as pill_key, status_word};

/// The pill for one status, named for a screen reader by its word.
pub fn status_pill(status: CheckStatus) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(copy::text(pill_key(status)))
        .width_chars(1)
        .valign(gtk::Align::Center)
        .build();

    label.add_css_class("card");
    label.add_css_class("fermix-pill");
    label.add_css_class("monospace");
    label.update_property(&[gtk::accessible::Property::Label(&copy::text(status_word(
        status,
    )))]);

    label
}

/// The pill for a state the product itself names, so a summary line and a row
/// use one shape.
pub fn word_pill(word: Key) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(copy::text(word))
        .valign(gtk::Align::Center)
        .build();
    label.add_css_class("card");
    label.add_css_class("fermix-pill");
    label
}
