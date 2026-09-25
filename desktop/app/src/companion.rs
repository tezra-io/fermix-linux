//! The companion window (spec_voice §2.4): a small undecorated window holding the
//! mascot, dragged from anywhere. A primary click begins or ends the call; a
//! secondary click offers the call controls and a way back to Fermix. Linux
//! gives no way to keep it above other windows, so it never pretends to: GNOME's
//! own window menu (Alt+Space) has Always on Top, and the Voice page says so.
//! From the keyboard the card is one button: Space or Enter begins or ends the
//! call, and the Menu key or Shift+F10 opens the menu.

use crate::mascot::Mascot;
use crate::voice::VoiceView;
use adw::prelude::*;
use fermix_client::realtime::session::Palette;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;

pub struct Companion {
    pub window: gtk::Window,
    /// The card, which is the call's one button.
    card: gtk::Box,
    mascot: Mascot,
    status: gtk::Label,
    /// The menu's call items, which follow the call.
    call_items: gio::Menu,
    /// What was drawn last, so a new output level alone redraws only the mascot.
    shown: RefCell<Option<VoiceView>>,
}

impl Companion {
    pub fn new(application: &adw::Application) -> Companion {
        // The macOS pet's own stage size.
        let mascot = Mascot::new(132, 116);
        mascot.widget.set_halign(gtk::Align::Center);
        let status = gtk::Label::builder()
            .css_classes(["caption-heading"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let card = gtk::Box::builder()
            .focusable(true)
            .accessible_role(gtk::AccessibleRole::Button)
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            // Adwaita's clickable card: hover, press and the keyboard focus ring.
            .css_classes(["card", "activatable"])
            .build();
        card.append(&mascot.widget);
        card.append(&status);
        let window = companion_window(application, &card);
        let call_items = gio::Menu::new();
        let menu = gtk::PopoverMenu::builder()
            .menu_model(&companion_menu(&call_items))
            .has_arrow(false)
            .build();
        menu.set_parent(&card);
        let parented = menu.clone();
        window.connect_destroy(move |_| parented.unparent());
        wire_clicks(&card, &menu);
        wire_keys(&card, &menu);
        Companion {
            window,
            card,
            mascot,
            status,
            call_items,
            shown: RefCell::default(),
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
        // The error mode's word is a whole sentence, too long for this window:
        // the window shows that voice failed, and its tooltip says why.
        let error = view.palette == Palette::Error;
        let word = if error { "Voice failed" } else { &view.word };
        self.status.set_text(word);
        let action = if view.in_call { "End" } else { "Begin" };
        self.card
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "{action} voice call. Fermix: {word}"
            ))]);
        self.render_menu(view);
        let hint = if view.in_call {
            "Click to end the call. Right-click for more."
        } else {
            "Click to begin a voice call. Right-click for more."
        };
        let tip = if error {
            format!("{}\n{hint}", view.word)
        } else {
            hint.to_owned()
        };
        self.window.set_tooltip_text(Some(&tip));
        *self.shown.borrow_mut() = Some(view.clone());
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

fn companion_window(application: &adw::Application, card: &gtk::Box) -> gtk::Window {
    let handle = gtk::WindowHandle::builder().child(card).build();
    let window = gtk::Window::builder()
        .application(application)
        .title("Fermix Voice")
        .decorated(false)
        .resizable(false)
        .default_width(180)
        .default_height(200)
        .child(&handle)
        .build();
    window.add_css_class("fermix-companion");
    // Without a compositor nothing is see-through, and the corners around
    // the card would be black: the window takes the card's colour instead.
    if !WidgetExt::display(&window).is_composited() {
        window.add_css_class("opaque");
    }
    window
}

/// Primary click toggles the call; secondary click opens the menu. A drag
/// moves the window instead, through the `WindowHandle` around it.
fn wire_clicks(card: &gtk::Box, menu: &gtk::PopoverMenu) {
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
    card.add_controller(primary);
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
    card.add_controller(secondary);
}

/// The same for someone without a pointer: Space or Enter toggles the call,
/// the Menu key or Shift+F10 opens the menu.
fn wire_keys(card: &gtk::Box, menu: &gtk::PopoverMenu) {
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
    card.add_controller(keys);
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
