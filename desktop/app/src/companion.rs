//! The companion window (spec_voice §2.4), floating as the macOS pet does: no card
//! and no window, only the mascot and its glow on whatever is below. The window is
//! see-through, and it takes the pointer only over the mascot and, while they
//! show, the call controls under it (`fermix_client::companion`); a click anywhere
//! else reaches the desktop. It is dragged from anywhere it takes the pointer.
//!
//! As on macOS, a small pill of call controls shows while the pointer is over the
//! pet, through a call and while Fermix speaks, and the state is the mascot's own
//! expression; the tooltip says what a click does or why voice failed. A primary
//! click on the mascot begins or ends the call; a secondary click offers the call
//! controls and a way back to Fermix. Linux gives no way to keep it above other
//! windows, so it never pretends to: GNOME's own window menu (Alt+Space) has
//! Always on Top, and the Voice page says so. From the keyboard the mascot is one
//! button: Space or Enter begins or ends the call, and the Menu key or Shift+F10
//! opens the menu.
//!
//! Without a compositor nothing is see-through: the window is then a plain tile
//! in the window's own colour with the controls always on it, and all of it
//! takes the pointer.

use crate::mascot::Mascot;
use crate::voice::VoiceView;
use adw::prelude::*;
use fermix_client::companion::{self, Rect};
use fermix_client::mascot::Expression;
use fermix_client::realtime::session::Palette;
use gtk::{cairo, gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Companion {
    pub window: gtk::Window,
    /// The mascot's frame, which is the call's one button.
    button: MascotButton,
    mascot: Mascot,
    controls: Controls,
    /// The menu's call items, which follow the call.
    call_items: gio::Menu,
    /// What was drawn last, so a new output level alone redraws only the mascot.
    /// The hover handler reads it too.
    shown: Rc<RefCell<Option<VoiceView>>>,
    hovered: Rc<Cell<bool>>,
    /// The window is see-through. Without a compositor it is a solid tile.
    floating: bool,
}

impl Companion {
    pub fn new(application: &adw::Application) -> Companion {
        // The macOS pet's own stage size.
        let mascot = Mascot::new(132, 116);
        let button = MascotButton::new(&mascot.widget);
        let controls = Controls::new();
        let stage = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        stage.append(&button);
        stage.append(&controls.dock);
        let window = companion_window(application, &stage);
        let call_items = wire_button(&button, &window);
        let shown = Rc::default();
        let hovered = Rc::default();
        // Without a compositor nothing is see-through, and the space around the
        // mascot would be black: the window is then a solid tile in its own
        // colour, all of it takes the pointer, and the controls always show on it.
        let floating = WidgetExt::display(&window).is_composited();
        if floating {
            follow_input_region(&window, &mascot.widget, &controls.dock);
            wire_hover(&stage, &controls.dock, &shown, &hovered);
        } else {
            window.add_css_class("opaque");
            controls.dock.set_reveal_child(true);
        }
        Companion {
            window,
            button,
            mascot,
            controls,
            call_items,
            shown,
            hovered,
            floating,
        }
    }

    pub fn render(&self, view: &VoiceView) {
        self.mascot.set_level(view.level);
        let unchanged = self
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|last| last.same_apart_from_level(view));
        if unchanged {
            return;
        }
        self.mascot.set_expression(view.expression);
        self.mascot.set_in_call(view.in_call);
        // The error mode's word is a whole sentence: the label says that voice
        // failed, and the tooltip says why.
        let error = view.palette == Palette::Error;
        let word = if error { "Voice failed" } else { &view.word };
        let action = if view.in_call { "End" } else { "Begin" };
        self.button
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "{action} voice call. Fermix: {word}"
            ))]);
        self.button.set_tooltip_text(Some(&tooltip(view, error)));
        self.render_menu(view);
        self.controls.render(view);
        *self.shown.borrow_mut() = Some(view.clone());
        if self.floating {
            show_controls(&self.controls.dock, self.hovered.get(), Some(view));
        }
    }

    /// The call's items. Mute is the stateful `app.voice-mute`, so the menu
    /// shows it checked while the microphone is muted.
    fn render_menu(&self, view: &VoiceView) {
        self.call_items.remove_all();
        let call = if view.in_call {
            "End Voice Call"
        } else {
            "Begin Voice Call"
        };
        self.call_items.append(Some(call), Some("app.voice-call"));
        if view.in_call {
            self.call_items
                .append(Some("Mute Microphone"), Some("app.voice-mute"));
        }
        if view.can_stop {
            self.call_items
                .append(Some("Stop the Reply"), Some("app.voice-stop"));
        }
    }
}

/// What a click does, and while voice has failed, why first.
fn tooltip(view: &VoiceView, error: bool) -> String {
    let hint = if view.in_call {
        "Click to end the call. Right-click for more."
    } else {
        "Click to begin a voice call. Right-click for more."
    };
    if error {
        format!("{}\n{hint}", view.word)
    } else {
        hint.to_owned()
    }
}

/// The pill of call controls under the mascot (macOS `ControlDock`): begin or
/// end, stop the reply while Fermix thinks or speaks, and mute through a call.
struct Controls {
    /// Fades in and out, keeping its room, so the mascot never moves.
    dock: gtk::Revealer,
    call: gtk::Button,
    mute: gtk::ToggleButton,
    stop: gtk::Button,
}

impl Controls {
    fn new() -> Controls {
        let call = control_button("call-start-symbolic", "Begin Voice Call");
        call.set_action_name(Some("app.voice-call"));
        let stop = control_button("media-playback-stop-symbolic", "Stop the Reply");
        stop.set_action_name(Some("app.voice-stop"));
        let mute = gtk::ToggleButton::builder()
            .icon_name("microphone-disabled-symbolic")
            .tooltip_text("Mute Microphone")
            .css_classes(["flat", "circular"])
            .build();
        mute.update_property(&[gtk::accessible::Property::Label("Mute Microphone")]);
        mute.set_action_name(Some("app.voice-mute"));
        let pill = gtk::Box::builder()
            .spacing(4)
            .css_classes(["fermix-companion-dock"])
            .build();
        pill.append(&call);
        pill.append(&stop);
        pill.append(&mute);
        let dock = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .halign(gtk::Align::Center)
            .child(&pill)
            .build();
        Controls {
            dock,
            call,
            mute,
            stop,
        }
    }

    fn render(&self, view: &VoiceView) {
        let (icon, label) = if view.in_call {
            ("call-stop-symbolic", "End Voice Call")
        } else {
            ("call-start-symbolic", "Begin Voice Call")
        };
        self.call.set_icon_name(icon);
        self.call.set_tooltip_text(Some(label));
        self.call
            .update_property(&[gtk::accessible::Property::Label(label)]);
        if view.in_call {
            self.call.add_css_class("destructive-action");
        } else {
            self.call.remove_css_class("destructive-action");
        }
        // A call that is starting or ending waits, as the Voice page's button does.
        self.call.set_sensitive(!view.busy);
        self.stop.set_visible(view.can_stop);
        self.mute.set_visible(view.in_call);
    }
}

fn control_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(label)
        .css_classes(["flat", "circular"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// Shows or hides the call controls by the macOS rule.
fn show_controls(dock: &gtk::Revealer, hovered: bool, view: Option<&VoiceView>) {
    let (in_call, expression) =
        view.map_or((false, Expression::Idle), |v| (v.in_call, v.expression));
    dock.set_reveal_child(companion::controls_shown(hovered, in_call, expression));
}

/// The pointer over the mascot or its controls shows them, as hovering the pet
/// does on macOS; leaving hides them again unless the call keeps them.
fn wire_hover(
    stage: &gtk::Box,
    dock: &gtk::Revealer,
    shown: &Rc<RefCell<Option<VoiceView>>>,
    hovered: &Rc<Cell<bool>>,
) {
    let (dock, shown, hovered) = (dock.clone(), shown.clone(), hovered.clone());
    let follow = Rc::new(move |inside: bool| {
        hovered.set(inside);
        show_controls(&dock, inside, shown.borrow().as_ref());
    });
    let motion = gtk::EventControllerMotion::new();
    let enter = follow.clone();
    motion.connect_enter(move |_, _, _| enter(true));
    motion.connect_leave(move |_| follow(false));
    stage.add_controller(motion);
}

fn companion_window(application: &adw::Application, stage: &gtk::Box) -> gtk::Window {
    let handle = gtk::WindowHandle::builder().child(stage).build();
    let window = gtk::Window::builder()
        .application(application)
        .title("Fermix Voice")
        .decorated(false)
        .resizable(false)
        .child(&handle)
        .build();
    window.add_css_class("fermix-companion");
    window
}

/// After each frame the window takes the pointer only where the mascot and its
/// shown controls are, so clicks on the see-through rest reach what is below.
/// The frame clock belongs to the realized window, so the watch does too. Each
/// time the window is shown the region is set again, in case showing it anew
/// gave the compositor a new surface.
fn follow_input_region(window: &gtk::Window, mascot: &gtk::Widget, dock: &gtk::Revealer) {
    let watch: Rc<RefCell<Option<(gdk::FrameClock, glib::SignalHandlerId)>>> = Rc::default();
    let applied: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let (held, cleared) = (watch.clone(), applied.clone());
    window.connect_map(move |_| cleared.borrow_mut().clear());
    let (mascot, dock) = (mascot.downgrade(), dock.downgrade());
    window.connect_realize(move |window| {
        let clock = window
            .frame_clock()
            .expect("a realized window has a frame clock");
        let (window, mascot, dock) = (window.downgrade(), mascot.clone(), dock.clone());
        let applied = applied.clone();
        let handler = clock.connect_after_paint(move |_| {
            let (Some(window), Some(mascot), Some(dock)) =
                (window.upgrade(), mascot.upgrade(), dock.upgrade())
            else {
                return;
            };
            apply_input_region(&window, &mascot, &dock, &applied);
        });
        held.replace(Some((clock, handler)));
    });
    window.connect_unrealize(move |_| {
        if let Some((clock, handler)) = watch.take() {
            clock.disconnect(handler);
        }
    });
}

/// Sets the surface's input region when the parts that take the pointer moved.
fn apply_input_region(
    window: &gtk::Window,
    mascot: &gtk::Widget,
    dock: &gtk::Revealer,
    applied: &RefCell<Vec<Rect>>,
) {
    // Before the first layout there is nothing to measure yet.
    let Some(stage) = surface_rect(window, mascot) else {
        return;
    };
    let controls = dock
        .reveals_child()
        .then(|| surface_rect(window, dock.upcast_ref()))
        .flatten();
    let rects = companion::input_region(stage, controls);
    if *applied.borrow() == rects {
        return;
    }
    let Some(surface) = window.surface() else {
        return;
    };
    let region = cairo::Region::create();
    for r in &rects {
        let rectangle = cairo::RectangleInt::new(r.x, r.y, r.width, r.height);
        if let Err(e) = region.union_rectangle(&rectangle) {
            glib::g_warning!("fermix", "the companion's input region failed: {e}");
            return;
        }
    }
    surface.set_input_region(Some(&region));
    glib::g_debug!(
        "fermix",
        "the companion takes the pointer over {} rectangles within {stage:?}",
        rects.len()
    );
    *applied.borrow_mut() = rects;
}

/// Where `widget` sits on the window's surface, in whole pixels that cover it.
fn surface_rect(window: &gtk::Window, widget: &gtk::Widget) -> Option<Rect> {
    let bounds = widget.compute_bounds(window)?;
    let (dx, dy) = window.surface_transform();
    let left = (f64::from(bounds.x()) + dx).floor();
    let top = (f64::from(bounds.y()) + dy).floor();
    let right = (f64::from(bounds.x() + bounds.width()) + dx).ceil();
    let bottom = (f64::from(bounds.y() + bounds.height()) + dy).ceil();
    Some(Rect {
        x: left as i32,
        y: top as i32,
        width: (right - left) as i32,
        height: (bottom - top) as i32,
    })
}

/// The menu on the mascot, and the clicks and keys that open it or toggle the
/// call. Returns the menu's call items, which follow the call.
fn wire_button(button: &MascotButton, window: &gtk::Window) -> gio::Menu {
    let call_items = gio::Menu::new();
    let menu = gtk::PopoverMenu::builder()
        .menu_model(&companion_menu(&call_items))
        .has_arrow(false)
        .build();
    menu.set_parent(button);
    let parented = menu.clone();
    window.connect_destroy(move |_| parented.unparent());
    wire_clicks(button, &menu);
    wire_keys(button, &menu);
    call_items
}

/// Primary click toggles the call; secondary click opens the menu. A drag
/// moves the window instead, through the `WindowHandle` around it.
fn wire_clicks(button: &MascotButton, menu: &gtk::PopoverMenu) {
    let primary = gtk::GestureClick::builder()
        .button(gdk::BUTTON_PRIMARY)
        .build();
    primary.connect_released(|gesture, presses, _, _| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        // Like any button, a click leaves the keyboard on it.
        widget.grab_focus();
        if presses == 1 {
            toggle_call(&widget);
        }
    });
    button.add_controller(primary);
    let secondary = gtk::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    let menu = menu.clone();
    secondary.connect_pressed(move |_, _, x, y| {
        // A point rectangle at the click, which is where the menu belongs.
        #[allow(clippy::cast_possible_truncation)]
        let at = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        menu.set_pointing_to(Some(&at));
        menu.popup();
    });
    button.add_controller(secondary);
}

/// The same for someone without a pointer: Space or Enter toggles the call,
/// the Menu key or Shift+F10 opens the menu.
fn wire_keys(button: &MascotButton, menu: &gtk::PopoverMenu) {
    let keys = gtk::EventControllerKey::new();
    let menu = menu.clone();
    keys.connect_key_pressed(move |controller, key, _, modifiers| {
        let Some(widget) = controller.widget() else {
            return glib::Propagation::Proceed;
        };
        let shift_f10 = key == gdk::Key::F10 && modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        if key == gdk::Key::Menu || shift_f10 {
            menu.set_pointing_to(None);
            menu.popup();
            return glib::Propagation::Stop;
        }
        let activates = [gdk::Key::space, gdk::Key::Return, gdk::Key::KP_Enter];
        if !activates.contains(&key) {
            return glib::Propagation::Proceed;
        }
        toggle_call(&widget);
        glib::Propagation::Stop
    });
    button.add_controller(keys);
}

fn toggle_call(widget: &gtk::Widget) {
    if let Err(e) = widget.activate_action("app.voice-call", None) {
        glib::g_warning!("fermix", "app.voice-call could not be sent: {e}");
    }
}

fn companion_menu(call: &gio::Menu) -> gio::Menu {
    let window = gio::Menu::new();
    window.append(Some("Open Fermix"), Some("app.show-voice"));
    window.append(Some("Close Companion"), Some("app.companion-close"));
    let menu = gio::Menu::new();
    menu.append_section(None, call);
    menu.append_section(None, &window);
    menu
}

glib::wrapper! {
    /// The mascot's frame, one button. A box would only pass the keyboard on to
    /// its children, and a `gtk::Button` would claim the press that the window
    /// handle needs to drag the companion, so the frame is its own widget: it
    /// takes the keyboard itself, and it leaves presses to its gestures.
    pub struct MascotButton(ObjectSubclass<frame::MascotButton>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MascotButton {
    fn new(mascot: &gtk::Widget) -> MascotButton {
        let button: MascotButton = glib::Object::builder()
            .property("focusable", true)
            .property("halign", gtk::Align::Center)
            .build();
        button.add_css_class("fermix-companion-mascot");
        mascot.set_parent(&button);
        button
    }
}

mod frame {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct MascotButton;

    #[glib::object_subclass]
    impl ObjectSubclass for MascotButton {
        const NAME: &'static str = "FermixMascotButton";
        type Type = super::MascotButton;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_accessible_role(gtk::AccessibleRole::Button);
        }
    }

    impl ObjectImpl for MascotButton {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for MascotButton {}
}
