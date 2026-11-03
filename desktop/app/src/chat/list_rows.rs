//! The rows both lists surfaces draw (M85 lists §4.1): an item, as a reply's card and the Lists
//! pane draw it, with its settle after a tick, and a row or button row that acts on a single
//! click or Return in a list that activates its rows on a double-click. Markup off before any
//! text, and no list edit is an exported action: a tick is the check button's own closure, which
//! hands it to the session through `Lister`.

use super::lists::{act, ListAct, Lister};
use adw::prelude::*;
use fermix_client::chat_wire::protocol::{ListOp, ListRow};
use fermix_client::checklists::card::Settling;
use fermix_client::checklists::draft::is_local;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// How long a ticked row keeps its place before it folds away, with animations on.
const SETTLE: Duration = Duration::from_millis(700);
/// The mark of a list, on a card's head and a folded line's buttons.
pub const LIST_MARK: &str = "checkbox-checked-symbolic";

/// An item of `list_id`: its words, with its note under them where `note` asks (the pane), and a
/// check that ticks it, which takes no focus; Space on the row ticks it too. Done items are
/// dimmed, and a row added here that the engine has not named yet takes no tick. A row ticked
/// done keeps its place in `settling` a moment, with animations on, then the lists are drawn
/// again without it there.
pub fn item_row(
    row: &ListRow,
    note: bool,
    list_id: &str,
    lister: &Lister,
    settling: &Rc<RefCell<Settling>>,
) -> adw::ActionRow {
    let item = adw::ActionRow::new();
    item.set_use_markup(false);
    item.set_title(&row.text);
    item.set_title_lines(0);
    let note = row.note.as_deref().filter(|text| note && !text.is_empty());
    if let Some(note) = note {
        item.set_subtitle(note);
    }
    let check = gtk::CheckButton::new();
    check.set_active(row.is_done());
    check.set_focusable(false);
    check.set_valign(gtk::Align::Center);
    item.add_prefix(&check);
    if row.is_done() {
        item.add_css_class("dimmed");
    }
    item.set_sensitive(!is_local(&row.row_id));
    spoken(&item, row, note);
    let ticks = Ticks {
        list_id: list_id.to_owned(),
        row_id: row.row_id.clone(),
        lister: lister.clone(),
        settling: settling.clone(),
    };
    tick_on_toggle(&check, ticks);
    tick_on_space(&item, &check);
    item
}

/// To a screen reader an item is its row: the words, the note as its description, and its done
/// state as Checked.
fn spoken(item: &adw::ActionRow, row: &ListRow, note: Option<&str>) {
    let spoken = item.upcast_ref::<gtk::Widget>();
    spoken.update_property(&[gtk::accessible::Property::Label(&row.text)]);
    if let Some(note) = note {
        spoken.update_property(&[gtk::accessible::Property::Description(note)]);
    }
    let checked = match row.is_done() {
        true => gtk::AccessibleTristate::True,
        false => gtk::AccessibleTristate::False,
    };
    spoken.update_state(&[gtk::accessible::State::Checked(checked)]);
}

/// What a tick names and where it goes.
struct Ticks {
    list_id: String,
    row_id: String,
    lister: Lister,
    settling: Rc<RefCell<Settling>>,
}

/// The check's own closure sends the tick, after a row ticked done starts to settle.
fn tick_on_toggle(check: &gtk::CheckButton, ticks: Ticks) {
    check.connect_toggled(move |check| {
        let done = check.is_active();
        if done && check.settings().is_gtk_enable_animations() {
            let lister = ticks.lister.clone();
            settle(&ticks.settling, &ticks.row_id, move || {
                act(&lister, ListAct::Redraw)
            });
        }
        let row_id = ticks.row_id.clone();
        let op = match done {
            true => ListOp::Check { row_id },
            false => ListOp::Uncheck { row_id },
        };
        let list_id = ticks.list_id.clone();
        act(&ticks.lister, ListAct::Edit { list_id, op });
    });
}

/// Space on a focused item ticks it, before the row's own Space activates it.
fn tick_on_space(item: &adw::ActionRow, check: &gtk::CheckButton) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let check = check.downgrade();
    keys.connect_key_pressed(move |_, key, _, state| {
        let plain = (state & gtk::accelerator_get_default_mod_mask()).is_empty();
        let Some(check) = check
            .upgrade()
            .filter(|_| plain && key == gtk::gdk::Key::space)
        else {
            return glib::Propagation::Proceed;
        };
        check.set_active(!check.is_active());
        glib::Propagation::Stop
    });
    item.add_controller(keys);
}

/// Keeps `row_id` in its place for `SETTLE`, then, unless a later settle of it is running,
/// `settled` draws the lists without it there.
fn settle(settling: &Rc<RefCell<Settling>>, row_id: &str, settled: impl FnOnce() + 'static) {
    settling.borrow_mut().begin(row_id);
    let (settling, row_id) = (settling.clone(), row_id.to_owned());
    glib::timeout_add_local_once(SETTLE, move || {
        let ended = settling.borrow_mut().end(&row_id);
        if ended {
            settled();
        }
    });
}

/// A row that acts on a single click, Return or Space, in a list that activates its rows on a
/// double-click (M85 lists §2.6, §3.4). Its own click gesture takes both clicks of a double-click,
/// so the list never activates it on the second; a key activates it through the list. `acted`
/// learns whether a key did it.
pub fn on_click_or_key(row: &adw::ActionRow, acted: impl Fn(bool) + 'static) {
    row.set_activatable(true);
    let acted = Rc::new(acted);
    let by_key = acted.clone();
    row.connect_activated(move |_| by_key(true));
    acts_on_press(row.upcast_ref(), move || acted(false));
}

/// A button row that acts on a single click, Return or Space, as `on_click_or_key`'s row does: a
/// one-list card's "N more" and "Show less" (M85 lists §2.6).
pub fn button_on_click_or_key(row: &adw::ButtonRow, acted: impl Fn() + 'static) {
    let acted = Rc::new(acted);
    let by_key = acted.clone();
    row.connect_activated(move |_| by_key());
    acts_on_press(row.upcast_ref(), move || acted());
}

/// `row`'s own click gesture takes every press, so its list never sees one, and acts on the first.
fn acts_on_press(row: &gtk::Widget, acted: impl Fn() + 'static) {
    let click = gtk::GestureClick::new();
    click.connect_pressed(move |click, presses, _, _| {
        click.set_state(gtk::EventSequenceState::Claimed);
        if presses == 1 {
            acted();
        }
    });
    row.add_controller(click);
}

/// A button that acts when pressed, by a click or a key. `acted` learns whether a key did it:
/// GtkButton's activate is a key's (Space or Return), and the clicked that ends it comes after.
pub fn on_press(button: &gtk::Button, acted: impl Fn(bool) + 'static) {
    let keyed = Rc::new(Cell::new(false));
    let key = keyed.clone();
    button.connect_activate(move |_| key.set(true));
    button.connect_clicked(move |_| acted(keyed.replace(false)));
}
