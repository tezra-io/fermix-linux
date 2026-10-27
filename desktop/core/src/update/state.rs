//! What the window knows about updates, beside the daemon's state: how this
//! copy of Fermix was installed, what this run's checks found, whether a
//! restart is pending, and where an install the dialog runs stands. Only the
//! controller changes it; every update surface draws from it.

use crate::update::feed::{Arch, Format};
use crate::update::identity::Alignment;
use crate::update::packagekit;
use crate::update::policy::{Offer, Outcome, PACKAGED_PATH};
use crate::update::store::Remembered;
use crate::update::version::{Installed, Version};
use std::path::{Path, PathBuf};

/// How the window comes back after an update: the installed binary, never
/// `/proc/self/exe`, which names the replaced file, taking the app's D-Bus
/// name over from this process.
pub const RELAUNCH: [&str; 2] = [PACKAGED_PATH, "--gapplication-replace"];

/// How this copy of Fermix was installed, as the package databases say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installation {
    /// The `fermix-desktop` package at a release version. `format` is the
    /// database that owns it; with none, updates are shown but never installed.
    Package {
        version: Version,
        format: Option<Format>,
    },
    /// A `+dev` package, or a window not run from the package at all.
    Development { version: String },
    /// The package database names a version no release of this project has.
    Unknown { version: String },
}

/// `packaged`: the window runs from the package's directory. `format`: the
/// database that owns `/usr/bin/fermix-desktop`, and `package_version` what
/// it says is installed (`None` when it did not answer). `own_version` is the
/// window's compiled version.
pub fn installation(
    packaged: bool,
    format: Option<Format>,
    package_version: Option<&str>,
    own_version: &str,
) -> Installation {
    if !packaged {
        return Installation::Development {
            version: own_version.to_owned(),
        };
    }
    let version = match format {
        Some(_) => package_version.unwrap_or(own_version),
        None => own_version,
    };
    match Installed::parse(version) {
        Some(Installed::Release(version)) => Installation::Package { version, format },
        Some(Installed::Development(_)) => Installation::Development {
            version: version.to_owned(),
        },
        None => Installation::Unknown {
            version: version.to_owned(),
        },
    }
}

impl Installation {
    /// The installed version as a person reads it.
    pub fn version_text(&self) -> String {
        match self {
            Installation::Package { version, .. } => version.to_string(),
            Installation::Development { version } | Installation::Unknown { version } => {
                version.clone()
            }
        }
    }
}

/// The window's whole knowledge of updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updates {
    /// `None` until the package databases have answered, at launch.
    pub installation: Option<Installation>,
    pub arch: Option<Arch>,
    pub remembered: Remembered,
    pub checking: bool,
    /// What this run's last check found, with the release it offers.
    pub found: Option<Outcome>,
    /// Why this run's last check failed, until the next one ends.
    pub failure: Option<String>,
    /// What is installed is newer than what runs (M38 §9.2).
    pub restart_pending: bool,
    /// Where an install the dialog runs stands.
    pub step: Option<Step>,
}

impl Updates {
    pub fn new(remembered: Remembered) -> Updates {
        Updates {
            installation: None,
            arch: Arch::this_machine(),
            remembered,
            checking: false,
            found: None,
            failure: None,
            restart_pending: false,
            step: None,
        }
    }

    /// The release this run's last check offers, skipped or not.
    pub fn offer(&self) -> Option<&Offer> {
        match &self.found {
            Some(Outcome::Available(offer)) => Some(offer),
            _ => None,
        }
    }

    /// The offer the tray and Home announce: a skipped version is quiet
    /// until the person asks.
    pub fn announced(&self) -> Option<&Offer> {
        self.offer().filter(|offer| !offer.skipped)
    }

    /// The package database the in-app install goes through, if any.
    pub fn format(&self) -> Option<Format> {
        match &self.installation {
            Some(Installation::Package { format, .. }) => *format,
            _ => None,
        }
    }

    /// An install or restart the dialog runs, which nothing else may start.
    pub fn busy(&self) -> bool {
        matches!(&self.step, Some(step) if !matches!(step, Step::Failed(_)))
    }
}

/// One step of the install, which runs inside the dialog from Install
/// Update to the window coming back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Downloading {
        got: u64,
        size: u64,
    },
    /// Every byte is in; it is held to the feed's size and sha256.
    Verifying,
    /// PackageKit is asked; the system's password prompt comes next.
    Asking,
    Installing {
        percent: Option<u32>,
        waiting: Option<Waiting>,
    },
    Restarting,
    Failed(Failure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// Another program is installing software.
    Lock,
    /// The system's password prompt is open.
    Password,
}

/// What a transaction's PackageKit status says it waits for, if anything.
pub fn waiting(status: u32) -> Option<Waiting> {
    match status {
        packagekit::status::WAITING_FOR_LOCK => Some(Waiting::Lock),
        packagekit::status::WAITING_FOR_AUTH => Some(Waiting::Password),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub stage: Stage,
    /// What went wrong, as a sentence.
    pub sentence: String,
    pub manual: Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Download,
    Install,
    Restart,
}

/// What the person can do instead when the app could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manual {
    /// The downloaded package's path, and the command that installs what is
    /// there when it is run.
    Command { command: String, file: PathBuf },
    /// The package's own address, when it never downloaded whole.
    Link { url: String },
    /// Quit Fermix and open it again.
    Reopen,
}

/// The package manager's own command for the downloaded file. Its full path,
/// so it works from any folder, quoted for the shell only when it must be.
pub fn install_command(format: Format, file: &Path) -> String {
    let tool = match format {
        Format::Deb => "apt",
        Format::Rpm => "dnf",
    };
    format!(
        "sudo {tool} install {}",
        shell_word(&file.to_string_lossy())
    )
}

fn shell_word(text: &str) -> String {
    let plain = text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._+-".contains(&b));
    if plain {
        return text.to_owned();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// What starts the restart into an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    /// The install the dialog ran was just confirmed.
    AfterInstall,
    /// Restart, or Try Again after a failed restart.
    Asked,
}

/// Whether the restart into an update may start now. Not `busy`, which a
/// confirmed install still is: the restart is that install's own next step.
/// Asked for, it never cuts into a download, an install or a restart, and
/// needs something installed to restart into.
pub fn may_restart(updates: &Updates, restart: Restart) -> bool {
    if !matches!(updates.installation, Some(Installation::Package { .. })) {
        return false;
    }
    match (&updates.step, restart) {
        (Some(Step::Asking | Step::Installing { .. }), Restart::AfterInstall) => true,
        (Some(Step::Failed(failure)), Restart::Asked) => {
            failure.stage == Stage::Restart || updates.restart_pending
        }
        (None, Restart::Asked) => updates.restart_pending,
        _ => false,
    }
}

/// After an install, the engine restarts only when another build of it is
/// installed (a desktop-only release ships the same engine), or when an older
/// daemon names no build. An engine that is not this package's is never
/// touched, and one that is not running starts as the new one.
pub fn engine_needs_restart(alignment: &Alignment) -> bool {
    matches!(alignment, Alignment::PendingRestart | Alignment::Unknown)
}
