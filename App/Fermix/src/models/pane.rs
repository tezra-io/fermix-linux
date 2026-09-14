//! The thirteen panes, in four groups.
//!
//! One table. The model routes and persists through it, the pane list draws it,
//! and a deep link from an attention row or a Doctor remediation lands on it.
//! The panes themselves are the daemon's: `settings.sections` names one per
//! section, and this table says which group each sits in, what the product
//! calls it, and whether its content is a descriptor or a hand-built surface.

use crate::copy::Key;
use crate::management::types::SettingsPane;
use crate::management::vocabulary::{pane_for_slug, pane_slug};

/// How a pane's content is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    /// Every row comes from `settings.get` through the one descriptor form.
    Descriptor,
    /// The descriptor form, under statements the platform owes a person before
    /// they touch a control (M38 section 8.6). One pane is this: Voice, whose
    /// controls open a microphone this platform does not gate.
    StatedDescriptor,
    /// The pane is hand-built, because its flows are interaction rather than
    /// values (M38 section 5.7).
    HandBuilt,
}

/// One pane's place in the product.
#[derive(Debug, Clone, Copy)]
pub struct PaneRow {
    pub pane: SettingsPane,
    pub group: Key,
    pub title: Key,
    pub kind: PaneKind,
}

/// The four groups, in order.
pub const GROUPS: &[Key] = &[
    Key::SettingsGroupAssistant,
    Key::SettingsGroupConnections,
    Key::SettingsGroupCapabilities,
    Key::SettingsGroupSystem,
];

/// The thirteen panes, in the order the sidebar lists them.
pub const PANES: &[PaneRow] = &[
    PaneRow {
        pane: SettingsPane::Providers,
        group: Key::SettingsGroupAssistant,
        title: Key::PaneProviders,
        kind: PaneKind::HandBuilt,
    },
    PaneRow {
        pane: SettingsPane::Personality,
        group: Key::SettingsGroupAssistant,
        title: Key::PanePersonality,
        kind: PaneKind::Descriptor,
    },
    PaneRow {
        pane: SettingsPane::Memory,
        group: Key::SettingsGroupAssistant,
        title: Key::PaneMemory,
        kind: PaneKind::Descriptor,
    },
    PaneRow {
        pane: SettingsPane::Channels,
        group: Key::SettingsGroupConnections,
        title: Key::PaneChannels,
        kind: PaneKind::HandBuilt,
    },
    PaneRow {
        pane: SettingsPane::Integrations,
        group: Key::SettingsGroupConnections,
        title: Key::PaneIntegrations,
        kind: PaneKind::HandBuilt,
    },
    PaneRow {
        pane: SettingsPane::Voice,
        group: Key::SettingsGroupCapabilities,
        title: Key::PaneVoice,
        kind: PaneKind::StatedDescriptor,
    },
    PaneRow {
        pane: SettingsPane::Meetings,
        group: Key::SettingsGroupCapabilities,
        title: Key::PaneMeetings,
        kind: PaneKind::HandBuilt,
    },
    PaneRow {
        pane: SettingsPane::Computer,
        group: Key::SettingsGroupCapabilities,
        title: Key::PaneComputer,
        kind: PaneKind::HandBuilt,
    },
    PaneRow {
        pane: SettingsPane::Coding,
        group: Key::SettingsGroupCapabilities,
        title: Key::PaneCodingAgents,
        kind: PaneKind::Descriptor,
    },
    PaneRow {
        pane: SettingsPane::Search,
        group: Key::SettingsGroupCapabilities,
        title: Key::PaneSearch,
        kind: PaneKind::Descriptor,
    },
    PaneRow {
        pane: SettingsPane::Images,
        group: Key::SettingsGroupCapabilities,
        title: Key::PaneImages,
        kind: PaneKind::Descriptor,
    },
    PaneRow {
        pane: SettingsPane::Sandbox,
        group: Key::SettingsGroupSystem,
        title: Key::PaneSandbox,
        kind: PaneKind::Descriptor,
    },
    PaneRow {
        pane: SettingsPane::Permissions,
        group: Key::SettingsGroupSystem,
        title: Key::PanePermissions,
        kind: PaneKind::HandBuilt,
    },
];

/// The first pane, which is what a window with nothing remembered opens on.
pub fn first() -> SettingsPane {
    PANES[0].pane
}

/// One pane's row.
pub fn row(pane: SettingsPane) -> Option<&'static PaneRow> {
    PANES.iter().find(|row| row.pane == pane)
}

/// How one pane is written down, for persistence and for a stack page name.
pub fn slug(pane: SettingsPane) -> Option<&'static str> {
    pane_slug(pane)
}

/// One pane, read back from what was persisted.
pub fn for_slug(slug: &str) -> Option<SettingsPane> {
    pane_for_slug(slug)
}

/// The panes of one group, in order.
pub fn of_group(group: Key) -> impl Iterator<Item = &'static PaneRow> {
    PANES.iter().filter(move |row| row.group == group)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_thirteen_panes_in_four_groups() {
        assert_eq!(PANES.len(), 13);
        assert_eq!(GROUPS.len(), 4);

        let grouped: usize = GROUPS.iter().map(|group| of_group(*group).count()).sum();
        assert_eq!(grouped, PANES.len(), "every pane sits in a group");
    }

    #[test]
    fn the_groups_hold_the_panes_the_redlines_name() {
        let names = |group: Key| -> Vec<Key> { of_group(group).map(|row| row.title).collect() };

        assert_eq!(
            names(Key::SettingsGroupAssistant),
            vec![Key::PaneProviders, Key::PanePersonality, Key::PaneMemory]
        );
        assert_eq!(
            names(Key::SettingsGroupConnections),
            vec![Key::PaneChannels, Key::PaneIntegrations]
        );
        assert_eq!(
            names(Key::SettingsGroupCapabilities),
            vec![
                Key::PaneVoice,
                Key::PaneMeetings,
                Key::PaneComputer,
                Key::PaneCodingAgents,
                Key::PaneSearch,
                Key::PaneImages,
            ]
        );
        assert_eq!(
            names(Key::SettingsGroupSystem),
            vec![Key::PaneSandbox, Key::PanePermissions]
        );
    }

    #[test]
    fn the_six_hand_built_panes_are_the_ones_the_design_names() {
        let hand_built: Vec<SettingsPane> = PANES
            .iter()
            .filter(|row| row.kind == PaneKind::HandBuilt)
            .map(|row| row.pane)
            .collect();

        assert_eq!(
            hand_built,
            vec![
                SettingsPane::Providers,
                SettingsPane::Channels,
                SettingsPane::Integrations,
                SettingsPane::Meetings,
                SettingsPane::Computer,
                SettingsPane::Permissions,
            ]
        );
    }

    #[test]
    fn one_pane_carries_a_statement_over_its_controls() {
        let stated: Vec<SettingsPane> = PANES
            .iter()
            .filter(|row| row.kind == PaneKind::StatedDescriptor)
            .map(|row| row.pane)
            .collect();

        assert_eq!(
            stated,
            vec![SettingsPane::Voice],
            "the microphone statement belongs above the controls that open one"
        );
    }

    #[test]
    fn every_pane_is_written_down_once_and_reads_back() {
        for row in PANES {
            let slug = slug(row.pane).expect("every pane in the table is spelled");
            assert_eq!(for_slug(slug), Some(row.pane));
        }
    }
}
