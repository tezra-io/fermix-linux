//! The rows under the tray icon, as a table rather than as code.
//!
//! The table is a pure value so the order, the labels and the actions can be
//! asserted without a bus, a host or a desktop. Everything that actually talks
//! to D-Bus reads this and performs no decisions of its own.
//!
//! The first row is the state line: a disabled row that says what Fermix is
//! doing. It is disabled because it is a fact, not a thing to click, and it
//! carries the finer five-way answer the three-way glyph cannot.

use crate::copy::Key;
use crate::models::home::StatusWord;

/// What a tray row does when it is clicked.
///
/// Named as the `GAction` it activates rather than as a closure, so the table
/// stays a value. Every one of these must be an application-level action:
/// the whole point of the tray is that it works with no window open, and a
/// `win.` action with no window goes nowhere at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    /// Raise the window and show Home, creating the window if none is open.
    OpenFermix,
    /// Raise the window and show Settings.
    OpenSettings,
    /// Raise the window and show Doctor.
    RunDoctor,
    /// Restart the daemon.
    RestartDaemon,
    /// Quit, which releases the hold and ends the process.
    Quit,
}

impl TrayCommand {
    /// The action this row activates.
    pub fn action(self) -> &'static str {
        match self {
            TrayCommand::OpenFermix => "app.home",
            TrayCommand::OpenSettings => "app.tray-settings",
            TrayCommand::RunDoctor => "app.tray-doctor",
            TrayCommand::RestartDaemon => "app.tray-restart",
            TrayCommand::Quit => "app.quit",
        }
    }

    /// The product's words for this row.
    pub fn label(self) -> Key {
        match self {
            TrayCommand::OpenFermix => Key::BackToFermix,
            TrayCommand::OpenSettings => Key::MenuSettings,
            TrayCommand::RunDoctor => Key::MenuRunDoctor,
            TrayCommand::RestartDaemon => Key::MenuRestart,
            TrayCommand::Quit => Key::MenuQuit,
        }
    }
}

/// One row of the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayRow {
    /// The disabled state line, whose words come from the status word.
    State(Key),
    /// A divider.
    Separator,
    /// A row that does something.
    Command(TrayCommand),
}

/// The menu for a status word, top to bottom.
///
/// The grouping follows the Mac's: what to open, then what to do to the
/// service, then the way out. Quit sits alone at the bottom because with the
/// hold taken it is the only thing that ends the process, and a Quit adjacent
/// to Restart is a Quit that gets clicked by accident.
pub fn rows(status: StatusWord) -> Vec<TrayRow> {
    vec![
        TrayRow::State(status.key()),
        TrayRow::Separator,
        TrayRow::Command(TrayCommand::OpenFermix),
        TrayRow::Command(TrayCommand::OpenSettings),
        TrayRow::Command(TrayCommand::RunDoctor),
        TrayRow::Separator,
        TrayRow::Command(TrayCommand::RestartDaemon),
        TrayRow::Separator,
        TrayRow::Command(TrayCommand::Quit),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(status: StatusWord) -> Vec<TrayCommand> {
        rows(status)
            .into_iter()
            .filter_map(|row| match row {
                TrayRow::Command(command) => Some(command),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_is_the_five_rows_the_design_names_in_order() {
        assert_eq!(
            commands(StatusWord::Running),
            vec![
                TrayCommand::OpenFermix,
                TrayCommand::OpenSettings,
                TrayCommand::RunDoctor,
                TrayCommand::RestartDaemon,
                TrayCommand::Quit,
            ]
        );
    }

    #[test]
    fn the_first_row_is_the_state_line_and_it_follows_the_status_word() {
        // The state line is the one row that changes with the daemon, and it
        // carries the distinction the glyph cannot draw.
        assert_eq!(
            rows(StatusWord::NotRunning).first(),
            Some(&TrayRow::State(StatusWord::NotRunning.key()))
        );
        assert_eq!(
            rows(StatusWord::SetupRequired).first(),
            Some(&TrayRow::State(StatusWord::SetupRequired.key()))
        );
    }

    #[test]
    fn every_status_word_produces_a_state_line_with_words_behind_it() {
        for status in [
            StatusWord::Running,
            StatusWord::Unknown,
            StatusWord::SetupRequired,
            StatusWord::RestartToFinishUpdating,
            StatusWord::NotRunning,
        ] {
            match rows(status).first() {
                Some(TrayRow::State(key)) => assert_eq!(*key, status.key()),
                other => panic!("{status:?} gave {other:?} rather than a state line"),
            }
        }
    }

    #[test]
    fn the_rows_offered_do_not_change_with_the_daemons_state() {
        // A menu whose rows come and go under the pointer is how a click lands
        // on the row that moved into place. The state line says what is true;
        // the rows stay put and refuse when they cannot act.
        let running = commands(StatusWord::Running);
        for status in [
            StatusWord::Unknown,
            StatusWord::SetupRequired,
            StatusWord::RestartToFinishUpdating,
            StatusWord::NotRunning,
        ] {
            assert_eq!(commands(status), running, "rows moved for {status:?}");
        }
    }

    #[test]
    fn every_command_activates_an_application_action() {
        // A `win.` action with no window open does nothing and says nothing,
        // which is precisely the case the tray exists to serve.
        for command in commands(StatusWord::Running) {
            let action = command.action();
            assert!(
                action.starts_with("app."),
                "{command:?} activates {action}, which is not an application action"
            );
        }
    }

    #[test]
    fn no_two_commands_share_an_action() {
        let mut actions: Vec<&str> = commands(StatusWord::Running)
            .iter()
            .map(|command| command.action())
            .collect();
        let before = actions.len();
        actions.sort_unstable();
        actions.dedup();
        assert_eq!(before, actions.len(), "two rows activate the same action");
    }

    #[test]
    fn quit_is_last_and_is_not_adjacent_to_restart() {
        let rows = rows(StatusWord::Running);
        assert_eq!(rows.last(), Some(&TrayRow::Command(TrayCommand::Quit)));

        let quit = rows.len() - 1;
        assert_eq!(
            rows.get(quit - 1),
            Some(&TrayRow::Separator),
            "Quit must be separated from what sits above it"
        );
    }
}
