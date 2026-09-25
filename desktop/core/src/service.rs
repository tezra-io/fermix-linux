//! The background service as systemd reports it (spec §6.2, §6.3) and the
//! desktop's own login registration (§6.6). Only words and rules live here;
//! the D-Bus calls are the app's.

/// Where the package installs the unit. Any other effective path shadows it.
pub const VENDOR_UNIT: &str = "/usr/lib/systemd/user/fermix.service";
/// The command that writes the binding and adopts or migrates the unit (M38 §4.5).
pub const INSTALL_COMMAND: &str = "fermix service install";
/// What linger is fixed with when the app may not set it.
pub const LINGER_COMMAND: &str = "loginctl enable-linger";
pub const BACKGROUND_NOTE: &str =
    "Fermix starts with your computer and keeps answering when this window is closed.";

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

/// How a Background portal request ended.
#[derive(Debug, Clone, PartialEq)]
pub enum LoginAnswer {
    /// Granted: whether the app now opens at login.
    Set(bool),
    /// The person dismissed the desktop's question.
    Cancelled,
    Refused(&'static str),
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
/// exist (its file is there, or the daemon answers): without one, enabling would
/// start a unit that can only fail (spec §6.5). `busy` while a change runs.
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
    if !on && !bound {
        return refused(format!(
            "Fermix is not set up on this computer yet. Run {INSTALL_COMMAND} in a terminal first."
        ));
    }
    BackgroundSwitch {
        on,
        sensitive: !busy,
        note: BACKGROUND_NOTE.into(),
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

/// Reads the portal's `Response(code, {autostart})` for a request that asked
/// for `wanted`. Code 0 is granted, 1 cancelled, anything else an error.
pub fn login_answer(code: u32, autostart: Option<bool>, wanted: bool) -> LoginAnswer {
    match code {
        0 => {
            let now = autostart.unwrap_or(false);
            if wanted && !now {
                LoginAnswer::Refused("Your desktop did not allow Fermix to open at login.")
            } else {
                LoginAnswer::Set(now)
            }
        }
        1 => LoginAnswer::Cancelled,
        _ => LoginAnswer::Refused("Your desktop could not change this, so nothing changed."),
    }
}
