//! An item's menu in the Lists pane (M85 lists §3.6): Move to list, with the other lists by
//! folder, then Move up and Move down. A right-click, a touch long press, the Menu key or
//! Shift+F10 opens it over the item, as GTK's GtkPopoverBin opens a context menu
//! (gtkpopoverbin.c:640-656); a popover bin cannot hold the item itself, which is its list's own
//! row. The menu acts through an action group of its own, inserted on its popover, which GTK does
//! not export (§4.2). A list's name in a menu item has each `_` doubled, as a menu item reads `_`
//! as a mnemonic; a folder's name heads its section as written.

use adw::prelude::*;
use fermix_client::chat_wire::protocol::ListEntry;
use fermix_client::checklists::index::Folders;
use fermix_client::checklists::words::{self, MOVE_DOWN, MOVE_TO_LIST, MOVE_UP};
use gtk::{gdk, gio, glib};
use std::cell::Cell;
use std::rc::Rc;

/// The prefix the item menu's actions are named under, on its popover.
const ITEM_GROUP: &str = "item";

/// What an item's menu offers now: the lists it can move to, and whether it steps up or down.
pub struct ItemOffers {
    pub others: Folders,
    pub up: bool,
    pub down: bool,
}

/// The item menu: Move to list ▸ the other lists, each folder's under its name and the lists in
/// no folder last; then Move up and Move down.
pub fn item_menu(others: &Folders) -> gio::Menu {
    let lists = gio::Menu::new();
    for folder in &others.folders {
        lists.append_section(Some(&folder.name), &list_items(&folder.lists));
    }
    if !others.loose.is_empty() {
        lists.append_section(None, &list_items(&others.loose));
    }
    let move_to = gio::MenuItem::new_submenu(Some(MOVE_TO_LIST), &lists);
    move_to.set_detailed_action("item.lists");
    let menu = gio::Menu::new();
    menu.append_item(&move_to);
    menu.append(Some(MOVE_UP), Some("item.up"));
    menu.append(Some(MOVE_DOWN), Some("item.down"));
    menu
}

/// One item for each list, its name with each `_` doubled, moving the item there.
fn list_items(lists: &[ListEntry]) -> gio::Menu {
    let items = gio::Menu::new();
    for entry in lists {
        let item = gio::MenuItem::new(Some(&words::menu_name(&entry.title)), None);
        item.set_action_and_target_value(Some("item.to-list"), Some(&entry.list_id.to_variant()));
        items.append_item(&item);
    }
    items
}

/// The item menu's actions. `lists` only turns Move to list on while there is another list: a
/// submenu's item is as sensitive as its action. `to-list` moves the item to the list it names,
/// and `up` and `down` step it, each on while the item steps that way. Each takes effect at the
/// next idle, once the menu that activated it has closed: it can draw the item's row again.
pub fn item_actions(
    offers: &ItemOffers,
    to_list: impl Fn(&str) + 'static,
    step: impl Fn(bool) + 'static,
) -> gio::SimpleActionGroup {
    let group = gio::SimpleActionGroup::new();
    let lists = gio::SimpleAction::new("lists", None);
    lists.set_enabled(!offers.others.is_empty());
    group.add_action(&lists);
    let to = gio::SimpleAction::new("to-list", Some(glib::VariantTy::STRING));
    let to_list = Rc::new(to_list);
    to.connect_activate(move |_, target| {
        let Some(list_id) = target.and_then(glib::Variant::str).map(str::to_owned) else {
            return glib::g_warning!("fermix", "Move to list named no list");
        };
        let to_list = to_list.clone();
        glib::idle_add_local_once(move || to_list(&list_id));
    });
    group.add_action(&to);
    let step = Rc::new(step);
    let up = gio::SimpleAction::new("up", None);
    let down = gio::SimpleAction::new("down", None);
    for (action, upward, on) in [(&up, true, offers.up), (&down, false, offers.down)] {
        action.set_enabled(on);
        let step = step.clone();
        action.connect_activate(move |_, _| {
            let step = step.clone();
            glib::idle_add_local_once(move || step(upward));
        });
        group.add_action(action);
    }
    group
}

/// Opens `menu` acting through `actions` over `row`: pointing at (x, y) in it after a pointer or
/// a touch, else under the whole row, as GtkPopoverBin does. The menu is named by the item's
/// words to a screen reader, and the row is drawn as hovered while the menu shows, by the stock
/// `has-open-popup` class GtkPopoverBin gives its child. The popover is the row's until it
/// closes, and goes at the next idle after, or with the row if a redraw takes the row first.
pub fn pop_up(
    row: &adw::ActionRow,
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    at: Option<(f64, f64)>,
) {
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.insert_action_group(ITEM_GROUP, Some(actions));
    popover.set_position(gtk::PositionType::Bottom);
    if let Some((x, y)) = at {
        let press = gdk::Rectangle::new(x as i32, y as i32, 0, 0);
        popover.set_pointing_to(Some(&press));
    }
    popover.update_property(&[gtk::accessible::Property::Label(&row.title())]);
    marks_open(&popover, row);
    popover.set_parent(row);
    let gone = popover.downgrade();
    let with_row = row.connect_destroy(move |_| {
        if let Some(popover) = gone.upgrade() {
            popover.unparent();
        }
    });
    let with_row = Cell::new(Some(with_row));
    let row = row.downgrade();
    popover.connect_closed(move |popover| {
        let (popover, row, with_row) = (popover.clone(), row.clone(), with_row.take());
        glib::idle_add_local_once(move || {
            if let (Some(row), Some(handler)) = (row.upgrade(), with_row) {
                row.disconnect(handler);
            }
            if popover.parent().is_some() {
                popover.unparent();
            }
        });
    });
    popover.popup();
}

/// While `popover` is mapped, `row` carries `has-open-popup`, as gtkpopoverbin.c:116-126 marks
/// its child: libadwaita draws an activatable row with it as hovered.
fn marks_open(popover: &gtk::PopoverMenu, row: &adw::ActionRow) {
    let marked = row.downgrade();
    popover.connect_map(move |_| {
        if let Some(row) = marked.upgrade() {
            row.add_css_class("has-open-popup");
        }
    });
    let marked = row.downgrade();
    popover.connect_unmap(move |_| {
        if let Some(row) = marked.upgrade() {
            row.remove_css_class("has-open-popup");
        }
    });
}

/// Wires `row` so `open` opens its menu on a right-click, a touch long press, the Menu key or
/// Shift+F10, as GtkPopoverBin wires its child (gtkpopoverbin.c:640-656). `open` learns where in
/// the row a pointer or a touch pressed, and `None` for a key.
pub fn menu_on(row: &adw::ActionRow, open: impl Fn(&adw::ActionRow, Option<(f64, f64)>) + 'static) {
    let open = Rc::new(open);
    let weak = row.downgrade();
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_exclusive(true);
    let (by_click, clicked) = (open.clone(), weak.clone());
    click.connect_pressed(move |click, _, x, y| {
        let sequence = click.current_sequence();
        let event = click.last_event(sequence.as_ref());
        let menu = event.is_some_and(|e| e.triggers_context_menu());
        let (Some(row), true) = (clicked.upgrade(), menu) else {
            click.set_state(gtk::EventSequenceState::Denied);
            return;
        };
        by_click(&row, Some((x, y)));
        click.set_state(gtk::EventSequenceState::Claimed);
        click.reset();
    });
    row.add_controller(click);
    let press = gtk::GestureLongPress::new();
    press.set_touch_only(true);
    press.set_exclusive(true);
    let (by_press, pressed) = (open.clone(), weak.clone());
    press.connect_pressed(move |press, x, y| {
        press.set_state(gtk::EventSequenceState::Claimed);
        if let Some(row) = pressed.upgrade() {
            by_press(&row, Some((x, y)));
        }
    });
    row.add_controller(press);
    let keys = gtk::ShortcutController::new();
    let trigger = gtk::AlternativeTrigger::new(
        gtk::KeyvalTrigger::new(gdk::Key::Menu, gdk::ModifierType::empty()),
        gtk::KeyvalTrigger::new(gdk::Key::F10, gdk::ModifierType::SHIFT_MASK),
    );
    let action = gtk::CallbackAction::new(move |_, _| {
        if let Some(row) = weak.upgrade() {
            open(&row, None);
        }
        glib::Propagation::Stop
    });
    keys.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
    row.add_controller(keys);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(list_id: &str, title: &str, folder: Option<&str>) -> ListEntry {
        ListEntry {
            list_id: list_id.into(),
            title: title.into(),
            folder: folder.map(str::to_owned),
            open: 1,
            done: 0,
            rev: 1,
        }
    }

    fn text(menu: &gio::MenuModel, at: i32, attribute: &str) -> Option<String> {
        let value = menu.item_attribute_value(at, attribute, None)?;
        value.str().map(str::to_owned)
    }

    /// Each item's label, its action and its target, in one section of a menu.
    fn items(section: &gio::MenuModel) -> Vec<(String, String, String)> {
        (0..section.n_items())
            .map(|at| {
                let label = text(section, at, "label").unwrap_or_default();
                let action = text(section, at, "action").unwrap_or_default();
                let target = text(section, at, "target").unwrap_or_default();
                (label, action, target)
            })
            .collect()
    }

    fn triple(label: &str, action: &str, target: &str) -> (String, String, String) {
        (label.into(), action.into(), target.into())
    }

    /// Move to list holds the other lists by folder, the lists in no folder last; a list's `_` is
    /// doubled in its item, as a menu item reads `_` as a mnemonic, and a folder's heading keeps
    /// it (M85 lists §3.6). Move up and Move down follow.
    #[test]
    fn move_to_list_names_the_other_lists_by_folder_then_move_up_and_move_down() {
        let others = Folders::of(&[
            entry("ck_a", "to_do", Some("Home_work")),
            entry("ck_b", "Groceries", None),
            entry("ck_c", "Packing", Some("Home_work")),
        ]);
        let menu: gio::MenuModel = item_menu(&others).upcast();

        assert_eq!(
            items(&menu),
            [
                triple("Move to list", "item.lists", ""),
                triple("Move up", "item.up", ""),
                triple("Move down", "item.down", ""),
            ]
        );
        let lists = menu
            .item_link(0, "submenu")
            .expect("Move to list opens the lists");
        assert_eq!(lists.n_items(), 2, "a folder, then the lists in no folder");
        assert_eq!(text(&lists, 0, "label").as_deref(), Some("Home_work"));
        assert_eq!(text(&lists, 1, "label"), None);
        let section = |at| lists.item_link(at, "section").expect("a section");
        assert_eq!(
            items(&section(0)),
            [
                triple("to__do", "item.to-list", "ck_a"),
                triple("Packing", "item.to-list", "ck_c"),
            ]
        );
        assert_eq!(
            items(&section(1)),
            [triple("Groceries", "item.to-list", "ck_b")]
        );
    }

    /// Move to list is off while there is no other list, and Move up and Move down while the item
    /// does not step that way.
    #[test]
    fn move_to_list_is_off_with_no_other_list_and_a_step_off_where_none_is_said() {
        let offers = ItemOffers {
            others: Folders::default(),
            up: false,
            down: true,
        };
        let actions = item_actions(&offers, |_| {}, |_| {});
        assert!(!actions.is_action_enabled("lists"));
        assert!(!actions.is_action_enabled("up"));
        assert!(actions.is_action_enabled("down"));
        assert!(actions.is_action_enabled("to-list"));
        assert_eq!(
            actions.action_parameter_type("to-list").as_deref(),
            Some(glib::VariantTy::STRING)
        );
        assert_eq!(
            item_menu(&offers.others)
                .item_link(0, "submenu")
                .map(|m| m.n_items()),
            Some(0)
        );

        let offers = ItemOffers {
            others: Folders::of(&[entry("ck_b", "Groceries", None)]),
            up: true,
            down: false,
        };
        let actions = item_actions(&offers, |_| {}, |_| {});
        assert!(actions.is_action_enabled("lists"));
        assert!(actions.is_action_enabled("up"));
        assert!(!actions.is_action_enabled("down"));
    }
}
