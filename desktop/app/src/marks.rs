//! Vendor marks for every row that names a vendor: providers, plugins, channels,
//! meeting platforms, features and sign-in clients. Each file ships byte for byte
//! beside its macOS record in `marks/PROVENANCE.json`, and the table below draws
//! what that record says. A key without a record gets its kind's neutral symbolic
//! icon, never an invented mark.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use std::cell::{Cell, OnceCell};
use std::rc::Rc;

/// Every mark sits in the same square slot so row titles line up.
const SLOT: i32 = 32;
/// The neutral fallback is drawn smaller and dimmed, so it never reads as a vendor mark.
const NEUTRAL_SIZE: i32 = 20;
/// Clips a mark that carries its own ground to the slot's radius (style.css).
const BLEED_CLASS: &str = "plugin-mark-bleed";

/// Which roster a key belongs to. Two kinds may name one vendor with different
/// art: Discord and Slack are each both a channel and a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Provider,
    Plugin,
    Channel,
    MeetingPlatform,
    Feature,
    /// A sign-in client on the Integrations page, keyed by its provider.
    OAuthClient,
}

impl Kind {
    /// The symbolic icon a key of this kind gets when no mark ships for it. The
    /// GNOME 50 Adwaita theme has no plain chat bubble, so channels take the app's own.
    fn neutral_icon(self) -> &'static str {
        match self {
            Kind::Provider => "network-server-symbolic",
            Kind::Channel => "fermix-chat-symbolic",
            Kind::MeetingPlatform => "camera-video-symbolic",
            Kind::Plugin | Kind::Feature | Kind::OAuthClient => "application-x-addon-symbolic",
        }
    }
}

/// One shipped file: its path under `marks/`, as its record names it, and its bytes.
struct Asset {
    path: &'static str,
    bytes: &'static [u8],
}

/// Pairs a file's recorded path with its bytes, so the two cannot drift apart.
macro_rules! asset {
    ($path:literal) => {
        Asset {
            path: $path,
            bytes: include_bytes!(concat!("../marks/", $path)),
        }
    };
}

/// How a mark's record says it is drawn.
enum Ink {
    /// One official file in both appearances.
    File(Asset),
    /// The vendor's own file for each appearance, never one file tinted.
    Pair { light: Asset, dark: Asset },
    /// A single-ink file with no published variant, drawn in the label colour.
    Template(Asset),
}

impl Ink {
    fn asset(&self, dark: bool) -> &Asset {
        match self {
            Ink::File(asset) | Ink::Template(asset) => asset,
            Ink::Pair { dark: asset, .. } if dark => asset,
            Ink::Pair { light, .. } => light,
        }
    }
}

/// How a mark sits in its slot, as its record's `plate` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plate {
    /// The file carries its own ground: it fills the slot, clipped to its radius.
    Bleed,
    /// A glyph on transparency, drawn as is.
    Neutral,
}

struct Mark {
    kind: Kind,
    key: &'static str,
    ink: Ink,
    plate: Plate,
}

/// The mark table, one entry per record the app draws, in the record's words:
/// `tests` proves each entry, record and file agree.
const MARKS: &[Mark] = &[
    // Providers: each vendor's official service icon.
    Mark {
        kind: Kind::Provider,
        key: "anthropic",
        ink: Ink::File(asset!("providers/anthropic-color.png")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Provider,
        key: "mistral",
        ink: Ink::File(asset!("providers/mistral-color.svg")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Provider,
        key: "ollama",
        ink: Ink::Template(asset!("providers/ollama-mono.svg")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Provider,
        key: "openai",
        ink: Ink::File(asset!("providers/openai-color.svg")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Provider,
        key: "openai_codex",
        ink: Ink::File(asset!("providers/openai-color.svg")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Provider,
        key: "openrouter",
        ink: Ink::Pair {
            light: asset!("providers/openrouter-grape.svg"),
            dark: asset!("providers/openrouter-volt.svg"),
        },
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Provider,
        key: "venice",
        ink: Ink::Pair {
            light: asset!("providers/venice-deep-blue.svg"),
            dark: asset!("providers/venice-off-white.svg"),
        },
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Provider,
        key: "xai",
        ink: Ink::Template(asset!("providers/xai-mono.png")),
        plate: Plate::Neutral,
    },
    // Channels. `acp` (Editors) is a local protocol, not a vendor, so it has no mark.
    Mark {
        kind: Kind::Channel,
        key: "discord",
        ink: Ink::File(asset!("channels/discord-blurple.svg")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Channel,
        key: "signal",
        ink: Ink::File(asset!("channels/signal-ultramarine.svg")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Channel,
        key: "slack",
        ink: Ink::File(asset!("channels/slack-color.png")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Channel,
        key: "telegram",
        ink: Ink::File(asset!("channels/telegram-color.svg")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Channel,
        key: "whatsapp",
        ink: Ink::File(asset!("channels/whatsapp-color.webp")),
        plate: Plate::Bleed,
    },
    // Plugins, from the catalog the engine installs from and the set it ships inside itself.
    Mark {
        kind: Kind::Plugin,
        key: "agentmail",
        ink: Ink::File(asset!("plugins/agentmail-color.png")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Plugin,
        key: "discord",
        ink: Ink::File(asset!("plugins/discord-color.svg")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Plugin,
        key: "github",
        ink: Ink::Pair {
            light: asset!("plugins/github-black.svg"),
            dark: asset!("plugins/github-white.svg"),
        },
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Plugin,
        key: "gmail",
        ink: Ink::File(asset!("plugins/gmail-color.png")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Plugin,
        key: "google_calendar",
        ink: Ink::File(asset!("plugins/google-calendar-color.png")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Plugin,
        key: "google_drive",
        ink: Ink::File(asset!("plugins/google-drive-color.png")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Plugin,
        key: "notion",
        ink: Ink::File(asset!("plugins/notion-color.svg")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Plugin,
        key: "obsidian",
        ink: Ink::File(asset!("plugins/obsidian-color.png")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Plugin,
        key: "slack",
        ink: Ink::File(asset!("plugins/slack-color.svg")),
        plate: Plate::Bleed,
    },
    Mark {
        kind: Kind::Plugin,
        key: "tesla",
        ink: Ink::File(asset!("plugins/tesla-color.png")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Plugin,
        key: "x",
        ink: Ink::File(asset!("plugins/x-color.svg")),
        plate: Plate::Bleed,
    },
    // The native features the Integrations page lists, in Fermix's own art.
    Mark {
        kind: Kind::Feature,
        key: "computer_use",
        ink: Ink::File(asset!("features/computer-use-color.svg")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::Feature,
        key: "meetings",
        ink: Ink::File(asset!("features/meetings-color.svg")),
        plate: Plate::Neutral,
    },
    // The platforms the Meetings pane is sectioned by.
    Mark {
        kind: Kind::MeetingPlatform,
        key: "google_meet",
        ink: Ink::File(asset!("meeting_platforms/google-meet-color.png")),
        plate: Plate::Neutral,
    },
    Mark {
        kind: Kind::MeetingPlatform,
        key: "zoom",
        ink: Ink::File(asset!("meeting_platforms/zoom-color.png")),
        plate: Plate::Neutral,
    },
    // Google names a shared sign-in client rather than one Google plugin.
    Mark {
        kind: Kind::OAuthClient,
        key: "google",
        ink: Ink::Pair {
            light: asset!("oauth_clients/google-light.png"),
            dark: asset!("oauth_clients/google-dark.png"),
        },
        plate: Plate::Bleed,
    },
];

/// The mark for this key, if one ships. A sign-in client without a record of its
/// own takes the plugin mark of the same name (GitHub, Notion, Tesla, X), which is
/// how the macOS table resolves client rows too.
fn lookup(kind: Kind, key: &str) -> Option<&'static Mark> {
    let find = |kind: Kind| MARKS.iter().find(|m| m.kind == kind && m.key == key);
    match kind {
        Kind::OAuthClient => find(kind).or_else(|| find(Kind::Plugin)),
        _ => find(kind),
    }
}

/// A 32 px image for a row's prefix: the vendor's mark for this key, following
/// light and dark, or the kind's neutral icon when none ships. It is decorative:
/// the row around it carries the name.
pub fn mark(kind: Kind, key: &str) -> gtk::Image {
    assert!(!key.is_empty(), "a mark is keyed by its vendor");
    let image = gtk::Image::builder()
        .width_request(SLOT)
        .height_request(SLOT)
        .valign(gtk::Align::Center)
        .overflow(gtk::Overflow::Hidden)
        .build();
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    let style = adw::StyleManager::default();
    paint(&image, kind, key, style.is_dark());
    let key = key.to_owned();
    let weak = image.downgrade();
    let handler = style.connect_dark_notify(move |style| {
        if let Some(image) = weak.upgrade() {
            paint(&image, kind, &key, style.is_dark());
        }
    });
    // The style manager outlives every row, so the handler goes with the image.
    let handler = Cell::new(Some(handler));
    image.connect_destroy(move |_| {
        if let Some(handler) = handler.take() {
            adw::StyleManager::default().disconnect(handler);
        }
    });
    image
}

fn paint(image: &gtk::Image, kind: Kind, key: &str, dark: bool) {
    let Some(mark) = lookup(kind, key) else {
        return paint_neutral(image, kind);
    };
    let asset = mark.ink.asset(dark);
    let paintable = match decode(asset) {
        Ok(paintable) => paintable,
        Err(e) => {
            glib::g_warning!("fermix", "the {} mark did not decode: {e}", asset.path);
            return paint_neutral(image, kind);
        }
    };
    let paintable = match mark.ink {
        Ink::Template(_) => TemplateMark::new(&paintable).upcast(),
        Ink::File(_) | Ink::Pair { .. } => paintable,
    };
    image.remove_css_class("dim-label");
    if mark.plate == Plate::Bleed {
        image.add_css_class(BLEED_CLASS);
    } else {
        image.remove_css_class(BLEED_CLASS);
    }
    image.set_pixel_size(SLOT);
    image.set_paintable(Some(&paintable));
}

fn paint_neutral(image: &gtk::Image, kind: Kind) {
    image.remove_css_class(BLEED_CLASS);
    image.add_css_class("dim-label");
    image.set_pixel_size(NEUTRAL_SIZE);
    image.set_icon_name(Some(kind.neutral_icon()));
}

/// GTK's own SVG renderer draws an SVG: it needs no image loader and stays sharp
/// at any size. Where it reports a feature it does not implement, the file goes to
/// the image loader instead, the rule GTK's icon loader follows. A raster always does.
fn decode(asset: &Asset) -> Result<gdk::Paintable, glib::Error> {
    let bytes = glib::Bytes::from_static(asset.bytes);
    if asset.path.ends_with(".svg") {
        if let Some(svg) = native_svg(asset.path, &bytes) {
            return Ok(svg.upcast());
        }
    }
    gdk::Texture::from_bytes(&bytes).map(|texture| texture.upcast())
}

/// The SVG as GTK draws it, or `None` when GTK lacks a feature the file uses.
fn native_svg(path: &str, bytes: &glib::Bytes) -> Option<gtk::Svg> {
    let svg = gtk::Svg::new();
    let lacking = Rc::new(Cell::new(false));
    let seen = lacking.clone();
    let path = path.to_owned();
    let handler = svg.connect_error(move |_, e| {
        // An attribute GTK skips (role, aria-label) changes nothing drawn; a missing feature does.
        glib::g_debug!("fermix", "{path}: {e}");
        seen.set(seen.get() || e.matches(gtk::SvgError::NotImplemented));
    });
    svg.load_from_bytes(bytes);
    svg.disconnect(handler);
    (!lacking.get()).then_some(svg)
}

glib::wrapper! {
    /// A single-ink mark in the label colour: the file's own alpha, filled with the
    /// foreground colour GTK hands a symbolic paintable, so it reads on light and dark.
    pub struct TemplateMark(ObjectSubclass<template::TemplateMark>)
        @implements gdk::Paintable, gtk::SymbolicPaintable;
}

impl TemplateMark {
    fn new(ink: &gdk::Paintable) -> TemplateMark {
        let mark: TemplateMark = glib::Object::new();
        mark.imp()
            .ink
            .set(ink.clone())
            .expect("a new template mark has no ink yet");
        mark
    }
}

mod template {
    use super::{gdk, glib, graphene, gsk, OnceCell};
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct TemplateMark {
        pub(super) ink: OnceCell<gdk::Paintable>,
    }

    impl TemplateMark {
        fn ink(&self) -> &gdk::Paintable {
            self.ink
                .get()
                .expect("a template mark is built with its ink")
        }

        /// The file's own alpha, filled with the foreground colour, which GTK
        /// passes first among the symbolic colours.
        fn draw_in(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, colors: &[gdk::RGBA]) {
            let snapshot = snapshot
                .downcast_ref::<gtk::Snapshot>()
                .expect("GTK draws paintables into a GtkSnapshot");
            let ink = colors
                .first()
                .expect("GTK passes the foreground colour first");
            snapshot.push_mask(gsk::MaskMode::Alpha);
            self.ink().snapshot(snapshot, width, height);
            snapshot.pop();
            let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
            snapshot.append_color(ink, &bounds);
            snapshot.pop();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TemplateMark {
        const NAME: &'static str = "FermixTemplateMark";
        type Type = super::TemplateMark;
        type Interfaces = (gdk::Paintable, gtk::SymbolicPaintable);
    }

    impl ObjectImpl for TemplateMark {}

    impl PaintableImpl for TemplateMark {
        fn intrinsic_width(&self) -> i32 {
            self.ink().intrinsic_width()
        }

        fn intrinsic_height(&self) -> i32 {
            self.ink().intrinsic_height()
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            self.ink().intrinsic_aspect_ratio()
        }

        /// Outside a widget there is no label colour, so the file draws as it is.
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            self.ink().snapshot(snapshot, width, height);
        }
    }

    impl SymbolicPaintableImpl for TemplateMark {
        fn snapshot_symbolic(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            colors: &[gdk::RGBA],
        ) {
            self.draw_in(snapshot, width, height, colors);
        }

        /// A widget draws a symbolic paintable through this one, and the interface
        /// has no default to chain up to. The weight thickens an icon's strokes; a
        /// vendor's file has none the weight is meant for.
        fn snapshot_with_weight(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            colors: &[gdk::RGBA],
            _weight: f64,
        ) {
            self.draw_in(snapshot, width, height, colors);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::Path;

    const PROVENANCE: &str = include_str!("../marks/PROVENANCE.json");

    fn records() -> Vec<Value> {
        let provenance: Value = serde_json::from_str(PROVENANCE).expect("PROVENANCE.json is JSON");
        provenance["marks"].as_array().expect("records").clone()
    }

    /// The record's name for a kind; `None` for a record that is not a vendor mark
    /// (the pet, the wordmark, the app icons).
    fn kind_named(name: &str) -> Option<Kind> {
        let kind = match name {
            "provider" => Kind::Provider,
            "plugin" => Kind::Plugin,
            "channel" => Kind::Channel,
            "meeting_platform" => Kind::MeetingPlatform,
            "feature" => Kind::Feature,
            "oauth_client" => Kind::OAuthClient,
            _ => return None,
        };
        Some(kind)
    }

    fn record(kind: Kind, key: &str) -> Value {
        let found = records().into_iter().find(|r| {
            kind_named(r["kind"].as_str().unwrap_or_default()) == Some(kind) && r["key"] == key
        });
        found.unwrap_or_else(|| panic!("{kind:?} {key} is drawn but has no record"))
    }

    /// The roles a record gives the files of each treatment.
    fn roles(ink: &'static Ink) -> Vec<(&'static str, &'static Asset)> {
        match ink {
            Ink::File(asset) => vec![("color", asset)],
            Ink::Template(asset) => vec![("monochrome", asset)],
            Ink::Pair { light, dark } => vec![("light", light), ("dark", dark)],
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        glib::compute_checksum_for_data(glib::ChecksumType::Sha256, bytes)
            .expect("sha256 is a glib checksum")
            .to_string()
    }

    #[test]
    fn each_drawn_file_is_its_record_byte_for_byte() {
        for mark in MARKS {
            let record = record(mark.kind, mark.key);
            let plate = match mark.plate {
                Plate::Bleed => "bleed",
                Plate::Neutral => "neutral",
            };
            assert_eq!(record["plate"], plate, "{:?} {}", mark.kind, mark.key);
            for (role, asset) in roles(&mark.ink) {
                let assets = record["assets"].as_array().expect("assets");
                let recorded = assets
                    .iter()
                    .find(|a| a["role"] == role && a["path"] == asset.path);
                let recorded = recorded.unwrap_or_else(|| panic!("{} is not {role}", asset.path));
                assert_eq!(recorded["sha256"], sha256(asset.bytes), "{}", asset.path);
            }
        }
    }

    #[test]
    fn each_vendor_record_is_drawn() {
        for record in records() {
            let Some(kind) = kind_named(record["kind"].as_str().expect("kind")) else {
                continue;
            };
            let key = record["key"].as_str().expect("key");
            let drawn = MARKS.iter().any(|m| m.kind == kind && m.key == key);
            assert!(drawn, "{kind:?} {key} has a record but is not drawn");
        }
    }

    /// Every recorded file as (path from `marks/`, sha256). A path that leaves
    /// `marks/` (`../`) names a file elsewhere in the workspace: the pet's layers,
    /// the app icons.
    fn recorded_files() -> Vec<(String, String)> {
        records()
            .iter()
            .flat_map(|r| r["assets"].as_array().cloned().unwrap_or_default())
            .map(|a| {
                (
                    a["path"].as_str().unwrap().to_owned(),
                    a["sha256"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// Nothing ships without a record, and every recorded file ships.
    #[test]
    fn each_shipped_file_is_recorded() {
        let recorded: HashMap<String, String> = recorded_files()
            .into_iter()
            .filter(|(path, _)| !path.starts_with("../"))
            .collect();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("marks");
        let dirs = std::fs::read_dir(&root)
            .expect("marks/")
            .map(|d| d.expect("entry").path());
        let mut shipped = 0;
        for dir in dirs.filter(|d| d.is_dir()) {
            for file in std::fs::read_dir(&dir).expect("a kind's directory") {
                let file = file.expect("entry").path();
                let path = file
                    .strip_prefix(&root)
                    .expect("under marks/")
                    .to_string_lossy()
                    .into_owned();
                let bytes = std::fs::read(&file).expect("readable");
                assert_eq!(recorded.get(&path), Some(&sha256(&bytes)), "{path}");
                shipped += 1;
            }
        }
        assert_eq!(
            shipped,
            recorded.len(),
            "a recorded file is missing from marks/"
        );
    }

    /// A recorded file outside `marks/` is there, with the bytes its record pins.
    #[test]
    fn each_recorded_file_outside_marks_is_its_record() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("marks");
        let outside: Vec<(String, String)> = recorded_files()
            .into_iter()
            .filter(|(path, _)| path.starts_with("../"))
            .collect();
        assert!(
            !outside.is_empty(),
            "the pet and the app icons are recorded"
        );
        for (path, sha) in outside {
            let bytes = std::fs::read(root.join(&path)).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(sha256(&bytes), sha, "{path}");
        }
    }

    #[test]
    fn a_sign_in_client_without_its_own_record_takes_the_plugin_mark() {
        let google = lookup(Kind::OAuthClient, "google").map(|m| (m.kind, m.ink.asset(true).path));
        assert_eq!(
            google,
            Some((Kind::OAuthClient, "oauth_clients/google-dark.png"))
        );
        for key in ["github", "notion", "tesla", "x"] {
            let found = lookup(Kind::OAuthClient, key).map(|m| (m.kind, m.key));
            assert_eq!(found, Some((Kind::Plugin, key)));
        }
        assert!(
            lookup(Kind::Plugin, "google").is_none(),
            "only clients fall back"
        );
    }

    #[test]
    fn a_key_without_a_record_has_no_mark() {
        assert!(
            lookup(Kind::Channel, "acp").is_none(),
            "Editors has no vendor"
        );
        assert!(lookup(Kind::Provider, "a_new_provider").is_none());
        assert!(lookup(Kind::Feature, "computer_history").is_none());
    }

    #[test]
    fn one_vendor_in_two_kinds_draws_each_kinds_own_art() {
        let path = |kind| lookup(kind, "discord").map(|m| m.ink.asset(false).path);
        assert_eq!(path(Kind::Channel), Some("channels/discord-blurple.svg"));
        assert_eq!(path(Kind::Plugin), Some("plugins/discord-color.svg"));
    }

    #[test]
    fn the_single_ink_marks_are_templates() {
        for key in ["xai", "ollama"] {
            let mark = lookup(Kind::Provider, key).expect("a provider mark");
            assert!(matches!(mark.ink, Ink::Template(_)), "{key}");
        }
    }

    #[test]
    fn every_png_decodes() {
        let assets = MARKS.iter().flat_map(|m| roles(&m.ink));
        for (_, asset) in assets.filter(|(_, a)| a.path.ends_with(".png")) {
            let texture = gdk::Texture::from_bytes(&glib::Bytes::from_static(asset.bytes));
            let texture = texture.unwrap_or_else(|e| panic!("{}: {e}", asset.path));
            assert!(
                texture.width() >= 72 && texture.height() >= 72,
                "{}",
                asset.path
            );
        }
    }
}
