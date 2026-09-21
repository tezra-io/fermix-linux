//! The menu as `com.canonical.dbusmenu` serves it.
//!
//! This interface is not optional on the desktops we target. The GNOME
//! AppIndicator extension treats an item whose `Menu` property names no
//! dbusmenu object as not ready and never draws it: it re-reads the property
//! three times, a second apart, and then gives up without a message
//! (appIndicator.js lines 114 and 133, read on Pop!_OS 22.04). An icon that
//! never appears and a desktop with no tray look exactly alike, so the menu has
//! to exist before the icon can.
//!
//! Only part of the interface is implemented, and the part was measured rather
//! than guessed. The host's own client calls `GetLayout`, `GetGroupProperties`
//! and `Event`, and listens for `LayoutUpdated` and `ItemsPropertiesUpdated`.
//! It never calls `AboutToShow`, `AboutToShowGroup`, `GetProperty` or
//! `EventGroup`, though the specification defines them. Those are left
//! unimplemented rather than stubbed to return something agreeable, so that a
//! host which does call one gets a plain "unknown method" it can report,
//! instead of an empty menu it cannot explain.
//!
//! The layout is built here as a pure function of the rows, which is what makes
//! the fiddly half of this file testable without a bus: `(u(ia{sv}av))` nests a
//! struct inside a struct inside an array, and a signature that is wrong by one
//! level produces a menu that is silently empty.

use gtk4::glib::prelude::ToVariant;
use gtk4::glib::variant::{DictEntry, Variant};

use crate::copy;
use crate::tray::menu::{TrayCommand, TrayRow};

/// The root item's id. The specification fixes it at zero.
pub const ROOT_ID: i32 = 0;

/// The id of the row at `index` in the table.
///
/// Ids start at one because zero is the root. They are positional, which is
/// safe here only because the table does not change shape with the daemon's
/// state -- `menu::rows` returns the same rows in the same order for every
/// status word, and there is a test on that. If rows ever start appearing and
/// disappearing, these ids must become stable identities rather than offsets,
/// or a click will land on whatever moved into that position.
fn id_for(index: usize) -> i32 {
    i32::try_from(index)
        .unwrap_or(i32::MAX - 1)
        .saturating_add(1)
}

/// The properties one row publishes.
fn properties(row: &TrayRow) -> Vec<DictEntry<String, Variant>> {
    let mut entries = Vec::new();

    match row {
        TrayRow::Separator => {
            entries.push(DictEntry::new("type".to_string(), "separator".to_variant()));
        }
        TrayRow::State(key) => {
            entries.push(DictEntry::new(
                "label".to_string(),
                copy::text(*key).to_variant(),
            ));
            // The state line is a fact, not a thing to click. Disabled rather
            // than absent, because the answer belongs at the top of the menu.
            entries.push(DictEntry::new("enabled".to_string(), false.to_variant()));
        }
        TrayRow::Command(command) => {
            entries.push(DictEntry::new(
                "label".to_string(),
                copy::text(command.label()).to_variant(),
            ));
            entries.push(DictEntry::new("enabled".to_string(), true.to_variant()));
        }
    }

    entries.push(DictEntry::new("visible".to_string(), true.to_variant()));
    entries
}

/// One `(ia{sv}av)` item, with no children.
fn item(id: i32, row: &TrayRow) -> Variant {
    let props = Variant::array_from_iter::<DictEntry<String, Variant>>(
        properties(row).iter().map(ToVariant::to_variant),
    );
    let children = Variant::array_from_iter::<Variant>(std::iter::empty());

    Variant::tuple_from_iter([id.to_variant(), props, children])
}

/// The whole layout, as `GetLayout` returns it: `(u(ia{sv}av))`.
///
/// Revision and the root wrapper are part of the reply rather than of the menu,
/// which is why they live here and not in the table.
pub fn layout(revision: u32, rows: &[TrayRow]) -> Variant {
    let children: Vec<Variant> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| item(id_for(index), row).to_variant())
        .collect();

    let root_props = Variant::array_from_iter::<DictEntry<String, Variant>>([DictEntry::new(
        "children-display".to_string(),
        "submenu".to_variant(),
    )
    .to_variant()]);

    let root = Variant::tuple_from_iter([
        ROOT_ID.to_variant(),
        root_props,
        Variant::array_from_iter::<Variant>(children),
    ]);

    Variant::tuple_from_iter([revision.to_variant(), root])
}

/// Every row's properties, as `GetGroupProperties` replies: `(a(ia{sv}))`.
///
/// Wrapped in a tuple, like every D-Bus reply body: the reply is the method's
/// out-arguments, and one out-argument of type `a(ia{sv})` is a ONE-TUPLE
/// containing that array, not the bare array. Returning the array itself makes
/// GDBus refuse the reply with a CRITICAL about the value not being a tuple,
/// the host gets an error instead of a menu, and nothing in a pure-value test
/// can see it -- the array has exactly the right shape right up until it is put
/// on a bus. `tests/tray_bus.rs` found this, which is the whole argument for
/// that file existing.
///
/// The `ids` argument is ignored on purpose. The specification lets a host ask
/// for a subset, and the menu is nine flat rows: filtering would add a branch
/// whose wrong half returns a menu missing exactly the rows the host asked
/// about, and save nothing worth having.
pub fn group_properties(rows: &[TrayRow]) -> Variant {
    let entries: Vec<Variant> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let props = Variant::array_from_iter::<DictEntry<String, Variant>>(
                properties(row).iter().map(ToVariant::to_variant),
            );

            Variant::tuple_from_iter([id_for(index).to_variant(), props])
        })
        .collect();

    let array = Variant::array_from_iter::<(i32, Vec<DictEntry<String, Variant>>)>(entries);

    Variant::tuple_from_iter([array])
}

/// What the row with this id does, if it does anything.
///
/// Separators and the state line answer `None`, which is how a click on them
/// becomes nothing rather than becoming the wrong command.
pub fn command_for(rows: &[TrayRow], id: i32) -> Option<TrayCommand> {
    rows.iter()
        .enumerate()
        .find(|(index, _)| id_for(*index) == id)
        .and_then(|(_, row)| match row {
            TrayRow::Command(command) => Some(*command),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::home::StatusWord;
    use crate::tray::menu;

    fn rows() -> Vec<TrayRow> {
        menu::rows(StatusWord::Running)
    }

    /// The child at `index`, unboxed.
    ///
    /// The root's children are `av`, so each element is a boxed variant and
    /// reading it without unboxing walks into the box rather than the item.
    fn child(layout: &Variant, index: usize) -> Variant {
        layout
            .child_value(1)
            .child_value(2)
            .child_value(index)
            .as_variant()
            .expect("every child of an av is a boxed variant")
    }

    /// The value of one property of one row, if the row publishes it.
    fn property(layout: &Variant, index: usize, name: &str) -> Option<Variant> {
        let props = child(layout, index).child_value(1);

        (0..props.n_children()).find_map(|entry| {
            let entry = props.child_value(entry);
            (entry.child_value(0).str() == Some(name))
                .then(|| entry.child_value(1))?
                .as_variant()
        })
    }

    #[test]
    fn the_layout_carries_the_signature_the_host_parses() {
        // The whole reason this is a tested pure function: one level wrong here
        // and the host shows an empty menu with no error anywhere.
        let layout = layout(1, &rows());
        assert_eq!(layout.type_().as_str(), "(u(ia{sv}av))");
    }

    #[test]
    fn the_root_is_id_zero_and_holds_every_row_as_a_child() {
        let layout = layout(1, &rows());
        let root = layout.child_value(1);

        assert_eq!(root.child_value(0).get::<i32>(), Some(ROOT_ID));
        assert_eq!(root.child_value(2).n_children(), rows().len());
    }

    #[test]
    fn the_revision_is_returned_as_given() {
        assert_eq!(layout(7, &rows()).child_value(0).get::<u32>(), Some(7));
    }

    #[test]
    fn no_row_takes_the_roots_id() {
        // A row sharing the root's id makes the menu its own parent, which
        // reads to the host as a cycle.
        for index in 0..rows().len() {
            assert_ne!(id_for(index), ROOT_ID);
        }
    }

    #[test]
    fn every_row_has_a_distinct_id() {
        let mut ids: Vec<i32> = (0..rows().len()).map(id_for).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "two rows share an id");
    }

    #[test]
    fn the_group_properties_carry_the_signature_the_host_parses() {
        // The tuple is the reply body, not decoration. See the comment on
        // `group_properties`.
        assert_eq!(group_properties(&rows()).type_().as_str(), "(a(ia{sv}))");
    }

    #[test]
    fn the_group_properties_describe_every_row_under_its_layout_id() {
        // The two replies have to agree about which id is which row, or a click
        // lands on a row the host labelled differently.
        let rows = rows();
        let group = group_properties(&rows).child_value(0);

        assert_eq!(group.n_children(), rows.len());
        for index in 0..rows.len() {
            assert_eq!(
                group.child_value(index).child_value(0).get::<i32>(),
                Some(id_for(index))
            );
        }
    }

    #[test]
    fn a_click_on_a_row_reaches_that_rows_command() {
        let rows = rows();
        for (index, row) in rows.iter().enumerate() {
            if let TrayRow::Command(expected) = row {
                assert_eq!(
                    command_for(&rows, id_for(index)),
                    Some(*expected),
                    "id {} did not reach {expected:?}",
                    id_for(index)
                );
            }
        }
    }

    #[test]
    fn a_click_on_a_separator_or_the_state_line_does_nothing() {
        let rows = rows();
        for (index, row) in rows.iter().enumerate() {
            match row {
                TrayRow::Command(_) => {}
                _ => assert_eq!(
                    command_for(&rows, id_for(index)),
                    None,
                    "a non-command row at {index} answered a command"
                ),
            }
        }
    }

    #[test]
    fn an_id_no_row_carries_does_nothing_rather_than_falling_through() {
        // The absent-name control. An unknown id must answer None, not the
        // first row or the last one.
        let rows = rows();
        assert_eq!(command_for(&rows, ROOT_ID), None);
        assert_eq!(command_for(&rows, 9_999), None);
        assert_eq!(command_for(&rows, -1), None);
    }

    #[test]
    fn the_state_line_is_disabled_and_the_commands_are_not() {
        let rows = rows();
        let layout = layout(1, &rows);

        for (index, row) in rows.iter().enumerate() {
            let enabled = property(&layout, index, "enabled").and_then(|value| value.get::<bool>());

            match row {
                TrayRow::State(_) => {
                    assert_eq!(enabled, Some(false), "the state line is clickable")
                }
                TrayRow::Command(_) => assert_eq!(enabled, Some(true), "a command is disabled"),
                TrayRow::Separator => {}
            }
        }
    }

    #[test]
    fn a_separator_says_so_in_its_type() {
        let rows = rows();
        let layout = layout(1, &rows);

        let separators = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, TrayRow::Separator))
            .count();
        assert!(separators > 0, "the table has no separators to check");

        for (index, row) in rows.iter().enumerate() {
            if !matches!(row, TrayRow::Separator) {
                continue;
            }

            let kind = property(&layout, index, "type").and_then(|value| value.get::<String>());

            assert_eq!(kind.as_deref(), Some("separator"));
        }
    }
}
