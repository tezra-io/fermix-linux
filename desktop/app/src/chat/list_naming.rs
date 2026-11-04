//! Naming in place in the Lists pane's head (M85 lists §3.3, §10): New list, Rename list and New
//! folder. The picker gives way to an entry, "Folder name" for a new folder and "List name"
//! otherwise; Rename list's starts with the list's title selected, the others empty. Return keeps
//! the name, as core's `pane::named` makes its op; Escape, or the focus leaving, changes nothing,
//! so a click elsewhere never makes a list or a folder. A rename or a new folder ends with nothing
//! changed when the pane turns to another list, and any naming does when the pane hides. No list
//! edit is an exported action: the name goes to the session through `Lister`.

use super::list_menu::at_idle;
use super::lists::{act, ListAct, Lister};
use crate::fields::still_active;
use adw::prelude::*;
use fermix_client::chat_wire::protocol::ListOp;
use fermix_client::checklists::pane::{named, Naming};
use fermix_client::checklists::words::{FOLDER_NAME, LIST_NAME};
use gtk::glib;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// The naming going on: what it names, and the list it names for, none for a new list.
struct Going {
    naming: Naming,
    list_id: Option<String>,
}

/// The head's first place: the picker, or the entry while a name is typed there.
pub struct Namer {
    pub widget: gtk::Stack,
    picker: gtk::MenuButton,
    entry: gtk::Entry,
    lister: Lister,
    going: RefCell<Option<Going>>,
}

impl Namer {
    /// The picker's place in the head, holding `picker` until a naming begins.
    pub fn new(picker: &gtk::MenuButton, lister: &Lister) -> Rc<Namer> {
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        let widget = gtk::Stack::new();
        widget.add_named(picker, Some("picker"));
        widget.add_named(&entry, Some("name"));
        widget.set_visible_child(picker);
        let namer = Rc::new(Namer {
            widget,
            picker: picker.clone(),
            entry,
            lister: lister.clone(),
            going: RefCell::default(),
        });
        namer.wire();
        namer
    }

    /// Whether a name is being typed.
    pub fn naming(&self) -> bool {
        self.going.borrow().is_some()
    }

    /// Begins naming `naming`, for `list_id` unless it names a new list: the entry takes the
    /// picker's place and the focus, holding `text` selected. A naming going on ends first, with
    /// nothing changed.
    pub fn start(&self, naming: Naming, list_id: Option<&str>, text: &str) {
        self.end(false);
        let words = match naming {
            Naming::NewFolder(_) => FOLDER_NAME,
            Naming::NewList | Naming::Rename { .. } => LIST_NAME,
        };
        self.entry.set_placeholder_text(Some(words));
        self.entry
            .update_property(&[gtk::accessible::Property::Label(words)]);
        self.entry.set_text(text);
        let list_id = list_id.map(str::to_owned);
        self.going.replace(Some(Going { naming, list_id }));
        self.widget.set_visible_child(&self.entry);
        if !self.entry.grab_focus_without_selecting() {
            glib::g_warning!("fermix", "the Lists pane's name field took no focus");
        }
        self.entry.select_region(0, -1);
    }

    /// The pane shows `list_id` now, none with no lists: a rename or a new folder of another list
    /// ends, with nothing changed.
    pub fn shows(&self, list_id: Option<&str>) {
        let going = self.going.borrow();
        let named_for = going.as_ref().and_then(|going| going.list_id.as_deref());
        let other = named_for.is_some_and(|named_for| Some(named_for) != list_id);
        drop(going);
        if other {
            self.end(false);
        }
    }

    /// Ends a naming going on with nothing sent: the pane hid.
    pub fn cancel(&self) {
        self.end(false);
    }

    /// Ends the naming going on: kept, the op its name makes goes to the session, if it makes
    /// one; otherwise nothing is sent. The picker comes back, and takes the focus when the entry
    /// held it, the window's focus cleared first as GTK would move it a frame later.
    fn end(&self, kept: bool) {
        let Some(going) = self.going.take() else {
            return;
        };
        let typed = self.entry.text();
        let held = self.holds_focus();
        if let Some(root) = self.entry.root().filter(|_| held) {
            root.set_focus(None::<&gtk::Widget>);
        }
        self.widget.set_visible_child(&self.picker);
        if held {
            self.picker.grab_focus();
        }
        let Some(op) = kept.then(|| named(&going.naming, &typed)).flatten() else {
            return;
        };
        match asked(op, going.list_id) {
            Some(asked) => act(&self.lister, asked),
            None => glib::g_warning!("fermix", "a name for a list was typed with no list"),
        }
    }

    /// Whether the focus is in the entry.
    fn holds_focus(&self) -> bool {
        let focus = self.entry.root().and_then(|root| root.focus());
        let entry = self.entry.upcast_ref::<gtk::Widget>();
        focus.is_some_and(|focus| focus == *entry || focus.is_ancestor(entry))
    }

    /// Return keeps the name; Escape, or the focus leaving, changes nothing.
    fn wire(self: &Rc<Self>) {
        let namer = Rc::downgrade(self);
        self.entry.connect_activate(move |_| {
            if let Some(namer) = namer.upgrade() {
                namer.end(true);
            }
        });
        leave_on_escape(&self.entry, Rc::downgrade(self));
        nothing_on_leaving(&self.entry, Rc::downgrade(self));
    }
}

/// What the session is asked to do with the op a name made: a new list, or an edit of the list
/// named for. None for an edit with no list, which no naming makes.
fn asked(op: ListOp, list_id: Option<String>) -> Option<ListAct> {
    match (op, list_id) {
        (ListOp::Create { title, folder }, _) => Some(ListAct::Create { title, folder }),
        (op, Some(list_id)) => Some(ListAct::Edit { list_id, op }),
        (_, None) => None,
    }
}

/// Escape in the entry ends the naming with nothing changed, before the pane over a narrow chat
/// takes it as its close. It is taken as Escape comes back up from the entry's text, after its
/// input method, as a rewording's Escape is (`list_reword`), so an Escape that cancels a preedit
/// leaves the naming going.
fn leave_on_escape(entry: &gtk::Entry, namer: Weak<Namer>) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, state| {
        let plain = (state & gtk::accelerator_get_default_mod_mask()).is_empty();
        let escape = plain && key == gtk::gdk::Key::Escape;
        let Some(namer) = namer.upgrade().filter(|namer| escape && namer.naming()) else {
            return glib::Propagation::Proceed;
        };
        namer.end(false);
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

/// The focus leaving the entry ends the naming with nothing changed, at the next idle, unless the
/// focus is back in the entry by then. A window losing the focus is no leaving.
fn nothing_on_leaving(entry: &gtk::Entry, namer: Weak<Namer>) {
    let focus = gtk::EventControllerFocus::new();
    let watched = entry.downgrade();
    focus.connect_leave(move |_| {
        if !watched.upgrade().is_some_and(|entry| still_active(&entry)) {
            return;
        }
        let namer = namer.clone();
        at_idle(move || {
            if let Some(namer) = namer.upgrade().filter(|namer| !namer.holds_focus()) {
                namer.end(false);
            }
        });
    });
    entry.add_controller(focus);
}
