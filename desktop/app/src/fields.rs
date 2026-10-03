//! Fields typed into rows, shared by the daemon's settings sections and the
//! integrations detail: the row a secret is typed into, and keeping someone's
//! place in a field while the rows around it are drawn again.

use adw::prelude::*;
use fermix_client::settings::FIELD_CHARS;
use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;

/// Whether a row is still on screen. A redraw retires the rows it replaces, and
/// a retired row sends nothing: taking a focused field away counts as leaving
/// it, which would otherwise save what was half typed.
pub type Live = Rc<Cell<bool>>;

/// Sends a typed secret, from the field it was typed in.
pub type Store = Rc<dyn Fn(&gtk::PasswordEntry, String)>;

/// Where a secret row sends its value, and how it forgets the stored one. Each
/// is handed the widget it fires from, so it can reach the window's actions.
pub struct Slot {
    pub label: String,
    pub present: bool,
    pub store: Store,
    pub remove: Rc<dyn Fn(&gtk::Button)>,
}

/// Whether the field's window still has the keyboard. Switching to another
/// window also counts as leaving a field in GTK, and a value half typed (a
/// token being copied across in two goes) must not be saved then.
pub fn still_active(field: &impl IsA<gtk::Widget>) -> bool {
    field
        .root()
        .and_downcast::<gtk::Window>()
        .is_some_and(|window| window.is_active())
}

/// A row that holds a field hands its focus to the field: Tab goes from field
/// to field, not to the row around the next one, and a click on the row lands
/// in its field.
pub fn field_row(action: &adw::ActionRow, field: &impl IsA<gtk::Widget>) {
    action.set_focusable(false);
    action.set_activatable_widget(Some(field));
}

/// Where someone is typing: the row, what they have typed, and the cursor.
pub struct Typing {
    key: String,
    text: String,
    position: i32,
}

/// How deep a row's widgets go before its text field; this bounds the search.
const MAX_ROW_DEPTH: usize = 12;

/// The row, by key, that holds the focused field, with what is typed in it.
pub fn typing_in(widgets: &[(String, gtk::Widget)]) -> Option<Typing> {
    let focus = widgets.first()?.1.root()?.focus()?;
    let text = focus.downcast::<gtk::Text>().ok()?;
    let (key, _) = widgets.iter().find(|(_, row)| text.is_ancestor(row))?;
    Some(Typing {
        key: key.clone(),
        text: text.text().into(),
        position: text.position(),
    })
}

/// Takes the focus off the field being typed in before its row is removed.
/// GTK moves the focus of a removed widget at the next frame, which would undo
/// `resume` putting it back; a window with no focus has nothing to move. The
/// field is retired first, so leaving it saves nothing.
pub fn let_go(widgets: &[(String, gtk::Widget)]) {
    if let Some(root) = widgets.first().and_then(|(_, row)| row.root()) {
        root.set_focus(None::<&gtk::Widget>);
    }
}

/// Puts the typing back into the rebuilt row. A stored secret being replaced
/// shows its field again first.
pub fn resume(widgets: &[(String, gtk::Widget)], typing: &Typing) {
    let Some((_, row)) = widgets.iter().find(|(key, _)| *key == typing.key) else {
        return;
    };
    let Some(text) = first_text(row, 0) else {
        return;
    };
    let swap = text
        .ancestor(gtk::Stack::static_type())
        .and_downcast::<gtk::Stack>();
    if let Some(swap) = swap.filter(|s| s.is_ancestor(row)) {
        // Nothing typed yet (as right after Enter stored it): Stored stands.
        if typing.text.is_empty() {
            return focus_stored(&swap);
        }
        swap.set_visible_child_name(TYPING);
    }
    text.set_text(&typing.text);
    // A row just added is not on screen yet and refuses the focus, which GTK
    // then gives to the first button; the next idle turn it takes it.
    let (target, position) = (text.downgrade(), typing.position);
    glib::idle_add_local_once(move || {
        let Some(text) = target.upgrade() else {
            return;
        };
        if !text.grab_focus() {
            glib::g_debug!("fermix", "a redrawn field did not take the focus back");
        }
        text.set_position(position);
    });
}

/// The focus goes to Stored's own buttons, on the next idle turn for the same
/// reason as in `resume`: `let_go` took it off the field, so it is not lost.
fn focus_stored(swap: &gtk::Stack) {
    let target = swap.downgrade();
    glib::idle_add_local_once(move || {
        let Some(swap) = target.upgrade() else {
            return;
        };
        if !swap.child_focus(gtk::DirectionType::TabForward) {
            glib::g_debug!("fermix", "a stored secret's buttons did not take the focus");
        }
    });
}

fn first_text(widget: &gtk::Widget, depth: usize) -> Option<gtk::Text> {
    if let Some(text) = widget.downcast_ref::<gtk::Text>() {
        return Some(text.clone());
    }
    if depth >= MAX_ROW_DEPTH {
        return None;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(text) = first_text(&current, depth + 1) {
            return Some(text);
        }
        child = current.next_sibling();
    }
    None
}

const STORED: &str = "stored";
const TYPING: &str = "typing";

/// A secret is typed in its own row, as on macOS (`SecretRow.swift`): a popup
/// never raises a second popup. Absent, the row is the password field; stored,
/// it reads Stored with Replace… and Remove, and Replace… turns the row into
/// the field in place. Escape, or leaving it empty, puts Stored back.
pub fn secret_row(action: &adw::ActionRow, slot: Slot, live: &Live) {
    let slot = Rc::new(slot);
    if !slot.present {
        let entry = secret_entry(&slot, None, live);
        action.add_suffix(&entry);
        field_row(action, &entry);
        return;
    }
    action.set_focusable(false);
    let swap = gtk::Stack::builder()
        .hhomogeneous(false)
        .valign(gtk::Align::Center)
        .build();
    let shown = swap.downgrade();
    let back: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(swap) = shown.upgrade() {
            swap.set_visible_child_name(STORED);
        }
    });
    let entry = secret_entry(&slot, Some(back), live);
    swap.add_named(&stored_controls(&slot, &swap, &entry), Some(STORED));
    swap.add_named(&entry, Some(TYPING));
    swap.set_visible_child_name(STORED);
    action.add_suffix(&swap);
}

/// Stored, Replace… and Remove. Replace… shows the field in their place.
fn stored_controls(slot: &Rc<Slot>, swap: &gtk::Stack, entry: &gtk::PasswordEntry) -> gtk::Box {
    let status = gtk::Label::builder()
        .label("Stored")
        .css_classes(["dim-label"])
        .build();
    let replace = gtk::Button::builder()
        .label("Replace…")
        .valign(gtk::Align::Center)
        .build();
    replace.update_property(&[gtk::accessible::Property::Label(&format!(
        "Replace {}",
        slot.label
    ))]);
    let (shown, field) = (swap.downgrade(), entry.downgrade());
    replace.connect_clicked(move |_| {
        let (Some(swap), Some(field)) = (shown.upgrade(), field.upgrade()) else {
            return;
        };
        swap.set_visible_child_name(TYPING);
        field.grab_focus();
    });
    let remove = gtk::Button::builder()
        .label("Remove")
        .valign(gtk::Align::Center)
        .build();
    remove.update_property(&[gtk::accessible::Property::Label(&format!(
        "Remove {}",
        slot.label
    ))]);
    let forget = slot.remove.clone();
    remove.connect_clicked(move |button| forget(button));
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.append(&status);
    controls.append(&replace);
    controls.append(&remove);
    controls
}

/// The field a secret is typed into. What is typed is stored on Enter and when
/// focus leaves (closing the dialog moves it), and is never shown again; Escape
/// drops it. The field clears as it sends, so a refused save shows its reason
/// under an empty field and the value is typed again. `back` is the Stored
/// state a replacement returns to.
fn secret_entry(slot: &Rc<Slot>, back: Option<Rc<dyn Fn()>>, live: &Live) -> gtk::PasswordEntry {
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Paste the value")
        .valign(gtk::Align::Center)
        .width_chars(FIELD_CHARS)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&slot.label)]);
    let on_enter = slot.clone();
    entry.connect_activate(move |entry| store_typed(entry, &on_enter));
    let focus = gtk::EventControllerFocus::new();
    let (watched, on_leave, live, slot) =
        (entry.downgrade(), back.clone(), live.clone(), slot.clone());
    focus.connect_leave(move |_| {
        let Some(entry) = watched.upgrade().filter(|e| live.get() && still_active(e)) else {
            return;
        };
        store_typed(&entry, &slot);
        if let Some(back) = &on_leave {
            back();
        }
    });
    entry.add_controller(focus);
    drop_on_escape(&entry, back);
    entry
}

/// Escape drops what was typed and, on a stored secret, shows Stored again. An
/// empty field lets Escape through, so it still closes the dialog.
fn drop_on_escape(entry: &gtk::PasswordEntry, back: Option<Rc<dyn Fn()>>) {
    let keys = gtk::EventControllerKey::new();
    let target = entry.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(entry) = target.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if key != gtk::gdk::Key::Escape || (entry.text().is_empty() && back.is_none()) {
            return glib::Propagation::Proceed;
        }
        entry.set_text("");
        if let Some(back) = &back {
            back();
        }
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

/// Sends what was typed, once: the field is cleared first, so the redraw that
/// follows cannot send it again. A blank is never sent.
fn store_typed(entry: &gtk::PasswordEntry, slot: &Slot) {
    let value = entry.text().trim().to_owned();
    if value.is_empty() {
        return;
    }
    entry.set_text("");
    (slot.store)(entry, value);
}
