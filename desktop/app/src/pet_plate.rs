//! The mark at the head of the sidebar: the one-ink pet the Pet page shows
//! (marks/PROVENANCE.json, key app_icon_symbolic), in the text colour, standing
//! on one clear glass tile the app draws behind it: a rounded square a little
//! lighter than the window, lit along its top edge, casting a soft shadow. It
//! has no colour of its own: its shade is drawn in the text colour, so it suits
//! light and dark windows without any CSS.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

/// The widget's side: the tile and its shadow, well inside the rail's 56
/// pixels, so the header that centres it never widens the rail.
const SIDE: i32 = 36;
/// The tile's side and its corner radius.
const PANE: f32 = 28.0;
const RADIUS: f32 = 8.0;
/// The mark's side. It fills its square, so it covers two thirds of the tile.
const MARK_SIDE: i32 = 18;
const MARK: &str = "io.tezra.Fermix-symbolic";

glib::wrapper! {
    pub struct PetPlate(ObjectSubclass<imp::PetPlate>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PetPlate {
    pub fn new() -> PetPlate {
        let plate: PetPlate = glib::Object::new();
        plate.set_size_request(SIDE, SIDE);
        plate
    }
}

/// The glass's colours under a text colour: dark text is a light window, where
/// the glass is nearly white and its shade is the text colour, faint; light
/// text is a dark one, where the glass is barely there and the shadow deepens.
struct Glass {
    shadow: gdk::RGBA,
    top: gdk::RGBA,
    bottom: gdk::RGBA,
    lit: gdk::RGBA,
    shade: gdk::RGBA,
}

impl Glass {
    fn under(ink: &gdk::RGBA) -> Glass {
        let luminance = 0.2126 * ink.red() + 0.7152 * ink.green() + 0.0722 * ink.blue();
        let white = |alpha| gdk::RGBA::new(1.0, 1.0, 1.0, alpha);
        let black = |alpha| gdk::RGBA::new(0.0, 0.0, 0.0, alpha);
        let inked = |alpha| gdk::RGBA::new(ink.red(), ink.green(), ink.blue(), alpha);
        if luminance > 0.5 {
            return Glass {
                shadow: black(0.45),
                top: white(0.16),
                bottom: white(0.08),
                lit: white(0.24),
                shade: black(0.30),
            };
        }
        Glass {
            shadow: black(0.14),
            top: white(0.95),
            bottom: white(0.75),
            lit: white(1.0),
            shade: inked(0.14),
        }
    }
}

/// The tile: its shadow, then the glass, lighter at the top than the bottom,
/// then its rim, lit along the top edge and shaded along the others.
fn draw_glass(snapshot: &gtk::Snapshot, pane: &graphene::Rect, glass: &Glass) {
    let tile = gsk::RoundedRect::from_rect(*pane, RADIUS);
    snapshot.append_outset_shadow(&tile, &glass.shadow, 0.0, 1.0, 0.0, 4.0);
    snapshot.push_rounded_clip(&tile);
    snapshot.append_linear_gradient(
        pane,
        &graphene::Point::new(pane.x(), pane.y()),
        &graphene::Point::new(pane.x(), pane.y() + pane.height()),
        &[
            gsk::ColorStop::new(0.0, glass.top),
            gsk::ColorStop::new(1.0, glass.bottom),
        ],
    );
    snapshot.pop();
    let (lit, shade) = (glass.lit, glass.shade);
    snapshot.append_border(&tile, &[1.0; 4], &[lit, shade, shade, shade]);
}

/// The one-ink mark, `MARK_SIDE` square and centred on `centre`, filled with `ink`.
fn draw_mark(
    widget: &gtk::Widget,
    snapshot: &gtk::Snapshot,
    centre: &graphene::Point,
    ink: &gdk::RGBA,
) {
    let theme = gtk::IconTheme::for_display(&widget.display());
    let icon = theme.lookup_icon(
        MARK,
        &[],
        MARK_SIDE,
        widget.scale_factor(),
        widget.direction(),
        gtk::IconLookupFlags::empty(),
    );
    let side = MARK_SIDE as f32;
    snapshot.save();
    snapshot.translate(&graphene::Point::new(
        centre.x() - side / 2.0,
        centre.y() - side / 2.0,
    ));
    icon.snapshot_symbolic(snapshot, f64::from(side), f64::from(side), &[*ink]);
    snapshot.restore();
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PetPlate;

    #[glib::object_subclass]
    impl ObjectSubclass for PetPlate {
        const NAME: &'static str = "FermixPetPlate";
        type Type = super::PetPlate;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            // Decorative: the sidebar's page is named Fermix in words.
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for PetPlate {}

    impl WidgetImpl for PetPlate {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            let pane = graphene::Rect::new((width - PANE) / 2.0, (height - PANE) / 2.0, PANE, PANE);
            let ink = widget.color();
            draw_glass(snapshot, &pane, &Glass::under(&ink));
            let centre = graphene::Point::new(pane.x() + PANE / 2.0, pane.y() + PANE / 2.0);
            draw_mark(widget.upcast_ref(), snapshot, &centre, &ink);
        }
    }
}
