//! The Fermix wordmark, where the app names itself: the head of the sidebar. It is
//! the macOS app's `Resources/Wordmark/fermix-wordmark.svg`, byte for byte, drawn
//! as macOS draws it (`FermixWordmark.swift`): the letters in the text colour, so
//! they follow light, dark and high contrast, and the two eye-dots in the file's
//! own blue in both.
//!
//! The letters are `currentColor`, which GTK's SVG renderer resolves to black, and
//! a template fill (`marks.rs`) would flatten the dots into the letters' colour.
//! So the file is drawn with its root `color` set to the text colour GTK hands a
//! symbolic paintable, as a web page sets it around an inline SVG.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};

/// The wordmark file, byte for byte (marks/PROVENANCE.json, key wordmark).
const SVG: &str = include_str!("../marks/wordmark/fermix-wordmark.svg");
/// The file's viewBox: the 384 by 100 glyphs inside a margin.
const VIEW_BOX: (i32, i32) = (396, 116);
/// What the drawing says, for anyone who cannot see it.
const NAME: &str = "Fermix";

/// The wordmark, `height` pixels tall with the file's margin, as a picture that
/// speaks as "Fermix".
pub fn picture(height: i32) -> gtk::Picture {
    // Centred, so a taller slot (a header bar's) does not scale it up.
    let picture = gtk::Picture::builder()
        .paintable(&Wordmark::new(height))
        .can_shrink(false)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .alternative_text(NAME)
        .build();
    picture.update_property(&[gtk::accessible::Property::Label(NAME)]);
    picture
}

/// The file with its root colour set, so its `currentColor` letters draw in `ink`.
fn tinted(ink: &str) -> String {
    let rest = SVG
        .strip_prefix("<svg")
        .expect("the wordmark file starts with its <svg> element");
    format!("<svg color=\"{ink}\"{rest}")
}

/// The width that keeps the file's proportions at `height`, to the nearest pixel.
fn width_for(height: i32) -> i32 {
    (height * VIEW_BOX.0 + VIEW_BOX.1 / 2) / VIEW_BOX.1
}

/// The file drawn with its letters in `ink`.
fn load(ink: &gdk::RGBA) -> gtk::Svg {
    let svg = gtk::Svg::new();
    let handler = svg.connect_error(|_, e| {
        // GTK skips the file's role and aria-label, which change nothing drawn.
        if e.matches(gtk::SvgError::NotImplemented) {
            glib::g_warning!("fermix", "the wordmark does not draw as published: {e}");
        } else {
            glib::g_debug!("fermix", "the wordmark: {e}");
        }
    });
    svg.load_from_bytes(&glib::Bytes::from_owned(tinted(&ink.to_string())));
    svg.disconnect(handler);
    svg
}

glib::wrapper! {
    /// The wordmark at a height. A widget drawing it hands it the text colour.
    pub struct Wordmark(ObjectSubclass<imp::Wordmark>)
        @implements gdk::Paintable, gtk::SymbolicPaintable;
}

impl Wordmark {
    fn new(height: i32) -> Wordmark {
        assert!(height > 0, "the wordmark needs a height, got {height}");
        let mark: Wordmark = glib::Object::new();
        mark.imp().height.set(height);
        mark
    }
}

mod imp {
    use super::{gdk, glib, load, width_for, Cell, RefCell};
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Wordmark {
        pub(super) height: Cell<i32>,
        /// The file as last drawn and the colour of its letters: the same colour
        /// draws it again, a new one loads it again.
        drawn: RefCell<Option<(gdk::RGBA, gtk::Svg)>>,
    }

    impl Wordmark {
        fn draw(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, ink: &gdk::RGBA) {
            let mut drawn = self.drawn.borrow_mut();
            if drawn.as_ref().is_none_or(|(drawn_in, _)| drawn_in != ink) {
                *drawn = Some((*ink, load(ink)));
            }
            let (_, svg) = drawn.as_ref().expect("loaded above");
            svg.snapshot(snapshot, width, height);
        }
    }

    /// GTK passes the text colour first among the symbolic colours.
    fn foreground(colors: &[gdk::RGBA]) -> &gdk::RGBA {
        colors
            .first()
            .expect("GTK passes the foreground colour first")
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Wordmark {
        const NAME: &'static str = "FermixWordmark";
        type Type = super::Wordmark;
        type Interfaces = (gdk::Paintable, gtk::SymbolicPaintable);
    }

    impl ObjectImpl for Wordmark {}

    impl PaintableImpl for Wordmark {
        fn intrinsic_width(&self) -> i32 {
            width_for(self.height.get())
        }

        fn intrinsic_height(&self) -> i32 {
            self.height.get()
        }

        /// Outside a widget there is no text colour: the letters are black, as the
        /// file draws them.
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            self.draw(snapshot, width, height, &gdk::RGBA::BLACK);
        }
    }

    impl SymbolicPaintableImpl for Wordmark {
        fn snapshot_symbolic(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            colors: &[gdk::RGBA],
        ) {
            self.draw(snapshot, width, height, foreground(colors));
        }

        /// A widget draws a symbolic paintable through this one. The weight
        /// thickens an icon's strokes; the wordmark is filled shapes.
        fn snapshot_with_weight(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            colors: &[gdk::RGBA],
            _weight: f64,
        ) {
            self.draw(snapshot, width, height, foreground(colors));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn the_letters_take_the_colour_given_and_nothing_else_changes() {
        let svg = tinted("rgb(1,2,3)");
        assert!(svg.starts_with("<svg color=\"rgb(1,2,3)\""), "{svg}");
        assert_eq!(svg.replacen(" color=\"rgb(1,2,3)\"", "", 1), SVG);
    }

    /// The letters are the one `currentColor` fill; the eye-dots are the brand blue.
    #[test]
    fn only_the_letters_follow_the_colour() {
        assert_eq!(SVG.matches("currentColor").count(), 1);
        assert_eq!(SVG.matches("fill=\"#2b5cff\"").count(), 2);
    }

    #[test]
    fn it_keeps_the_files_proportions() {
        assert_eq!(width_for(18), 61);
        assert_eq!(width_for(116), 396);
    }

    #[test]
    fn it_ships_byte_for_byte_as_recorded() {
        let provenance: Value =
            serde_json::from_str(include_str!("../marks/PROVENANCE.json")).expect("JSON");
        let records = provenance["marks"].as_array().expect("records");
        let record = records
            .iter()
            .find(|r| r["kind"] == "wordmark")
            .expect("the wordmark has a record");
        let asset = &record["assets"][0];
        assert_eq!(asset["path"], "wordmark/fermix-wordmark.svg");
        let sha = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, SVG.as_bytes())
            .expect("sha256");
        assert_eq!(asset["sha256"], sha.as_str());
        assert_eq!(
            asset["source_sha256"],
            sha.as_str(),
            "the macOS file's own bytes"
        );
    }
}
