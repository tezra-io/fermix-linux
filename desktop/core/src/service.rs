//! The background service as systemd reports it (spec §6.2, §6.3), the
//! CLI's binding of it to a home, and the engine's own activation. Only words
//! and rules live here; the D-Bus calls and the command run are the app's.

use crate::capped::Ended;
use serde_json::Value;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the package installs the unit. Any other effective path shadows it.
pub const VENDOR_UNIT: &str = "/usr/lib/systemd/user/fermix.service";
/// The command that writes the binding and adopts or migrates the unit (M38 §4.5).
pub const INSTALL_COMMAND: &str = "fermix service install";
/// The command that starts the unit on an account that is already bound.
pub const START_COMMAND: &str = "systemctl --user start fermix";
/// The same command as the app runs it: its answer is one JSON envelope on
/// stdout, and its progress goes to stderr (M38 §4.6).
pub const INSTALL_ARGV: [&str; 4] = ["fermix", "service", "install", "--json"];
/// How long the app lets the command run. It waits up to 90 s for the
/// service's socket and then up to 90 s for its web address, and refuses with
/// its own sentence when either runs out; the cap only stops a command that
/// hangs, and stops everything it started with it (`capped::run`).
pub const INSTALL_CAP: Duration = Duration::from_secs(240);
pub const INSTALL_UNREADABLE: &str = "The answer from fermix service install is not one this \
                                      app understands. Run it in a terminal to see what it says.";
/// What linger is fixed with when the app may not set it.
pub const LINGER_COMMAND: &str = "loginctl enable-linger";
pub const BACKGROUND_NOTE: &str =
    "Fermix starts with your computer and keeps answering when this window is closed.";
/// The switch's note on an account with no binding yet, where turning it on
/// runs `fermix service install`.
pub const SETUP_NOTE: &str = "Turning this on sets Fermix up on this computer, which can take a \
                              minute or two. It then starts with your computer and keeps \
                              answering when this window is closed.";

/// The unit's properties, as the systemd user manager reports them.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitFacts {
    /// `loaded`, or `not-found` when the package is not installed.
    pub load_state: String,
    /// `enabled`, `disabled`, `masked`, …
    pub unit_file_state: String,
    /// `active`, `inactive`, `failed`, `activating`, `deactivating`.
    pub active_state: String,
    pub fragment_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceRead {
    /// The user's service manager did not answer, or this app may not reach it.
    Unreachable,
    /// `linger` is `None` when logind could not be asked.
    Unit {
        unit: UnitFacts,
        linger: Option<bool>,
    },
}

/// The "Run in the background" switch as drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundSwitch {
    pub on: bool,
    pub sensitive: bool,
    /// The row's subtitle: what the switch does, or why it cannot be used.
    pub note: String,
}

/// "Enabled · running · stays on after logout", or why there is no such line.
pub fn service_line(read: &ServiceRead) -> String {
    let (unit, linger) = match read {
        ServiceRead::Unreachable => return "Unavailable (service manager)".into(),
        ServiceRead::Unit { unit, linger } => (unit, linger),
    };
    if unit.load_state == "not-found" {
        return "Not installed".into();
    }
    let mut parts = vec![
        capitalized(&unit.unit_file_state),
        run_word(&unit.active_state),
    ];
    match linger {
        Some(true) => parts.push("stays on after logout".into()),
        Some(false) => parts.push("stops when you log out".into()),
        None => {}
    }
    parts.join(" · ")
}

fn run_word(active_state: &str) -> String {
    let word = match active_state {
        "active" => "running",
        "inactive" => "stopped",
        "activating" => "starting",
        "deactivating" => "stopping",
        other => other,
    };
    word.to_owned()
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown".into(),
    }
}

/// The switch from what systemd reports. `bound` means a binding is known to
/// exist (its file is there, or the daemon answered): without one, turning the
/// switch on runs the engine's own activation, `fermix service install`, which
/// writes the binding and enables the service. `busy` while a change runs.
pub fn background_switch(read: Option<&ServiceRead>, bound: bool, busy: bool) -> BackgroundSwitch {
    let (unit, linger) = match read {
        None => return waiting("Reading from your service manager…"),
        Some(ServiceRead::Unreachable) => {
            return waiting("This app cannot reach your service manager.")
        }
        Some(ServiceRead::Unit { unit, linger }) => (unit, *linger),
    };
    if unit.load_state == "not-found" {
        return waiting("The fermix package is not installed on this computer.");
    }
    // Enabled but stopping at logout is not in the background; turning the
    // switch on then repairs linger. An unread linger is not held against it.
    let on = unit.unit_file_state == "enabled" && linger != Some(false);
    let refused = |note: String| BackgroundSwitch {
        on,
        sensitive: false,
        note,
    };
    if unit.fragment_path != VENDOR_UNIT {
        return refused(format!(
            "Another Fermix service is in charge here. To hand it to this app, run \
             {INSTALL_COMMAND} in a terminal."
        ));
    }
    let note = if on || bound {
        BACKGROUND_NOTE
    } else {
        SETUP_NOTE
    };
    BackgroundSwitch {
        on,
        sensitive: !busy,
        note: note.into(),
    }
}

fn waiting(note: &str) -> BackgroundSwitch {
    BackgroundSwitch {
        on: false,
        sensitive: false,
        note: note.into(),
    }
}

/// The body of "Turn off the background service?". An unknown count stays unknown.
pub fn disable_warning(active: Option<u64>) -> String {
    let stop = "Fermix stops now and does not start again until you turn this back on. Your \
                settings, memory and history are kept.";
    match active {
        Some(0) => stop.to_owned(),
        Some(1) => format!("{stop} 1 conversation is in progress and would be interrupted."),
        Some(n) => format!("{stop} {n} conversations are in progress and would be interrupted."),
        None => format!("{stop} Fermix could not say whether a conversation is in progress."),
    }
}

/// The binding under the config home, as the CLI names it (`binding.ex`).
const BINDING_FILE: &str = "fermix/service.json";
const BINDING_SCHEMA: u64 = 1;
const ENVELOPE_SCHEMA: u64 = 1;
/// The longest socket path Linux accepts: `sun_path` holds 108 bytes with its NUL.
const SOCKET_PATH_MAX: usize = 107;

/// The CLI's record of which home this account's service runs (M38 §4.7), as
/// the window needs it: which of the CLI's three outcomes it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// No file: a first run, which the setup assistant is for.
    Absent,
    /// A file the CLI would refuse, and its sentence. It is never a reason to
    /// guess the default home.
    Unreadable(String),
    /// A file that names a home the engine can run.
    Present,
}

/// `$XDG_CONFIG_HOME/fermix/service.json`, or the same under `~/.config` when
/// the variable is unset or empty. The CLI takes any other value as it is, and
/// so does this.
pub fn binding_path(config_home: Option<&OsStr>, home: &Path) -> PathBuf {
    assert!(
        home.is_absolute(),
        "the home directory is absolute: {home:?}"
    );
    let config = match config_home {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.join(".config"),
    };
    config.join(BINDING_FILE)
}

/// Reads the binding as the CLI's `Binding.read` does, with its sentences: a
/// missing file is a first run, a file that cannot be read or does not name a
/// usable home is refused with a sentence naming it, and any other names a home.
pub fn read_binding(path: &Path) -> Binding {
    assert!(
        path.ends_with(BINDING_FILE),
        "not the service binding: {path:?}"
    );
    let file = path.display();
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Binding::Absent,
        Err(e) => {
            let why = e.kind();
            return Binding::Unreadable(format!(
                "The service binding at {file} could not be read ({why})."
            ));
        }
    };
    let Ok(document) = serde_json::from_slice::<Value>(&contents) else {
        return Binding::Unreadable(format!(
            "The service binding at {file} could not be read as JSON."
        ));
    };
    let refusal = match (document.get("schema_version"), document.get("home")) {
        (Some(version), Some(home)) if *version == BINDING_SCHEMA => home_refusal(home),
        (Some(version), _) if *version != BINDING_SCHEMA => Some(format!(
            "The service binding at {file} is written in schema version {version}, which this \
             Fermix does not read."
        )),
        _ => Some(format!(
            "The service binding at {file} does not name a home."
        )),
    };
    refusal.map_or(Binding::Present, Binding::Unreadable)
}

/// Why the engine would refuse `home` (`Binding.validate`), if it would.
fn home_refusal(home: &Value) -> Option<String> {
    let Some(home) = home.as_str() else {
        return Some("The service home must be a path, written as text.".into());
    };
    if !Path::new(home).is_absolute() {
        return Some(format!(
            "The service home must be an absolute path, and {home:?} is not."
        ));
    }
    if home.chars().any(|c| c.is_ascii_control()) {
        return Some(
            "The service home contains a control character, which a path cannot carry.".into(),
        );
    }
    let bytes = Path::new(home).join("daemon.sock").as_os_str().len();
    (bytes > SOCKET_PATH_MAX).then(|| {
        format!(
            "The daemon.sock path under that home is {bytes} bytes, over the {SOCKET_PATH_MAX}-byte \
             limit this OS allows for a unix socket address. Choose a shorter home."
        )
    })
}

/// What `fermix service install --json` answered: `Ok` once the engine says the
/// service is bound, enabled and answering, else the engine's own sentence.
/// Anything but its envelope is never taken for success.
pub fn install_answer(stdout: &str) -> Result<(), String> {
    let unreadable = || INSTALL_UNREADABLE.to_owned();
    let envelope: Value = serde_json::from_str(stdout.trim()).map_err(|_| unreadable())?;
    if envelope["schema_version"] != ENVELOPE_SCHEMA {
        return Err(unreadable());
    }
    if envelope["ok"] == true {
        return Ok(());
    }
    match envelope["error"]["sentence"].as_str() {
        Some(sentence) if envelope["ok"] == false => Err(sentence.to_owned()),
        _ => Err(unreadable()),
    }
}

/// The command ran past `INSTALL_CAP` and was stopped.
pub fn install_timed_out() -> String {
    let minutes = INSTALL_CAP.as_secs() / 60;
    format!(
        "Fermix was not set up within {minutes} minutes, so this app stopped {INSTALL_COMMAND}. \
         Read what it said with journalctl --user -u fermix, then try again."
    )
}

/// The command could not be started, or its answer could not be read; `why` is
/// what the system said.
pub fn install_not_run(why: &str) -> String {
    format!("This app could not run {INSTALL_COMMAND}: {why}. Run it in a terminal to see what it says.")
}

/// How a run of `INSTALL_ARGV` within `INSTALL_CAP` came out: `Ok` once the
/// engine's envelope says so, else the sentence for what went wrong.
pub fn install_outcome(ended: io::Result<Ended>) -> Result<(), String> {
    match ended {
        Ok(Ended::Exited(stdout)) => {
            let stdout = String::from_utf8(stdout).map_err(|_| INSTALL_UNREADABLE.to_owned())?;
            install_answer(&stdout)
        }
        Ok(Ended::Capped) => Err(install_timed_out()),
        Err(e) => Err(install_not_run(&e.to_string())),
    }
}

/// The terminal command a failed start names, so its output shows why: the
/// unit's start on a bound account, and the install on one with no binding
/// yet, since that is what starting it runs there.
pub fn start_command(bound: bool) -> &'static str {
    if bound {
        START_COMMAND
    } else {
        INSTALL_COMMAND
    }
}
