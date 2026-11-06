//! The list's menu in the Lists pane (M85 lists §3.6), behind More, between the picker and Hide
//! lists: Rename list; Move to folder ▸ each folder then No folder, the list's own checked, then
//! New folder; Copy as text; Clear done; Recently removed, checked while shown; Remove list. More
//! makes the menu each time it opens, from what the pane draws then, and the menu acts through an
//! action group of its own, inserted on More, which GTK does not export (§4.2). A folder's name in
//! a menu item has each `_` doubled, as a menu item reads `_` as a mnemonic.

use super::list_menu::at_idle;
use adw::prelude::*;
use fermix_client::checklists::pane::ListOffers;
use fermix_client::checklists::words::{
    self, CLEAR_DONE, COPY_AS_TEXT, MOVE_TO_FOLDER, NEW_FOLDER, NO_FOLDER, RECENTLY_REMOVED,
    REMOVE_LIST, RENAME_LIST,
};
use gtk::{gio, glib};
use std::rc::Rc;

/// The prefix the list menu's actions are named under, on More.
const LIST_GROUP: &str = "list";
/// More's tooltip and its name to a screen reader: the app's own word, as on a place card.
const MORE: &str = "More";

/// What the list's menu does for the list it opened over.
pub struct ListActs {
    /// Rename list: names the list anew in the head.
    pub rename: Rc<dyn Fn()>,
    /// Moves the list to the folder picked, empty for No folder.
    pub folder: Rc<dyn Fn(&str)>,
    /// New folder: names a new folder for the list in the head.
    pub new_folder: Rc<dyn Fn()>,
    pub copy: Rc<dyn Fn()>,
    pub clear_done: Rc<dyn Fn()>,
    /// Recently removed: shows the list's removed items, or hides them (§3.4, §10).
    pub removed: Rc<dyn Fn()>,
    /// Remove list, with no confirmation: a toast offers Undo (§3.7, §10).
    pub remove_list: Rc<dyn Fn()>,
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

/// The list's menu: Rename list; Move to folder ▸ each folder in picker order, then No folder,
/// each a radio item on the folder the list is in, then New folder in a section of its own; then
/// Copy as text, Clear done, and Recently removed, a check item; then Remove list, last, in a
/// section of its own.
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
    let new_folder = gio::Menu::new();
    new_folder.append(Some(NEW_FOLDER), Some("list.new-folder"));
    folders.append_section(None, &new_folder);
    let menu = gio::Menu::new();
    menu.append(Some(RENAME_LIST), Some("list.rename"));
    menu.append_submenu(Some(MOVE_TO_FOLDER), &folders);
    menu.append(Some(COPY_AS_TEXT), Some("list.copy"));
    menu.append(Some(CLEAR_DONE), Some("list.clear-done"));
    menu.append(Some(RECENTLY_REMOVED), Some("list.removed"));
    let remove = gio::Menu::new();
    remove.append(Some(REMOVE_LIST), Some("list.remove-list"));
    menu.append_section(None, &remove);
    menu
}

/// The list menu's actions. `folder` holds the list's folder as its state, which checks that
/// item, and moves the list to the folder it names. `removed` holds whether Recently removed is
/// shown, which checks its item, and turns it the other way. `rename`, `new-folder`, `removed` and
/// `remove-list` are always on, as More shows only over a list drawn that is not removed; `copy`
/// and `clear-done` are on as `offers` says. Each action takes effect at the next idle, once the
/// menu has closed; a folder picked with Space closes it too.
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
        at_idle(move || to_folder(&picked));
    });
    group.add_action(&folder);
    group.add_action(&removed_action(offers.removed_shown, acts.removed));
    let rename = gio::SimpleAction::new("rename", None);
    let new_folder = gio::SimpleAction::new("new-folder", None);
    let copy = gio::SimpleAction::new("copy", None);
    let clear_done = gio::SimpleAction::new("clear-done", None);
    let remove_list = gio::SimpleAction::new("remove-list", None);
    for (action, on, act) in [
        (&rename, true, acts.rename),
        (&new_folder, true, acts.new_folder),
        (&copy, offers.copy, acts.copy),
        (&clear_done, offers.clear_done, acts.clear_done),
        (&remove_list, true, acts.remove_list),
    ] {
        action.set_enabled(on);
        action.connect_activate(move |_, _| {
            let act = act.clone();
            at_idle(move || act());
        });
        group.add_action(action);
    }
    group
}

/// Recently removed, a check item: its state is whether it is shown, which each activation turns
/// the other way at once, and through `act` at the next idle. Space keeps a check item's menu open
/// (GtkModelButton), so the check turns while it shows, until the act's redraw changes the list's
/// offers and the pane closes the menu as stale.
fn removed_action(shown: bool, act: Rc<dyn Fn()>) -> gio::SimpleAction {
    let removed = gio::SimpleAction::new_stateful("removed", None, &shown.to_variant());
    removed.connect_activate(move |removed, _| {
        let now = removed.state().and_then(|state| state.get::<bool>());
        removed.set_state(&(!now.unwrap_or(shown)).to_variant());
        let act = act.clone();
        at_idle(move || act());
    });
    removed
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

    /// The menu an item links to as `link` ("submenu" or "section"), if the menu has that item.
    fn linked(menu: &gio::MenuModel, at: i32, link: &str) -> Option<gio::MenuModel> {
        (at < menu.n_items())
            .then(|| menu.item_link(at, link))
            .flatten()
    }

    /// The submenu of the item labelled `label`.
    fn submenu(menu: &gio::MenuModel, label: &str) -> Option<gio::MenuModel> {
        let at =
            (0..menu.n_items()).find(|at| text(menu, *at, "label").as_deref() == Some(label))?;
        linked(menu, at, "submenu")
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

    /// The folders' radio items in Move to folder.
    fn radios(folders: &gio::MenuModel) -> Vec<(String, String, String)> {
        let mut radios = items(folders);
        radios.retain(|(_, action, _)| action == "list.folder");
        radios
    }

    fn triple(label: &str, action: &str, target: &str) -> (String, String, String) {
        (label.into(), action.into(), target.into())
    }

    fn acts() -> ListActs {
        ListActs {
            rename: Rc::new(|| {}),
            folder: Rc::new(|_| {}),
            new_folder: Rc::new(|| {}),
            copy: Rc::new(|| {}),
            clear_done: Rc::new(|| {}),
            removed: Rc::new(|| {}),
            remove_list: Rc::new(|| {}),
        }
    }

    fn offers(folders: &[&str], folder: &str, copy: bool, clear_done: bool) -> ListOffers {
        ListOffers {
            folders: folders.iter().map(|name| (*name).to_owned()).collect(),
            folder: folder.into(),
            moving_to: None,
            copy,
            clear_done,
            removed_shown: false,
        }
    }

    fn checked(actions: &gio::SimpleActionGroup) -> Option<String> {
        let state = actions.action_state("folder")?;
        state.str().map(str::to_owned)
    }

    /// Move to folder holds each folder in picker order, its `_` doubled as a menu item reads `_`
    /// as a mnemonic, then No folder, whose pick is empty; the list's own folder is checked (M85
    /// lists §3.6).
    #[test]
    fn move_to_folder_names_each_folder_as_written_then_no_folder_and_checks_the_lists_own() {
        let offered = offers(&["Home", "Home_work"], "Home_work", true, true);
        let menu: gio::MenuModel = list_menu(&offered).upcast();
        let folders = submenu(&menu, MOVE_TO_FOLDER).expect("Move to folder opens the folders");
        assert_eq!(
            radios(&folders),
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

    /// Rename list leads the menu, then Move to folder, Copy as text, Clear done and Recently
    /// removed, and Remove list ends it, in a section of its own; New folder ends Move to folder,
    /// after No folder, in a section of its own. The four are on whatever the list offers, as More
    /// shows only over a list drawn that is not removed (§3.6, §10).
    #[test]
    fn rename_list_leads_the_menu_new_folder_ends_move_to_folder_and_remove_list_the_menu() {
        let offered = offers(&["Home"], "Home", false, false);
        let menu: gio::MenuModel = list_menu(&offered).upcast();
        assert_eq!(
            items(&menu),
            [
                triple("Rename list", "list.rename", ""),
                triple("Move to folder", "", ""),
                triple("Copy as text", "list.copy", ""),
                triple("Clear done", "list.clear-done", ""),
                triple("Recently removed", "list.removed", ""),
                triple("", "", ""),
            ]
        );
        let remove = linked(&menu, 5, "section").expect("Remove list's section");
        assert_eq!(
            items(&remove),
            [triple("Remove list", "list.remove-list", "")]
        );
        let folders = submenu(&menu, MOVE_TO_FOLDER).expect("the folders");
        assert_eq!(folders.n_items(), 3, "Home, No folder, then a section");
        let section = linked(&folders, 2, "section").expect("New folder's section");
        assert_eq!(
            items(&section),
            [triple("New folder", "list.new-folder", "")]
        );
        let actions = list_actions(&offered, acts());
        assert!(actions.is_action_enabled("rename"));
        assert!(actions.is_action_enabled("new-folder"));
        assert!(actions.is_action_enabled("removed"));
        assert!(actions.is_action_enabled("remove-list"));
    }

    /// Recently removed is a check item, checked while it is shown: its action has a boolean state
    /// and no parameter, and an activation turns the check at once, while its act waits for the
    /// next idle (§3.6, §10).
    #[test]
    fn recently_removed_is_a_check_item_checked_while_shown() {
        let removed = |actions: &gio::SimpleActionGroup| {
            let state = actions.action_state("removed");
            state.and_then(|state| state.get::<bool>())
        };
        let hidden = list_actions(&offers(&[], "", true, false), acts());
        assert_eq!(removed(&hidden), Some(false));
        assert_eq!(hidden.action_parameter_type("removed"), None);
        let mut offered = offers(&[], "", true, false);
        offered.removed_shown = true;
        let shown = list_actions(&offered, acts());
        assert_eq!(removed(&shown), Some(true));
        on_own_loop(|_| {
            shown.activate_action("removed", None);
            assert_eq!(removed(&shown), Some(false), "the check turns at once");
            shown.activate_action("removed", None);
            assert_eq!(removed(&shown), Some(true));
        });
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
        let folders = submenu(&menu, MOVE_TO_FOLDER).expect("the folders");
        assert_eq!(radios(&folders), [triple("No folder", "list.folder", "")]);

        let actions = list_actions(&offers(&["Home"], "", true, false), acts());
        assert!(actions.is_action_enabled("copy"));
        assert!(!actions.is_action_enabled("clear-done"));
        let actions = list_actions(&offers(&["Home"], "", true, true), acts());
        assert!(actions.is_action_enabled("clear-done"));
    }

    /// Acts that write each call into `calls`, a folder's as "folder:" and its name.
    fn recording(calls: &Rc<RefCell<Vec<String>>>) -> ListActs {
        let (rename, folder, new_folder) = (calls.clone(), calls.clone(), calls.clone());
        let (copy, clear_done, remove_list) = (calls.clone(), calls.clone(), calls.clone());
        let removed = calls.clone();
        ListActs {
            rename: Rc::new(move || rename.borrow_mut().push("rename".into())),
            folder: Rc::new(move |picked| folder.borrow_mut().push(format!("folder:{picked}"))),
            new_folder: Rc::new(move || new_folder.borrow_mut().push("new_folder".into())),
            copy: Rc::new(move || copy.borrow_mut().push("copy".into())),
            clear_done: Rc::new(move || clear_done.borrow_mut().push("clear_done".into())),
            removed: Rc::new(move || removed.borrow_mut().push("removed".into())),
            remove_list: Rc::new(move || remove_list.borrow_mut().push("remove_list".into())),
        }
    }

    /// Runs `test` with a main context of its own as this thread's default: no other test's idles
    /// run in it, and none of its run in theirs.
    fn on_own_loop(test: impl FnOnce(&glib::MainContext)) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| test(&context))
            .expect("a new context is free to acquire");
    }

    /// Each action does its own act with the folder picked, and only at the next idle of the
    /// thread's default main context: nothing has run when it is activated, and after at most ten
    /// turns of the context, each has run once, in order.
    #[test]
    fn each_action_does_its_own_act_with_the_folder_picked_at_the_next_idle() {
        on_own_loop(|context| {
            let calls: Rc<RefCell<Vec<String>>> = Rc::default();
            let actions = list_actions(
                &offers(&["Home", "Home_work"], "Home", true, true),
                recording(&calls),
            );
            actions.activate_action("rename", None);
            actions.activate_action("copy", None);
            actions.activate_action("clear-done", None);
            actions.activate_action("folder", Some(&"Home_work".to_variant()));
            actions.activate_action("new-folder", None);
            actions.activate_action("folder", Some(&"".to_variant()));
            actions.activate_action("removed", None);
            actions.activate_action("remove-list", None);
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
                [
                    "rename",
                    "copy",
                    "clear_done",
                    "folder:Home_work",
                    "new_folder",
                    "folder:",
                    "removed",
                    "remove_list"
                ]
            );
        });
    }
}
