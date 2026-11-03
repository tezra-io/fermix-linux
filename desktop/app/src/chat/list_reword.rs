//! Rewording an item in place (M85 lists §2.2, §3.5, §6.1): the item's row gives way to an entry
//! for its words and, in the pane, one for its note, "Add a note". Return in either saves both,
//! Escape leaves with nothing changed, and focus leaving the pair saves, as a `gtk::EditableLabel`
//! does; a window losing the focus is no leaving. Only what changed from the item as the rewording
//! began is sent, as core's `Library::reword` measures it. The entries are kept while the item is
//! reworded, so a push that draws the list again leaves what is typed, and where, as it was.
//! Markup off before any text, and no edit is an exported action: the save goes to the session
//! through `Lister`.

use super::lists::{act, ListAct, Lister};
use crate::fields::still_active;
use adw::prelude::*;
use fermix_client::chat_wire::protocol::ListRow;
use fermix_client::checklists::draft::is_local;
use fermix_client::checklists::words::{ADD_NOTE, ITEM};
use gtk::glib;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// One item being reworded.
pub struct Rewording {
    list_id: String,
    /// The item as the rewording began: what changed is measured from it.
    was: ListRow,
    pub words: adw::EntryRow,
    /// The note's entry, in the pane; a card rewords the words alone.
    pub note: Option<adw::EntryRow>,
}

impl Rewording {
    pub fn row_id(&self) -> &str {
        &self.was.row_id
    }

    /// The pair's entries, the words first.
    pub fn entries(&self) -> impl Iterator<Item = &adw::EntryRow> {
        std::iter::once(&self.words).chain(&self.note)
    }

    /// The entry holding the window's focus, if one does.
    pub fn focused(&self) -> Option<&adw::EntryRow> {
        let focus = self.words.root()?.focus()?;
        self.entries()
            .find(|entry| focus.is_ancestor(*entry) || focus == *entry.upcast_ref::<gtk::Widget>())
    }

    /// The pair saved: the item as it began, and what the entries hold now.
    fn saved(&self) -> ListAct {
        ListAct::Reword {
            list_id: self.list_id.clone(),
            began: self.was.clone(),
            words: self.words.text().into(),
            note: self.note.as_ref().map(|note| note.text().into()),
        }
    }
}

/// The one rewording of a card or of the pane, if any. Its entries' closures hold it weakly, so
/// a card or pane dropped mid-rewording drops it too.
#[derive(Clone)]
pub struct Reworder {
    current: Rc<RefCell<Option<Rc<Rewording>>>>,
    lister: Lister,
}

/// A `Reworder` as its entries' closures hold it.
#[derive(Clone)]
struct WeakReworder {
    current: Weak<RefCell<Option<Rc<Rewording>>>>,
    lister: Lister,
}

impl WeakReworder {
    fn upgrade(&self) -> Option<Reworder> {
        let current = self.current.upgrade()?;
        let lister = self.lister.clone();
        Some(Reworder { current, lister })
    }
}

impl Reworder {
    pub fn new(lister: &Lister) -> Reworder {
        Reworder {
            current: Rc::default(),
            lister: lister.clone(),
        }
    }

    pub fn current(&self) -> Option<Rc<Rewording>> {
        self.current.borrow().clone()
    }

    /// The id of the item being reworded.
    pub fn row_id(&self) -> Option<String> {
        self.current()
            .map(|rewording| rewording.row_id().to_owned())
    }

    /// The words' entry while `row_id` is reworded.
    pub fn words_of(&self, row_id: &str) -> Option<adw::EntryRow> {
        let current = self.current()?;
        (current.row_id() == row_id).then(|| current.words.clone())
    }

    /// The note's entry while `row_id` is reworded in the pane.
    pub fn note_of(&self, row_id: &str) -> Option<adw::EntryRow> {
        let current = self.current()?;
        current.note.clone().filter(|_| current.row_id() == row_id)
    }

    /// Rewords `row` of `list_id`, its note too where `with_note`: a rewording going on is saved
    /// first, as its focus is leaving; the lists are drawn again with the entries in the row's
    /// place, and the words' entry takes the focus. A row the engine has not named yet is not
    /// reworded.
    pub fn start(&self, list_id: &str, row: &ListRow, with_note: bool) {
        if is_local(&row.row_id) {
            return;
        }
        self.save_going();
        let rewording = Rc::new(Rewording {
            list_id: list_id.to_owned(),
            was: row.clone(),
            words: entry(None, &row.text),
            note: with_note.then(|| entry(Some(ADD_NOTE), row.note.as_deref().unwrap_or(""))),
        });
        self.wire(&rewording);
        self.current.replace(Some(rewording.clone()));
        act(&self.lister, ListAct::Redraw);
        rewording.words.grab_focus();
    }

    /// Saves the rewording going on, if any, as its focus leaving would: before another starts,
    /// and before the owner folds or opens a section or the done items, or a card opens the pane
    /// at a list.
    pub fn save_going(&self) {
        if let Some(going) = self.current() {
            self.end(&going, true);
        }
    }

    /// Ends `rewording` if it is the one going on: saved, it goes to the session, which sends
    /// what changed; otherwise nothing is sent. Either way the lists are drawn again with the
    /// item's row back.
    fn end(&self, rewording: &Rc<Rewording>, saved: bool) {
        let going = self.current().is_some_and(|c| Rc::ptr_eq(&c, rewording));
        if !going {
            return;
        }
        self.current.replace(None);
        let asked = if saved {
            rewording.saved()
        } else {
            ListAct::Redraw
        };
        act(&self.lister, asked);
    }

    /// The item is no longer drawn after a push: its rewording ends with nothing sent, and the
    /// draw under way puts what it puts in its place.
    pub fn dropped(&self) {
        self.current.replace(None);
    }

    fn downgrade(&self) -> WeakReworder {
        let current = Rc::downgrade(&self.current);
        let lister = self.lister.clone();
        WeakReworder { current, lister }
    }

    /// Return in either entry saves, Escape leaves with nothing changed, and focus leaving the
    /// pair saves.
    fn wire(&self, rewording: &Rc<Rewording>) {
        for entry in rewording.entries() {
            let (reworder, weak) = (self.downgrade(), Rc::downgrade(rewording));
            entry.connect_entry_activated(move |_| {
                if let (Some(reworder), Some(rewording)) = (reworder.upgrade(), weak.upgrade()) {
                    reworder.end(&rewording, true);
                }
            });
            leave_on_escape(entry, self.downgrade(), rewording);
            save_on_leaving(entry, self.downgrade(), rewording);
        }
    }
}

/// An entry of the pair, markup off before its text. The words' has no title, and a screen reader
/// names it "Item"; the note's is "Add a note".
fn entry(title: Option<&str>, text: &str) -> adw::EntryRow {
    let entry = adw::EntryRow::new();
    entry.set_use_markup(false);
    entry.set_title(title.unwrap_or(""));
    entry.set_text(text);
    entry.set_selectable(false);
    if title.is_none() {
        named_item(&entry);
    }
    entry
}

/// The words' entry has no title for its text to be labelled by, which would name it nothing: its
/// text is named "Item" instead.
fn named_item(entry: &adw::EntryRow) {
    let Some(text) = entry.delegate() else {
        return glib::g_warning!("fermix", "a rewording's entry has no text to name");
    };
    text.reset_relation(gtk::AccessibleRelation::LabelledBy);
    text.update_property(&[gtk::accessible::Property::Label(ITEM)]);
}

/// Escape in the entry ends the rewording with nothing changed, before the pane over a narrow
/// chat takes it as its close. It is taken as Escape comes back up from the entry's text, after its
/// input method, so an Escape that cancels a preedit leaves the rewording going. A popover of the
/// text, its context menu or its emoji chooser, is a native of its own, which this controller does
/// not hear, so Escape there closes the popover and the rewording stays.
fn leave_on_escape(entry: &adw::EntryRow, reworder: WeakReworder, rewording: &Rc<Rewording>) {
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(rewording);
    keys.connect_key_pressed(move |_, key, _, state| {
        let plain = (state & gtk::accelerator_get_default_mod_mask()).is_empty();
        if !plain || key != gtk::gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        let (Some(reworder), Some(rewording)) = (reworder.upgrade(), weak.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        reworder.end(&rewording, false);
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

/// Focus leaving the entry saves once a double-click's time has passed, unless the focus is back
/// in the pair by then: in its other entry, or in the pair again after a draw. Until then the
/// rows stay where they are, so the second press of a double-click on another item lands on that
/// item, whose rewording saves this one at once. A window losing the focus is no leaving.
fn save_on_leaving(entry: &adw::EntryRow, reworder: WeakReworder, rewording: &Rc<Rewording>) {
    let focus = gtk::EventControllerFocus::new();
    let (weak, watched) = (Rc::downgrade(rewording), entry.downgrade());
    focus.connect_leave(move |_| {
        let Some(entry) = watched.upgrade().filter(still_active) else {
            return;
        };
        let (reworder, weak) = (reworder.clone(), weak.clone());
        glib::timeout_add_local_once(double_click(&entry), move || {
            let (Some(reworder), Some(rewording)) = (reworder.upgrade(), weak.upgrade()) else {
                return;
            };
            if rewording.focused().is_none() {
                reworder.end(&rewording, true);
            }
        });
    });
    entry.add_controller(focus);
}

/// How long a double-click may take on this display; 400 ms where the setting cannot be read as
/// one.
fn double_click(widget: &adw::EntryRow) -> Duration {
    let ms = u64::try_from(widget.settings().gtk_double_click_time()).unwrap_or(400);
    Duration::from_millis(ms)
}
