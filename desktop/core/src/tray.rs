//! The tray icon, as the macOS status item has it (StatusMenu.swift,
//! CommandTable.statusItem): a glyph that follows the daemon, a status line
//! at the top of its menu, and the app's commands under it. Only rules and
//! words live here; the D-Bus item is the app's. The status words are Home's
//! and the background row is Home's switch, so the two never disagree.

use crate::model::SetupState;
use crate::overview::duration_words;
use crate::service::BackgroundSwitch;
use crate::view::{status_word, DaemonProblem};

/// The glyph's three states, each its own image, as on macOS: the mark, the
/// mark in a lighter ink, and the mark with a badge cut into it. None of them
/// moves, and none is told apart by colour alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Running,
    Starting,
    Attention,
}

impl Glyph {
    /// The symbolic icon the package exports, which the desktop tints for its panel.
    pub fn icon_name(self) -> &'static str {
        match self {
            Glyph::Running => "io.tezra.Fermix-tray-symbolic",
            Glyph::Starting => "io.tezra.Fermix-tray-starting-symbolic",
            Glyph::Attention => "io.tezra.Fermix-tray-attention-symbolic",
        }
    }

    /// What a screen reader says for the glyph.
    pub fn label(self) -> &'static str {
        match self {
            Glyph::Running => "Fermix is running",
            Glyph::Starting => "Fermix is starting",
            Glyph::Attention => "Fermix needs attention",
        }
    }
}

/// The daemon as the tray sees it.
pub enum Daemon<'a> {
    /// Nothing has been read yet.
    Reading,
    /// A start or restart this app asked for is being waited on.
    Waking,
    Down(&'a DaemonProblem),
    Up {
        state: &'a SetupState,
        uptime_ms: Option<u64>,
    },
}

pub struct TrayFacts<'a> {
    pub daemon: Daemon<'a>,
    /// The companion window is on screen.
    pub pet_shown: bool,
    /// Home's "Run in the background" switch, as drawn.
    pub background: &'a BackgroundSwitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    OpenFermix,
    Settings,
    Doctor,
    /// Asks first, in the window, as Home's Restart does.
    Restart,
    /// systemd's restart, for a daemon that does not answer.
    RestartService,
    StartService,
    TogglePet,
    ToggleBackground,
    Quit,
}

impl Command {
    /// Whether the row shows the window before it acts: it opens a page, or it
    /// may ask a question there (a restart, turning the service off).
    pub fn needs_window(self) -> bool {
        matches!(
            self,
            Command::OpenFermix
                | Command::Settings
                | Command::Doctor
                | Command::Restart
                | Command::ToggleBackground
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub command: Command,
    pub label: &'static str,
    pub enabled: bool,
    /// `Some` for a row with a tick, and whether it is ticked.
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// The daemon's condition in words, a row that is read rather than clicked.
    Status(String),
    Separator,
    Item(Item),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrayView {
    pub glyph: Glyph,
    pub status: String,
    pub rows: Vec<Row>,
}

pub fn tray_view(facts: &TrayFacts) -> TrayView {
    let (glyph, status) = condition(&facts.daemon);
    let rows = vec![
        Row::Status(status.clone()),
        Row::Separator,
        plain(Command::OpenFermix, "Open Fermix", true),
        plain(Command::Settings, "Settings", true),
        plain(Command::Doctor, "Run Doctor", true),
        Row::Separator,
        restart_row(&facts.daemon),
        ticked(Command::TogglePet, "Show Pet", facts.pet_shown, true),
        ticked(
            Command::ToggleBackground,
            "Run in the Background",
            facts.background.on,
            facts.background.sensitive,
        ),
        Row::Separator,
        plain(Command::Quit, "Quit", true),
    ];
    TrayView {
        glyph,
        status,
        rows,
    }
}

fn condition(daemon: &Daemon) -> (Glyph, String) {
    match daemon {
        Daemon::Reading => (Glyph::Starting, "Reading from Fermix…".into()),
        Daemon::Waking => (Glyph::Starting, "Starting…".into()),
        Daemon::Down(problem) => (Glyph::Attention, problem.status_word().into()),
        Daemon::Up { state, .. } if state.readiness.status != "ready" || state.restart.required => {
            (Glyph::Attention, status_word(state).into())
        }
        Daemon::Up { uptime_ms, .. } => {
            let line = match uptime_ms.filter(|ms| *ms > 0) {
                Some(ms) => format!("Running for {}", duration_words(ms)),
                None => "Running".into(),
            };
            (Glyph::Running, line)
        }
    }
}

/// What Home offers for the daemon's condition: start it, restart it through
/// systemd when it does not answer, or the asked restart when it does.
fn restart_row(daemon: &Daemon) -> Row {
    match daemon {
        Daemon::Down(DaemonProblem::NotRunning) => {
            plain(Command::StartService, "Start Fermix", true)
        }
        Daemon::Down(DaemonProblem::NotResponding | DaemonProblem::Broken(_)) => {
            plain(Command::RestartService, "Restart Fermix", true)
        }
        Daemon::Up { .. } => plain(Command::Restart, "Restart Fermix…", true),
        Daemon::Reading | Daemon::Waking | Daemon::Down(DaemonProblem::UpdateNeeded(_)) => {
            plain(Command::Restart, "Restart Fermix…", false)
        }
    }
}

fn plain(command: Command, label: &'static str, enabled: bool) -> Row {
    Row::Item(Item {
        command,
        label,
        enabled,
        checked: None,
    })
}

fn ticked(command: Command, label: &'static str, on: bool, enabled: bool) -> Row {
    Row::Item(Item {
        command,
        label,
        enabled,
        checked: Some(on),
    })
}
