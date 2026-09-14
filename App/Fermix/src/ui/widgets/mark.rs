//! The vendor mark, in the shared leading slot.
//!
//! A mark is the vendor's own file, drawn as the vendor publishes it. Nothing
//! here redraws, crops, recolours or invents one: the treatment for every key
//! is recorded in `resources/VendorMarks/PROVENANCE.json`, this module reads
//! that record, and `scripts/check_vendor_marks.sh` proves the record is true
//! of the bytes in the bundle.
//!
//! Three treatments, all of them the record's:
//!
//! - **A file per appearance.** A vendor that publishes two inks has its own
//!   choice drawn for the appearance in use. Never one file tinted, because
//!   that is the recolouring those vendors ask callers not to do.
//! - **One file, drawn as it is.** A vendor that publishes a single coloured
//!   mark. A file carrying its own ground fills the slot; a glyph on
//!   transparency is inset in it.
//! - **One single-ink file, drawn in the platform's own label ink.** The two
//!   vendors who publish one monochrome file and no variant. The geometry is
//!   the vendor's and the colour is the toolkit's: the mark is a mask and the
//!   ink comes from the symbolic colour the toolkit hands the paintable, so
//!   there is no colour in this application and the mark is legible in both
//!   appearances.
//!
//! A key with no record, and a recorded file this host cannot decode, both
//! render the record's declared no-mark treatment: the vendor's text name
//! beside a neutral symbolic icon. The name is the row's own title, which every
//! surface that draws a mark already carries, so the slot holds the symbol and
//! the row holds the name. A file that fails to load says so in the journal,
//! because a mark that silently disappeared is a defect nobody can see.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::gdk;
use gtk4::gdk_pixbuf;
use gtk4::glib;
use gtk4::graphene;
use gtk4::gsk;
use gtk4::subclass::prelude::*;
use libadwaita as adw;
use serde::Deserialize;

use crate::metrics;

/// The record, read from the bytes the gate hashes.
const PROVENANCE: &str = include_str!("../../../resources/VendorMarks/PROVENANCE.json");

/// Where the bundle serves the marks. The gate asserts the bundle and this
/// prefix agree.
const RESOURCE_PREFIX: &str = "/io/tezra/Fermix/marks";

/// The style class that gives the slot its width. Public because the gate that
/// measures a mark finds the slots by it.
pub const SLOT_CLASS: &str = "fermix-artwork-slot";

/// The inset a mark drawn on transparency gets inside the slot. A file that
/// carries its own ground takes none: insetting it would draw a border of the
/// vendor's own colour.
const INSET: i32 = metrics::SPACE_TIGHT;

/// Which roster a key belongs to. Two kinds may name one vendor: Discord and
/// Slack are each both a channel and a plugin, with different art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Provider,
    Channel,
    Plugin,
    Feature,
    MeetingPlatform,
    OauthClient,
}

impl MarkKind {
    /// How the record spells this kind.
    fn recorded(self) -> &'static str {
        match self {
            MarkKind::Provider => "provider",
            MarkKind::Channel => "channel",
            MarkKind::Plugin => "plugin",
            MarkKind::Feature => "feature",
            MarkKind::MeetingPlatform => "meeting_platform",
            MarkKind::OauthClient => "oauth_client",
        }
    }

    /// The neutral symbol a key with no drawable mark falls back to.
    ///
    /// Deliberately generic: a symbol resembling a vendor's own mark would be
    /// the fabrication the no-monogram rule exists to prevent.
    pub fn neutral_icon(self) -> &'static str {
        match self {
            MarkKind::Provider => "application-x-executable-symbolic",
            MarkKind::Channel => "mail-unread-symbolic",
            MarkKind::MeetingPlatform => "camera-web-symbolic",
            MarkKind::Plugin | MarkKind::Feature | MarkKind::OauthClient => {
                "application-x-addon-symbolic"
            }
        }
    }
}

/// One shipped file: the role it plays and where the bundle serves it.
#[derive(Debug, Clone, Deserialize)]
struct Asset {
    role: String,
    path: String,
}

/// One vendor's record.
#[derive(Debug, Clone, Deserialize)]
pub struct MarkRecord {
    pub key: String,
    kind: String,
    pub display_name: String,
    /// The name the row around this mark speaks.
    pub accessibility_label: String,
    treatment: String,
    #[serde(default)]
    plate: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Provenance {
    marks: Vec<MarkRecord>,
}

impl MarkRecord {
    /// Whether the file carries its own ground, which is what decides the
    /// inset.
    fn bleeds(&self) -> bool {
        self.plate.as_deref() == Some("bleed")
    }

    /// Whether this record ships a file at all.
    fn ships_a_file(&self) -> bool {
        self.treatment == "vendor_mark"
    }

    /// The file to draw in this appearance, and whether it is a single ink the
    /// platform colours.
    fn asset(&self, dark: bool) -> Option<(&str, bool)> {
        if !self.ships_a_file() {
            return None;
        }

        let wanted = if dark { "dark" } else { "light" };
        let chosen = self
            .assets
            .iter()
            .find(|asset| asset.role == wanted)
            .or_else(|| self.assets.iter().find(|asset| asset.role == "color"))
            .or_else(|| self.assets.iter().find(|asset| asset.role == "monochrome"))?;

        Some((chosen.path.as_str(), chosen.role == "monochrome"))
    }
}

/// Every record, parsed once.
fn records() -> &'static [MarkRecord] {
    static RECORDS: OnceLock<Vec<MarkRecord>> = OnceLock::new();
    &RECORDS.get_or_init(|| {
        let provenance: Provenance = serde_json::from_str(PROVENANCE)
            .expect("the vendored provenance record is part of the binary");
        provenance.marks
    })[..]
}

/// One record, by kind and key.
pub fn record(kind: MarkKind, key: &str) -> Option<&'static MarkRecord> {
    records()
        .iter()
        .find(|mark| mark.kind == kind.recorded() && mark.key == key)
}

/// An integration row's record.
///
/// Integrations draws registry plugins and the native driver features in one
/// list and a row carries only its name, so the name is read against the plugin
/// roster and then the feature roster, in that order.
pub fn integration(key: &str) -> Option<&'static MarkRecord> {
    record(MarkKind::Plugin, key).or_else(|| record(MarkKind::Feature, key))
}

/// A sign-in client's record. Google names a shared sign-in client rather than
/// one Google plugin, so the client roster is read first and the plugin roster
/// after it.
pub fn oauth_client(key: &str) -> Option<&'static MarkRecord> {
    record(MarkKind::OauthClient, key).or_else(|| record(MarkKind::Plugin, key))
}

/// The leading artwork slot: one mark, at the width every list aligns on.
pub fn slot(kind: MarkKind, key: &str) -> gtk::Widget {
    from_record(kind, record(kind, key))
}

/// The leading artwork slot for a record already resolved.
pub fn from_record(kind: MarkKind, mark: Option<&'static MarkRecord>) -> gtk::Widget {
    let widget = match mark {
        Some(mark) if mark.ships_a_file() => drawn(mark),
        _ => neutral(kind),
    };

    // The slot's width is the one the stylesheet declares and the metrics
    // module owns, so every list that draws a mark aligns on the same number.
    widget.add_css_class(SLOT_CLASS);
    widget.set_valign(gtk::Align::Center);
    widget.set_halign(gtk::Align::Center);

    if let Some(mark) = mark {
        widget.update_property(&[gtk::accessible::Property::Label(&mark.accessibility_label)]);
    }

    widget
}

/// The treatment for a key with no drawable mark.
fn neutral(kind: MarkKind) -> gtk::Widget {
    let icon = gtk::Image::from_icon_name(kind.neutral_icon());
    icon.set_pixel_size(metrics::ARTWORK_SLOT - INSET * 2);
    icon.add_css_class("dim-label");
    icon.upcast()
}

/// A recorded mark, drawn from the bundle and kept in step with the appearance.
fn drawn(mark: &'static MarkRecord) -> gtk::Widget {
    let holder = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();

    let apply = {
        let holder = holder.clone();
        move |dark: bool| {
            while let Some(child) = holder.first_child() {
                holder.remove(&child);
            }
            holder.append(&image(mark, dark));
        }
    };

    let manager = adw::StyleManager::default();
    apply(manager.is_dark());

    // The appearance can change while the window is open, and two vendors
    // publish one file per appearance. The handler is dropped with the widget
    // rather than left on the singleton.
    let handler = manager.connect_dark_notify(glib::clone!(
        #[strong]
        apply,
        move |manager| apply(manager.is_dark())
    ));
    let held = RefCell::new(Some(handler));
    holder.connect_destroy(move |_| {
        if let Some(handler) = held.borrow_mut().take() {
            adw::StyleManager::default().disconnect(handler);
        }
    });

    holder.upcast()
}

/// One drawing of one mark, or the neutral treatment where this host cannot
/// decode the file the record names.
fn image(mark: &'static MarkRecord, dark: bool) -> gtk::Widget {
    let Some((path, single_ink)) = mark.asset(dark) else {
        return neutral_for(mark);
    };

    let size = if mark.bleeds() {
        metrics::ARTWORK_SLOT
    } else {
        metrics::ARTWORK_SLOT - INSET * 2
    };

    let Some(texture) = texture(mark, path, size) else {
        return neutral_for(mark);
    };

    if single_ink {
        // The record permits exactly this for a vendor that publishes one ink
        // and no variant: the vendor's geometry in the platform's own label
        // colour, which the toolkit hands the paintable.
        return at_slot_size(&TintedMark::new(texture), size);
    }

    at_slot_size(&texture, size)
}

/// A mark, drawn at the size the slot gives it and no larger.
///
/// The bytes are decoded above the slot so a scaled display has pixels to draw
/// with, which makes the paintable's own size larger than the slot's. A widget
/// that takes its natural size from the paintable therefore draws the mark at
/// that larger number and pulls its row's height up with it, unevenly, because
/// only some rows carry a mark. An image with a pixel size is the one widget
/// whose natural size is the number it is given, and it keeps the vendor's own
/// aspect ratio while it scales.
fn at_slot_size(paintable: &impl IsA<gdk::Paintable>, size: i32) -> gtk::Widget {
    let image = gtk::Image::from_paintable(Some(paintable.as_ref()));
    image.set_pixel_size(size);
    image.upcast()
}

/// The bytes, decoded at the size they are drawn at.
///
/// Loaded rather than handed to a widget that would fail quietly: a mark this
/// host has no loader for is the record's no-mark treatment, and the reason it
/// took it belongs in the journal.
fn texture(mark: &MarkRecord, path: &str, size: i32) -> Option<gdk::Texture> {
    let resource = format!("{RESOURCE_PREFIX}/{path}");
    // Twice the slot, so a scaled display has pixels to draw with.
    let scaled = size * 2;

    match gdk_pixbuf::Pixbuf::from_resource_at_scale(&resource, scaled, scaled, true) {
        Ok(pixbuf) => Some(gdk::Texture::for_pixbuf(&pixbuf)),
        Err(error) => {
            glib::g_warning!(
                "fermix-desktop",
                "the {} mark at {resource} was not decoded on this host, so it renders as its \
                 name: {error}",
                mark.key
            );
            None
        }
    }
}

fn neutral_for(mark: &MarkRecord) -> gtk::Widget {
    neutral(kind_of(mark))
}

fn kind_of(mark: &MarkRecord) -> MarkKind {
    match mark.kind.as_str() {
        "provider" => MarkKind::Provider,
        "channel" => MarkKind::Channel,
        "feature" => MarkKind::Feature,
        "meeting_platform" => MarkKind::MeetingPlatform,
        "oauth_client" => MarkKind::OauthClient,
        _ => MarkKind::Plugin,
    }
}

mod imp {
    use super::*;

    /// A single-ink mark, painted in the colour the toolkit supplies.
    #[derive(Default)]
    pub struct TintedMark {
        pub source: RefCell<Option<gdk::Texture>>,
        pub width: Cell<i32>,
        pub height: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TintedMark {
        const NAME: &'static str = "FermixTintedMark";
        type Type = super::TintedMark;
        type Interfaces = (gdk::Paintable, gtk::SymbolicPaintable);
    }

    impl ObjectImpl for TintedMark {}

    impl PaintableImpl for TintedMark {
        fn intrinsic_width(&self) -> i32 {
            self.width.get()
        }

        fn intrinsic_height(&self) -> i32 {
            self.height.get()
        }

        /// Outside a symbolic context there is no colour to ask for, so the
        /// vendor's own file is drawn as it is.
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if let Some(texture) = self.source.borrow().as_ref() {
                texture.snapshot(snapshot, width, height);
            }
        }
    }

    impl SymbolicPaintableImpl for TintedMark {
        fn snapshot_symbolic(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            colors: &[gdk::RGBA],
        ) {
            let borrowed = self.source.borrow();
            let (Some(texture), Some(ink)) = (borrowed.as_ref(), colors.first()) else {
                return;
            };

            let Some(snapshot) = snapshot.downcast_ref::<gtk::Snapshot>() else {
                return;
            };

            // The mark is the mask and the platform's colour is the paint, so
            // the vendor's geometry is preserved exactly and no colour is this
            // application's.
            snapshot.push_mask(gsk::MaskMode::Alpha);
            texture.snapshot(snapshot.upcast_ref::<gdk::Snapshot>(), width, height);
            snapshot.pop();
            snapshot.append_color(
                ink,
                &graphene::Rect::new(0.0, 0.0, width as f32, height as f32),
            );
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// A single-ink mark drawn in the toolkit's own symbolic colour.
    pub struct TintedMark(ObjectSubclass<imp::TintedMark>)
        @implements gdk::Paintable, gtk::SymbolicPaintable;
}

impl TintedMark {
    /// One mask, from one texture.
    pub fn new(texture: gdk::Texture) -> Self {
        let mark: Self = glib::Object::new();
        mark.imp().width.set(texture.width());
        mark.imp().height.set(texture.height());
        mark.imp().source.replace(Some(texture));
        mark
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_parses_and_carries_every_roster() {
        let marks = records();
        assert!(
            marks.len() >= 30,
            "the vendored record is unexpectedly small"
        );

        for kind in [
            MarkKind::Provider,
            MarkKind::Channel,
            MarkKind::Plugin,
            MarkKind::Feature,
            MarkKind::MeetingPlatform,
            MarkKind::OauthClient,
        ] {
            assert!(
                marks.iter().any(|mark| mark.kind == kind.recorded()),
                "{kind:?} has no record"
            );
            assert!(
                !kind.neutral_icon().is_empty(),
                "{kind:?} has no neutral icon"
            );
        }
    }

    #[test]
    fn every_kind_has_its_own_neutral_icon_where_the_rosters_differ() {
        // Plugins, features and sign-in clients share one symbol on purpose:
        // they are one list to a person. Providers, channels and meeting
        // platforms each have their own.
        assert_ne!(
            MarkKind::Provider.neutral_icon(),
            MarkKind::Channel.neutral_icon()
        );
        assert_ne!(
            MarkKind::Provider.neutral_icon(),
            MarkKind::Plugin.neutral_icon()
        );
        assert_eq!(
            MarkKind::Plugin.neutral_icon(),
            MarkKind::Feature.neutral_icon()
        );
    }

    #[test]
    fn the_two_ordered_lookups_read_their_second_roster() {
        // A feature is not a plugin, and a sign-in client is not always its own
        // vendor; both sites read one roster and then the other.
        assert!(record(MarkKind::Plugin, "meetings").is_none());
        assert_eq!(
            integration("meetings").map(|mark| mark.key.as_str()),
            Some("meetings")
        );

        assert!(record(MarkKind::OauthClient, "notion").is_none());
        assert_eq!(
            oauth_client("notion").map(|mark| mark.key.as_str()),
            Some("notion")
        );
    }

    #[test]
    fn an_appearance_pair_resolves_to_the_vendors_own_file() {
        let github = integration("github").expect("github is recorded");
        let (light, _) = github.asset(false).expect("a light ink");
        let (dark, _) = github.asset(true).expect("a dark ink");

        assert_ne!(
            light, dark,
            "a vendor with two inks draws its own per appearance"
        );
    }

    #[test]
    fn a_single_ink_mark_is_drawn_in_the_platforms_colour() {
        let ollama = record(MarkKind::Provider, "ollama").expect("ollama is recorded");
        let (_, single_ink) = ollama.asset(false).expect("one ink");

        assert!(single_ink, "a monochrome role is coloured by the toolkit");
    }

    #[test]
    fn a_mark_that_carries_its_own_ground_is_never_inset() {
        let telegram = record(MarkKind::Channel, "telegram").expect("telegram is recorded");
        let openrouter = record(MarkKind::Provider, "openrouter").expect("openrouter is recorded");

        assert!(telegram.bleeds());
        assert!(!openrouter.bleeds());
    }

    #[test]
    fn a_key_with_no_record_has_no_mark() {
        assert!(record(MarkKind::Provider, "a_provider_from_the_future").is_none());
        assert!(integration("a_plugin_from_the_future").is_none());
    }
}
