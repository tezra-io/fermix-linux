//! The one action map.
//!
//! Every action the product has is a row below, with its accelerator beside it.
//! The primary menu, the shortcuts dialog and every notification bind to these
//! rows, so an action is a menu item and a keyboard path by construction rather
//! than by discipline, and nothing is reachable from only one place.

use crate::copy::Key;

/// Which group of the shortcuts dialog a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    General,
    Navigation,
    Actions,
}

impl Group {
    /// The group's heading.
    pub fn title(self) -> Key {
        match self {
            Group::General => Key::ShortcutsGroupGeneral,
            Group::Navigation => Key::ShortcutsGroupNavigation,
            Group::Actions => Key::ShortcutsGroupActions,
        }
    }
}

/// One action, its accelerators and the words for it.
#[derive(Debug, Clone, Copy)]
pub struct ActionSpec {
    /// The fully qualified name, `app.*` or `win.*`.
    pub name: &'static str,
    /// Every accelerator that reaches it.
    pub accels: &'static [&'static str],
    /// How the shortcuts dialog names it.
    pub label: Key,
    /// Which group of the shortcuts dialog it sits in.
    pub group: Group,
}

/// Every action in the product.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "app.quit",
        accels: &["<Control>q"],
        label: Key::ShortcutQuit,
        group: Group::General,
    },
    ActionSpec {
        name: "app.about",
        accels: &[],
        label: Key::ShortcutAbout,
        group: Group::General,
    },
    // The one route a notification takes. It is an application action because a
    // notification is delivered to the application rather than to a window, and
    // it is in this table because the notification binds to the same map the
    // menus and the accelerators do.
    ActionSpec {
        name: "app.home",
        accels: &[],
        label: Key::ShortcutShowHome,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "app.shortcuts",
        accels: &["<Control>question"],
        label: Key::ShortcutKeyboardShortcuts,
        group: Group::General,
    },
    ActionSpec {
        name: "win.close",
        accels: &["<Control>w"],
        label: Key::ShortcutCloseWindow,
        group: Group::General,
    },
    ActionSpec {
        name: "win.settings",
        accels: &["<Control>comma"],
        label: Key::ShortcutOpenSettings,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "win.home",
        accels: &["<Control>1"],
        label: Key::ShortcutShowHome,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "win.doctor",
        accels: &["<Control>2"],
        label: Key::ShortcutShowDoctor,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "win.logs",
        accels: &["<Control>3"],
        label: Key::ShortcutShowLogs,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "win.back",
        accels: &["Escape", "<Alt>Left"],
        label: Key::ShortcutGoBack,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "win.toggle-sidebar",
        accels: &["F9"],
        label: Key::ShortcutToggleSidebar,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "win.search",
        accels: &["<Control>f"],
        label: Key::ShortcutSearchSurface,
        group: Group::Navigation,
    },
    ActionSpec {
        name: "win.run-doctor",
        accels: &["<Control>r"],
        label: Key::ShortcutRunDoctor,
        group: Group::Actions,
    },
    ActionSpec {
        name: "win.restart",
        // Modifier order is the toolkit's, not this table's: GTK normalises an
        // accelerator name to Shift, Lock, Control, Alt, and reads back what it
        // normalised. Spelling it the other way round round-trips to a
        // different string and the binding gate says so.
        accels: &["<Shift><Control>r"],
        label: Key::ShortcutRestart,
        group: Group::Actions,
    },
];

/// The primary menu, in the order the redlines fix: Settings, Run Doctor,
/// Restart Fermix, Keyboard Shortcuts, About Fermix, Quit. Each row names an
/// action from [`ACTIONS`] and the word the menu uses for it, which is not
/// always the word the shortcuts dialog uses.
pub const PRIMARY_MENU: &[(&str, Key)] = &[
    ("win.settings", Key::MenuSettings),
    ("win.run-doctor", Key::MenuRunDoctor),
    ("win.restart", Key::MenuRestart),
    ("app.shortcuts", Key::MenuKeyboardShortcuts),
    ("app.about", Key::MenuAbout),
    ("app.quit", Key::MenuQuit),
];

/// The rows of one shortcuts group, in table order.
pub fn group(group: Group) -> impl Iterator<Item = &'static ActionSpec> {
    ACTIONS
        .iter()
        .filter(move |spec| spec.group == group && listed(spec))
}

/// Whether the shortcuts reference lists one action.
///
/// It lists what a person can reach from the keyboard or from the menu. An
/// action that is neither — the route a notification takes back into the
/// application — is still in this one map, so nothing is reachable from only
/// one place, and it is not a keyboard path to print.
pub fn listed(spec: &ActionSpec) -> bool {
    !spec.accels.is_empty() || PRIMARY_MENU.iter().any(|(name, _)| *name == spec.name)
}

/// One action's row.
pub fn spec(name: &str) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.name == name)
}

/// The window actions, without their `win.` prefix, which is how a window adds
/// them to itself.
pub fn window_action_names() -> impl Iterator<Item = &'static str> {
    ACTIONS
        .iter()
        .filter_map(|spec| spec.name.strip_prefix("win."))
}

/// The application actions, without their `app.` prefix.
pub fn application_action_names() -> impl Iterator<Item = &'static str> {
    ACTIONS
        .iter()
        .filter_map(|spec| spec.name.strip_prefix("app."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_action_is_prefixed_and_named_once() {
        let mut seen = BTreeSet::new();
        for spec in ACTIONS {
            assert!(
                spec.name.starts_with("app.") || spec.name.starts_with("win."),
                "{} has no scope",
                spec.name
            );
            assert!(seen.insert(spec.name), "{} appears twice", spec.name);
        }
    }

    #[test]
    fn no_accelerator_is_bound_to_two_actions() {
        let mut seen = BTreeSet::new();
        for spec in ACTIONS {
            for accel in spec.accels {
                assert!(seen.insert(*accel), "{accel} is bound twice");
            }
        }
    }

    #[test]
    fn every_menu_row_names_an_action_that_exists() {
        for (name, _) in PRIMARY_MENU {
            assert!(
                spec(name).is_some(),
                "{name} is in the menu and not in the table"
            );
        }
    }

    #[test]
    fn the_menu_is_the_six_rows_the_redlines_fix_in_their_order() {
        let names: Vec<&str> = PRIMARY_MENU.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec![
                "win.settings",
                "win.run-doctor",
                "win.restart",
                "app.shortcuts",
                "app.about",
                "app.quit",
            ]
        );
    }

    #[test]
    fn every_action_a_person_can_reach_lands_in_a_shortcuts_group() {
        let grouped: usize = [Group::General, Group::Navigation, Group::Actions]
            .into_iter()
            .map(|which| group(which).count())
            .sum();
        let reachable = ACTIONS.iter().filter(|spec| listed(spec)).count();

        assert_eq!(grouped, reachable);
        assert!(
            reachable < ACTIONS.len(),
            "the one action a notification takes is in the map and not a keyboard path"
        );
    }

    #[test]
    fn the_two_scopes_partition_the_table() {
        assert_eq!(
            window_action_names().count() + application_action_names().count(),
            ACTIONS.len()
        );
    }

    #[test]
    fn the_keyboard_paths_the_redlines_name_are_all_bound() {
        let required = [
            ("win.settings", "<Control>comma"),
            ("win.home", "<Control>1"),
            ("win.doctor", "<Control>2"),
            ("win.logs", "<Control>3"),
            ("win.search", "<Control>f"),
            ("win.run-doctor", "<Control>r"),
            ("win.restart", "<Shift><Control>r"),
            ("win.toggle-sidebar", "F9"),
            ("win.back", "Escape"),
            ("win.back", "<Alt>Left"),
            ("win.close", "<Control>w"),
            ("app.quit", "<Control>q"),
            ("app.shortcuts", "<Control>question"),
        ];

        for (name, accel) in required {
            let spec = spec(name).unwrap_or_else(|| panic!("{name} is missing"));
            assert!(
                spec.accels.contains(&accel),
                "{name} is not bound to {accel}"
            );
        }
    }
}
