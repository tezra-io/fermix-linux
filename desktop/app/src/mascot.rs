//! The voice mascot: the macOS pet's layers, drawn with the motion that
//! `fermix_client::mascot` computes (spec §2.2). A frame-clock tick drives it
//! only while the widget is mapped, so a hidden page or window costs nothing.
//! With animations off it holds still in its current expression.

use fermix_client::mascot::{self, Expression, Fade, Plate, Pose, Transition, ART, STAGE};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

const RESOURCES: &str = "/io/tezra/Fermix/pet";
/// The accent glow's strength, out of a call and in one (M `PetView.swift:74-78`).
const GLOW: f32 = 0.18;
const GLOW_IN_CALL: f32 = 0.34;
/// The glow's radius and how far below the centre it sits, in default points:
/// it sits 6 pt low, as the macOS shadow does (M `PetView.swift:19`), and ends
/// at the stage's bottom edge. It holds its strength out to `GLOW_SOLID` of the
/// radius, mostly hidden by the body, then fades to nothing.
const GLOW_RADIUS: f32 = 52.0;
const GLOW_DROP: f32 = 6.0;
const GLOW_SOLID: f32 = 0.6;

/// The mascot widget and its controls. Clones share the one widget.
#[derive(Clone)]
pub struct Mascot {
    pub widget: gtk::Widget,
    canvas: Canvas,
}

impl Mascot {
    /// A mascot on a `width` by `height` stage. At 132 by 116, the macOS size,
    /// the mascot is 116 by 108; it scales with the stage.
    pub fn new(width: i32, height: i32) -> Mascot {
        assert!(
            width > 0 && height > 0,
            "the mascot's stage needs a size, got {width}x{height}"
        );
        let canvas: Canvas = glib::Object::new();
        canvas.set_size_request(width, height);
        canvas.imp().load_layers();
        Mascot {
            widget: canvas.clone().upcast(),
            canvas,
        }
    }

    /// Crossfades to `expression`; the expression already on show restarts nothing.
    pub fn set_expression(&self, expression: Expression) {
        let imp = self.canvas.imp();
        let now = imp.now.get();
        imp.transition
            .set(imp.transition.get().change(expression, now));
        self.canvas.queue_draw();
    }

    /// The smoothed output level, 0 to 1, which swells the speaking face.
    pub fn set_level(&self, level: f32) {
        assert!(
            (0.0..=1.0).contains(&level),
            "output level must be 0 to 1, got {level}"
        );
        self.canvas.imp().level.set(level);
    }

    /// A call makes the glow stronger.
    pub fn set_in_call(&self, in_call: bool) {
        self.canvas.imp().in_call.set(in_call);
        self.canvas.queue_draw();
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

mod imp {
    use super::*;

    /// The signal handlers a mapped mascot holds on objects that outlive it.
    struct Watches {
        settings: gtk::Settings,
        animations: glib::SignalHandlerId,
        accent: glib::SignalHandlerId,
    }

    pub struct Canvas {
        layers: RefCell<HashMap<&'static str, gdk::Texture>>,
        pub transition: Cell<Transition>,
        pub level: Cell<f32>,
        pub in_call: Cell<bool>,
        /// The frame clock's time at the last tick, in seconds.
        pub now: Cell<f64>,
        tick: RefCell<Option<gtk::TickCallbackId>>,
        watches: RefCell<Option<Watches>>,
    }

    impl Default for Canvas {
        fn default() -> Canvas {
            Canvas {
                layers: RefCell::default(),
                transition: Cell::new(Transition::settled(Expression::Idle)),
                level: Cell::new(0.0),
                in_call: Cell::new(false),
                now: Cell::new(0.0),
                tick: RefCell::default(),
                watches: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "FermixMascot";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            // Decorative: the status line beside it says the state in words.
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for Canvas {
        fn dispose(&self) {
            self.unwatch();
            self.stop_ticking();
        }
    }

    impl WidgetImpl for Canvas {
        fn map(&self) {
            self.parent_map();
            self.watch();
            self.follow_animations();
        }

        fn unmap(&self) {
            self.unwatch();
            self.stop_ticking();
            self.parent_unmap();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let t = self.now.get();
            let (transition, pose, blink) = if widget.settings().is_gtk_enable_animations() {
                let transition = self.transition.get();
                (
                    transition,
                    transition.pose(t, self.level.get()),
                    mascot::blink(t),
                )
            } else {
                (
                    Transition::settled(self.transition.get().to),
                    Pose::REST,
                    0.0,
                )
            };
            let unit = self.unit();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                widget.width() as f32 / 2.0,
                widget.height() as f32 / 2.0,
            ));
            snapshot.scale(unit, unit);
            self.draw_glow(snapshot);
            snapshot.rotate(pose.rotation_deg as f32);
            snapshot.translate(&graphene::Point::new(0.0, pose.dy as f32));
            snapshot.scale(pose.scale as f32, pose.scale as f32);
            self.draw_faces(snapshot, &transition.fades(t), blink);
            self.draw_plate(snapshot, &mascot::BALL);
            snapshot.restore();
        }
    }

    impl Canvas {
        /// Decodes every layer once. A missing one is a packaging defect that
        /// `core/tests/mascot.rs` catches, so GTK aborts naming the resource.
        pub fn load_layers(&self) {
            let mut layers = self.layers.borrow_mut();
            for layer in mascot::LAYERS {
                let path = format!("{RESOURCES}/{layer}.png");
                layers.insert(layer, gdk::Texture::from_resource(&path));
            }
        }

        /// Pixels per default point: the stage fits the size the mascot was made at.
        fn unit(&self) -> f32 {
            let (width, height) = self.obj().size_request();
            (f64::from(width) / STAGE.0).min(f64::from(height) / STAGE.1) as f32
        }

        /// A soft disc of the accent colour behind the ring.
        fn draw_glow(&self, snapshot: &gtk::Snapshot) {
            let accent = adw::StyleManager::default().accent_color_rgba();
            let strength = if self.in_call.get() {
                GLOW_IN_CALL
            } else {
                GLOW
            };
            let centre = graphene::Point::new(0.0, GLOW_DROP);
            let bounds = graphene::Rect::new(
                -GLOW_RADIUS,
                GLOW_DROP - GLOW_RADIUS,
                2.0 * GLOW_RADIUS,
                2.0 * GLOW_RADIUS,
            );
            let tint = accent.with_alpha(accent.alpha() * strength);
            let stops = [
                gsk::ColorStop::new(0.0, tint),
                gsk::ColorStop::new(GLOW_SOLID, tint),
                gsk::ColorStop::new(1.0, accent.with_alpha(0.0)),
            ];
            snapshot.append_radial_gradient(
                &bounds,
                &centre,
                GLOW_RADIUS,
                GLOW_RADIUS,
                0.0,
                1.0,
                &stops,
            );
        }

        /// The expression on show, or two mid-change. A cross-fade node mixes
        /// the two, so the body stays opaque where two translucent copies
        /// stacked would let the background through.
        fn draw_faces(&self, snapshot: &gtk::Snapshot, fades: &[Fade], blink: f64) {
            match fades {
                [only] => self.draw_expression(snapshot, only, blink),
                [from, to] => {
                    snapshot.push_cross_fade(to.weight);
                    self.draw_expression(snapshot, from, blink);
                    snapshot.pop();
                    self.draw_expression(snapshot, to, blink);
                    snapshot.pop();
                }
                _ => panic!("a crossfade is one or two expressions, got {fades:?}"),
            }
        }

        fn draw_expression(&self, snapshot: &gtk::Snapshot, fade: &Fade, blink: f64) {
            snapshot.save();
            snapshot.scale(fade.scale as f32, fade.scale as f32);
            for plate in mascot::plates(fade.expression, blink) {
                self.draw_plate(snapshot, &plate);
            }
            snapshot.restore();
        }

        /// One layer, [`ART`] points square about the centre, scaled then moved.
        fn draw_plate(&self, snapshot: &gtk::Snapshot, plate: &Plate) {
            let layers = self.layers.borrow();
            let texture = layers
                .get(plate.layer)
                .unwrap_or_else(|| panic!("the {} layer was never loaded", plate.layer));
            let side = (ART * plate.scale) as f32;
            let bounds = graphene::Rect::new(
                plate.dx as f32 - side / 2.0,
                plate.dy as f32 - side / 2.0,
                side,
                side,
            );
            snapshot.push_opacity(plate.opacity);
            snapshot.append_scaled_texture(texture, gsk::ScalingFilter::Trilinear, &bounds);
            snapshot.pop();
        }

        /// Ticks with animations on; with them off the mascot holds still.
        fn follow_animations(&self) {
            let widget = self.obj();
            widget.queue_draw();
            if !widget.settings().is_gtk_enable_animations() {
                return self.stop_ticking();
            }
            if self.tick.borrow().is_some() {
                return;
            }
            // Catch the clock up first, so the first frame after a pause does
            // not replay a change made while nothing was ticking.
            if let Some(clock) = widget.frame_clock() {
                self.now.set(seconds(&clock));
            }
            let tick = widget.add_tick_callback(|canvas, clock| {
                canvas.imp().now.set(seconds(clock));
                canvas.queue_draw();
                glib::ControlFlow::Continue
            });
            self.tick.replace(Some(tick));
        }

        fn stop_ticking(&self) {
            if let Some(tick) = self.tick.take() {
                tick.remove();
            }
        }

        /// Follows the animations setting and the accent colour while mapped.
        fn watch(&self) {
            let widget = self.obj();
            let settings = widget.settings();
            let weak = widget.downgrade();
            let animations = settings.connect_gtk_enable_animations_notify(move |_| {
                if let Some(canvas) = weak.upgrade() {
                    canvas.imp().follow_animations();
                }
            });
            let weak = widget.downgrade();
            let accent = adw::StyleManager::default().connect_accent_color_rgba_notify(move |_| {
                if let Some(canvas) = weak.upgrade() {
                    canvas.queue_draw();
                }
            });
            let watches = Watches {
                settings,
                animations,
                accent,
            };
            let previous = self.watches.replace(Some(watches));
            assert!(
                previous.is_none(),
                "the mascot was mapped twice without an unmap"
            );
        }

        fn unwatch(&self) {
            let Some(watches) = self.watches.take() else {
                return;
            };
            watches.settings.disconnect(watches.animations);
            adw::StyleManager::default().disconnect(watches.accent);
        }
    }

    /// The frame clock's time, in seconds.
    fn seconds(clock: &gdk::FrameClock) -> f64 {
        clock.frame_time() as f64 / 1_000_000.0
    }
}
