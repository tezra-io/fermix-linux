//! The list's menu in the Lists pane (M85 lists §3.6), behind More, between the picker and Hide
//! lists: Move to folder ▸ each folder then No folder, the list's own checked; Copy as text; Clear
//! done. More makes the menu each time it opens, from what the pane draws then, and the menu acts
//! through an action group of its own, inserted on More, which GTK does not export (§4.2). A
//! folder's name in a menu item has each `_` doubled, as a menu item reads `_` as a mnemonic.

use adw::prelude::*;
use fermix_client::checklists::pane::ListOffers;
use fermix_client::checklists::words::{self, CLEAR_DONE, COPY_AS_TEXT, MOVE_TO_FOLDER, NO_FOLDER};
use gtk::{gio, glib};
use std::rc::Rc;

/// The prefix the list menu's actions are named under, on More.
const LIST_GROUP: &str = "list";
/// More's tooltip and its name to a screen reader: the app's own word, as on a place card.
const MORE: &str = "More";

/// What the list's menu does for the list it opened over.
pub struct ListActs {
    /// Moves the list to the folder picked, empty for No folder.
    pub folder: Rc<dyn Fn(&str)>,
    pub copy: Rc<dyn Fn()>,
    pub clear_done: Rc<dyn Fn()>,
}

/// More: a flat button that opens the list's menu, hidden until the pane draws a list.
pub fn more_button() -> gtk::MenuButton {
    let more = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(MORE)
        .css_classes(["flat"])
        .visible(false)
        .build();
    more.update_property(&[gtk::accessible::Property::Label(MORE)]);
    more
}

/// The list's menu: Move to folder ▸ each folder in picker order, then No folder, each a radio
/// item on the folder the list is in; then Copy as text and Clear done.
pub fn list_menu(offers: &ListOffers) -> gio::Menu {
    let folders = gio::Menu::new();
    for name in offers.folders.iter().map(String::as_str).chain([""]) {
        let label = match name {
            "" => NO_FOLDER.to_owned(),
            name => words::menu_name(name),
        };
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some("list.folder"), Some(&name.to_variant()));
        folders.append_item(&item);
    }
    let menu = gio::Menu::new();
    menu.append_submenu(Some(MOVE_TO_FOLDER), &folders);
    menu.append(Some(COPY_AS_TEXT), Some("list.copy"));
    menu.append(Some(CLEAR_DONE), Some("list.clear-done"));
    menu
}

/// The list menu's actions. `folder` holds the list's folder as its state, which checks that
/// item, and moves the list to the folder it names. `copy` and `clear-done` are on as `offers`
/// says. Each action takes effect at the next idle, once the menu has closed; a folder picked with
/// Space closes it too.
pub fn list_actions(offers: &ListOffers, acts: ListActs) -> gio::SimpleActionGroup {
    let group = gio::SimpleActionGroup::new();
    let state = offers.folder.to_variant();
    let folder = gio::SimpleAction::new_stateful("folder", Some(glib::VariantTy::STRING), &state);
    let to_folder = acts.folder;
    folder.connect_activate(move |_, picked| {
        let Some(picked) = picked.and_then(glib::Variant::str).map(str::to_owned) else {
            return glib::g_warning!("fermix", "Move to folder named no folder");
        };
        let to_folder = to_folder.clone();
        glib::idle_add_local_once(move || to_folder(&picked));
    });
    group.add_action(&folder);
    let copy = gio::SimpleAction::new("copy", None);
    let clear_done = gio::SimpleAction::new("clear-done", None);
    for (action, on, act) in [
        (&copy, offers.copy, acts.copy),
        (&clear_done, offers.clear_done, acts.clear_done),
    ] {
        action.set_enabled(on);
        action.connect_activate(move |_, _| {
            let act = act.clone();
            glib::idle_add_local_once(move || act());
        });
        group.add_action(action);
    }
    group
}

/// Makes More's menu each time it opens from `open`: the list's offers and what its items do. With
/// none, which the pane hides More for, it opens nothing and says so in the log.
pub fn menu_on(
    more: &gtk::MenuButton,
    open: impl Fn() -> Option<(ListOffers, ListActs)> + 'static,
) {
    more.set_create_popup_func(move |more| {
        let Some((offers, acts)) = open() else {
            glib::g_warning!("fermix", "More opened while the pane offers no list menu");
            more.set_menu_model(None::<&gio::MenuModel>);
            more.set_active(false);
            return;
        };
        let actions = list_actions(&offers, acts);
        more.insert_action_group(LIST_GROUP, Some(&actions));
        more.set_menu_model(Some(&list_menu(&offers)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// An item's attribute, none past the menu's items: GMenu reads past them unchecked.
    fn text(menu: &gio::MenuModel, at: i32, attribute: &str) -> Option<String> {
        if at >= menu.n_items() {
            return None;
        }
        let value = menu.item_attribute_value(at, attribute, None)?;
        value.str().map(str::to_owned)
    }

    /// The submenu the menu's first item opens, if the menu has an item.
    fn first_submenu(menu: &gio::MenuModel) -> Option<gio::MenuModel> {
        (menu.n_items() > 0)
            .then(|| menu.item_link(0, "submenu"))
            .flatten()
    }

    /// Each item's label, its action and its target, in a menu.
    fn items(menu: &gio::MenuModel) -> Vec<(String, String, String)> {
        (0..menu.n_items())
            .map(|at| {
                let label = text(menu, at, "label").unwrap_or_default();
                let action = text(menu, at, "action").unwrap_or_default();
                let target = text(menu, at, "target").unwrap_or_default();
                (label, action, target)
            })
            .collect()
    }

    fn triple(label: &str, action: &str, target: &str) -> (String, String, String) {
        (label.into(), action.into(), target.into())
    }

    fn acts() -> ListActs {
        ListActs {
            folder: Rc::new(|_| {}),
            copy: Rc::new(|| {}),
            clear_done: Rc::new(|| {}),
        }
    }

    fn offers(folders: &[&str], folder: &str, copy: bool, clear_done: bool) -> ListOffers {
        ListOffers {
            folders: folders.iter().map(|name| (*name).to_owned()).collect(),
            folder: folder.into(),
            moving_to: None,
            copy,
            clear_done,
        }
    }

    fn checked(actions: &gio::SimpleActionGroup) -> Option<String> {
        let state = actions.action_state("folder")?;
        state.str().map(str::to_owned)
    }

    /// Move to folder holds each folder in picker order, its `_` doubled as a menu item reads `_`
    /// as a mnemonic, then No folder, whose pick is empty; the list's own folder is checked (M85
    /// lists §3.6). Copy as text and Clear done follow.
    #[test]
    fn move_to_folder_names_each_folder_as_written_then_no_folder_and_checks_the_lists_own() {
        let offered = offers(&["Home", "Home_work"], "Home_work", true, true);
        let menu: gio::MenuModel = list_menu(&offered).upcast();
        assert_eq!(
            items(&menu),
            [
                triple("Move to folder", "", ""),
                triple("Copy as text", "list.copy", ""),
                triple("Clear done", "list.clear-done", ""),
            ]
        );
        let folders = first_submenu(&menu).expect("Move to folder opens the folders");
        assert_eq!(
            items(&folders),
            [
                triple("Home", "list.folder", "Home"),
                triple("Home__work", "list.folder", "Home_work"),
                triple("No folder", "list.folder", ""),
            ]
        );
        assert_eq!(text(&folders, 2, "target").as_deref(), Some(""));
        let actions = list_actions(&offered, acts());
        assert_eq!(checked(&actions).as_deref(), Some("Home_work"));
        assert_eq!(
            actions.action_parameter_type("folder").as_deref(),
            Some(glib::VariantTy::STRING)
        );
    }

    /// Copy as text and Clear done are on as offered; a list in no folder checks No folder.
    #[test]
    fn copy_and_clear_done_are_on_as_offered_and_a_list_in_no_folder_checks_no_folder() {
        let offered = offers(&[], "", false, false);
        let actions = list_actions(&offered, acts());
        assert!(!actions.is_action_enabled("copy"));
        assert!(!actions.is_action_enabled("clear-done"));
        assert!(actions.is_action_enabled("folder"));
        assert_eq!(checked(&actions).as_deref(), Some(""));
        let menu: gio::MenuModel = list_menu(&offered).upcast();
        let folders = first_submenu(&menu).expect("the folders");
        assert_eq!(items(&folders), [triple("No folder", "list.folder", "")]);

        let actions = list_actions(&offers(&["Home"], "", true, false), acts());
        assert!(actions.is_action_enabled("copy"));
        assert!(!actions.is_action_enabled("clear-done"));
        let actions = list_actions(&offers(&["Home"], "", true, true), acts());
        assert!(actions.is_action_enabled("clear-done"));
    }

    /// Acts that write each call into `calls`, a folder's as "folder:" and its name.
    fn recording(calls: &Rc<RefCell<Vec<String>>>) -> ListActs {
        let (folder, copy, clear_done) = (calls.clone(), calls.clone(), calls.clone());
        ListActs {
            folder: Rc::new(move |picked| folder.borrow_mut().push(format!("folder:{picked}"))),
            copy: Rc::new(move || copy.borrow_mut().push("copy".into())),
            clear_done: Rc::new(move || clear_done.borrow_mut().push("clear_done".into())),
        }
    }

    /// Each action does its own act with the folder picked, and only at the next idle: nothing
    /// has run when it is activated, and after at most ten turns of the default main context,
    /// which this test holds so no other test runs its idles, each has run once, in order.
    #[test]
    fn each_action_does_its_own_act_with_the_folder_picked_at_the_next_idle() {
        let context = glib::MainContext::default();
        let _held = context
            .acquire()
            .expect("the default main context is free for this test");
        let calls: Rc<RefCell<Vec<String>>> = Rc::default();
        let actions = list_actions(
            &offers(&["Home", "Home_work"], "Home", true, true),
            recording(&calls),
        );
        actions.activate_action("copy", None);
        actions.activate_action("clear-done", None);
        actions.activate_action("folder", Some(&"Home_work".to_variant()));
        actions.activate_action("folder", Some(&"".to_variant()));
        assert!(
            calls.borrow().is_empty(),
            "{:?} ran at once",
            calls.borrow()
        );
        for _ in 0..10 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        assert_eq!(
            *calls.borrow(),
            ["copy", "clear_done", "folder:Home_work", "folder:"]
        );
    }
}
