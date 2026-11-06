//! The one widget that draws outside text (M60 cues §5.5): what a model or a stranger wrote,
//! under a header in Fermix's words naming where it came from, set plain in a quiet inset. It is
//! never Markdown and never a link: the labels take their text with `set_text`, which reads no
//! markup, and GTK makes no link of a plain label. Past `OUTSIDE_LINES` it is cut, with Show all
//! putting the whole text in place; a screen reader always gets the whole text.
//!
//! Only a proposal's card uses it, and no title, toast, tray line or notification carries its
//! text.

use adw::prelude::*;
use fermix_client::chat_wire::proposals::{outside, OutsidePart, SHOW_ALL};

/// A quiet caption line in Fermix's words, or the item's own fields under it.
fn caption(text: &str) -> gtk::Label {
    let label = plain(text);
    label.add_css_class("caption");
    label.add_css_class("dimmed");
    label
}

/// A label holding `text` as written: wrapping, selectable, with no markup read.
fn plain(text: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.set_text(text);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_selectable(true);
    label
}

/// `part`'s header over its outside text, the item's labelled facts (outside text too) on a
/// line inside the inset above it. `code` sets the text in the monospace face, for a call's
/// arguments and what a tool answered. The card hands each part over whole and reads none of it.
pub fn block(part: &OutsidePart) -> gtk::Box {
    let header = part.header.as_str();
    let facts = part.facts.as_deref();
    let text = part.text.as_str();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    column.append(&caption(header));
    let inset = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    if let Some(facts) = facts {
        inset.append(&caption(facts));
    }
    let shown = outside(text);
    let body = plain(&shown.text);
    if part.code {
        body.add_css_class("monospace");
    }
    // The cut is what the eye sees; a screen reader hears the item whole, header first.
    let spoken = [Some(header), facts, Some(text)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    body.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    inset.append(&body);
    column.append(&gtk::Frame::builder().child(&inset).build());
    if shown.cut {
        column.append(&show_all(&body, text));
    }
    column
}

/// Show all: the whole text in place of the cut, once.
fn show_all(body: &gtk::Label, whole: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(SHOW_ALL)
        .halign(gtk::Align::Start)
        .css_classes(["flat", "caption"])
        .build();
    let (body, whole) = (body.downgrade(), whole.to_owned());
    button.connect_clicked(move |button| {
        if let Some(body) = body.upgrade() {
            body.set_text(&whole);
        }
        button.set_visible(false);
    });
    button
}
