//! The voice mascot: the macOS app's Rive animation (`fermix_client::mascot`),
//! played offscreen by `fermix-rive` and drawn as a texture. A frame-clock tick
//! draws it 30 times a second only while it plays: while the widget is mapped
//! with animations on, and, with animations off, just long enough for a new
//! pose to land. Without the renderer (no EGL, or a frame that failed) it is
//! the mascot's one-ink mark, still, in the text colour.

use fermix_client::mascot::{
    self, level_to_write, plays, Expression, Pacing, LEVEL, MODE, STATE_MACHINE,
};
use fermix_rive::{Frame, Scene, Stage};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The mark drawn when the animation cannot be (marks/PROVENANCE.json, key app_icon_symbolic).
const STILL_MARK: &str = "io.tezra.Fermix-symbolic";
/// The still mark's share of the stage's shorter side: about the body's size in the animation.
const STILL_SHARE: f32 = 0.5;

/// Opens the renderer once for every mascot in the app. Without one, each mascot
/// is drawn still, and the log says why.
pub fn open_stage() -> Option<Rc<Stage>> {
    // The file is in the app's own resources; without it the build is broken.
    let riv = gio::resources_lookup_data(mascot::RESOURCE, gio::ResourceLookupFlags::NONE)
        .unwrap_or_else(|e| panic!("{} is not in the app's resources: {e}", mascot::RESOURCE));
    match Stage::new(&riv) {
        Ok(stage) => Some(stage),
        Err(error) => {
            glib::g_warning!("fermix", "the mascot is drawn still: {error}");
            None
        }
    }
}

/// The mascot widget and its controls. Clones share the one widget.
#[derive(Clone)]
pub struct Mascot {
    pub widget: gtk::Widget,
    canvas: Canvas,
}

impl Mascot {
    /// A mascot on a `width` by `height` stage, played on `stage`. The
    /// animation is fitted inside the stage; at 132 by 116, the macOS
    /// companion's stage, it is drawn 116 square, as there.
    pub fn new(stage: Option<&Rc<Stage>>, width: i32, height: i32) -> Mascot {
        assert!(
            width > 0 && height > 0,
            "the mascot's stage needs a size, got {width}x{height}"
        );
        let canvas: Canvas = glib::Object::new();
        canvas.set_size_request(width, height);
        canvas.imp().load(stage);
        Mascot {
            widget: canvas.clone().upcast(),
            canvas,
        }
    }

    /// Blends to `expression`; the expression already on show changes nothing.
    pub fn set_expression(&self, expression: Expression) {
        self.canvas.imp().set_expression(expression);
    }

    /// The smoothed output level, 0 to 1, which the speaking mouth follows.
    pub fn set_level(&self, level: f32) {
        assert!(
            (0.0..=1.0).contains(&level),
            "output level must be 0 to 1, got {level}"
        );
        self.canvas.imp().level.set(level);
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

mod imp {
    use super::*;

    /// The signal handler a mapped mascot holds on the settings, which outlive it.
    struct Watch {
        settings: gtk::Settings,
        animations: glib::SignalHandlerId,
    }

    pub struct Canvas {
        /// None when the animation cannot be drawn: the still mark is.
        scene: RefCell<Option<Scene>>,
        expression: Cell<Expression>,
        /// The level wanted, and the last written to the animation.
        pub level: Cell<f32>,
        written_level: Cell<f32>,
        /// When the pose last changed, in `glib::monotonic_time` seconds.
        pose_changed_at: Cell<Option<f64>>,
        pacing: Cell<Pacing>,
        frame: RefCell<Option<gdk::Texture>>,
        tick: RefCell<Option<gtk::TickCallbackId>>,
        watch: RefCell<Option<Watch>>,
    }

    impl Default for Canvas {
        fn default() -> Canvas {
            Canvas {
                scene: RefCell::default(),
                expression: Cell::new(Expression::Idle),
                level: Cell::new(0.0),
                written_level: Cell::new(0.0),
                pose_changed_at: Cell::new(None),
                pacing: Cell::default(),
                frame: RefCell::default(),
                tick: RefCell::default(),
                watch: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "FermixMascot";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            // Decorative: the Voice page's status line and the companion's
            // label and tooltip say the state in words.
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for Canvas {
        fn dispose(&self) {
            self.unwatch();
            self.stop_ticking();
            self.scene.take();
        }
    }

    impl WidgetImpl for Canvas {
        fn map(&self) {
            self.parent_map();
            self.watch();
            self.follow();
        }

        fn unmap(&self) {
            self.unwatch();
            self.stop_ticking();
            self.parent_unmap();
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            // A parked mascot draws once at its new size; a playing one will anyway.
            if self.tick.borrow().is_none() && self.obj().is_mapped() {
                self.draw_frame(0.0);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            if self.scene.borrow().is_none() {
                return self.snapshot_still(snapshot, width, height);
            }
            if let Some(frame) = self.frame.borrow().as_ref() {
                snapshot.append_texture(frame, &graphene::Rect::new(0.0, 0.0, width, height));
            }
        }
    }

    impl Canvas {
        /// A scene of its own on the shared stage; none without a stage.
        pub fn load(&self, stage: Option<&Rc<Stage>>) {
            let Some(stage) = stage else { return };
            match Scene::new(stage, STATE_MACHINE) {
                Ok(scene) => {
                    self.scene.replace(Some(scene));
                    self.write_pose(Expression::Idle);
                }
                Err(error) => glib::g_warning!("fermix", "the mascot is drawn still: {error}"),
            }
        }

        pub fn set_expression(&self, expression: Expression) {
            if expression == self.expression.get() {
                return;
            }
            self.expression.set(expression);
            self.write_pose(expression);
            self.pose_changed_at.set(Some(monotonic_seconds()));
            self.follow();
        }

        fn write_pose(&self, expression: Expression) {
            let mut scene = self.scene.borrow_mut();
            let Some(scene) = scene.as_mut() else { return };
            if let Err(error) = scene.set_enum(MODE, expression.mode()) {
                glib::g_critical!("fermix", "the mascot did not take its pose: {error}");
            }
        }

        fn write_level(&self) {
            let wanted = level_to_write(
                self.expression.get(),
                self.level.get(),
                self.written_level.get(),
            );
            let Some(level) = wanted else { return };
            let mut scene = self.scene.borrow_mut();
            let Some(scene) = scene.as_mut() else { return };
            match scene.set_number(LEVEL, level) {
                Ok(()) => self.written_level.set(level),
                Err(error) => {
                    glib::g_critical!("fermix", "the mascot did not take the level: {error}")
                }
            }
        }

        /// Plays while `plays` says so, drawing at the frame rate; stops the
        /// tick otherwise. Mapping draws the first frame at once.
        fn follow(&self) {
            let widget = self.obj();
            if !widget.is_mapped() || self.scene.borrow().is_none() {
                return;
            }
            if self.frame.borrow().is_none() {
                self.draw_frame(0.0);
            }
            if !self.playing() {
                return self.stop_ticking();
            }
            if self.tick.borrow().is_some() {
                return;
            }
            let tick = widget.add_tick_callback(|canvas, clock| canvas.imp().on_tick(clock));
            self.tick.replace(Some(tick));
        }

        fn playing(&self) -> bool {
            let animates = self.obj().settings().is_gtk_enable_animations();
            plays(animates, self.pose_changed_at.get(), monotonic_seconds())
        }

        fn on_tick(&self, clock: &gdk::FrameClock) -> glib::ControlFlow {
            if !self.playing() {
                // Returning Break removes the callback, so its id is spent.
                self.tick.take();
                self.pacing.set(Pacing::default());
                return glib::ControlFlow::Break;
            }
            let mut pacing = self.pacing.get();
            let step = pacing.frame(clock.frame_time() as f64 / 1_000_000.0);
            self.pacing.set(pacing);
            if let Some(step) = step {
                self.draw_frame(step);
            }
            glib::ControlFlow::Continue
        }

        /// Advances by `step` seconds and draws a frame at the widget's size in
        /// device pixels. A frame that fails drops the scene: the still mark
        /// is drawn from then on, and the log says why.
        fn draw_frame(&self, step: f64) {
            let widget = self.obj();
            let scale = widget.scale_factor();
            let (width, height) = (widget.width() * scale, widget.height() * scale);
            if width <= 0 || height <= 0 {
                return;
            }
            self.write_level();
            let drawn = {
                let mut scene = self.scene.borrow_mut();
                let Some(scene) = scene.as_mut() else { return };
                scene.advance(step as f32);
                scene.render(width as u32, height as u32)
            };
            match drawn {
                Ok(frame) => {
                    self.frame.replace(Some(texture(frame)));
                }
                Err(error) => {
                    glib::g_warning!("fermix", "the mascot is drawn still: {error}");
                    self.stop_ticking();
                    self.scene.take();
                    self.frame.take();
                }
            }
            widget.queue_draw();
        }

        /// The one-ink mark, centred, in the text colour.
        fn snapshot_still(&self, snapshot: &gtk::Snapshot, width: f32, height: f32) {
            let widget = self.obj();
            let side = width.min(height) * STILL_SHARE;
            let theme = gtk::IconTheme::for_display(&widget.display());
            let icon = theme.lookup_icon(
                STILL_MARK,
                &[],
                side.round() as i32,
                widget.scale_factor(),
                widget.direction(),
                gtk::IconLookupFlags::empty(),
            );
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                (width - side) / 2.0,
                (height - side) / 2.0,
            ));
            icon.snapshot_symbolic(
                snapshot,
                f64::from(side),
                f64::from(side),
                &[widget.color()],
            );
            snapshot.restore();
        }

        fn stop_ticking(&self) {
            if let Some(tick) = self.tick.take() {
                tick.remove();
            }
            self.pacing.set(Pacing::default());
        }

        /// Follows the animations setting while mapped.
        fn watch(&self) {
            let widget = self.obj();
            let settings = widget.settings();
            let weak = widget.downgrade();
            let animations = settings.connect_gtk_enable_animations_notify(move |_| {
                if let Some(canvas) = weak.upgrade() {
                    canvas.imp().follow();
                }
            });
            let previous = self.watch.replace(Some(Watch {
                settings,
                animations,
            }));
            assert!(
                previous.is_none(),
                "the mascot was mapped twice without an unmap"
            );
        }

        fn unwatch(&self) {
            if let Some(watch) = self.watch.take() {
                watch.settings.disconnect(watch.animations);
            }
        }
    }

    /// A frame as a texture GTK can draw: the renderer's premultiplied RGBA.
    fn texture(frame: Frame) -> gdk::Texture {
        let stride = frame.stride();
        let bytes = glib::Bytes::from_owned(frame.rgba);
        gdk::MemoryTexture::new(
            frame.width as i32,
            frame.height as i32,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &bytes,
            stride,
        )
        .upcast()
    }

    fn monotonic_seconds() -> f64 {
        glib::monotonic_time() as f64 / 1_000_000.0
    }
}
