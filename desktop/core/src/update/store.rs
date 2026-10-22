//! What the updater remembers between runs: the "Check for updates
//! automatically" switch, the newest feed it accepted (rule 2), its last
//! check and how that ended, and the version the person skipped (rule 5). The
//! app keeps no other per-user state, so this is its own file under
//! `$XDG_STATE_HOME`, replaced in one step on every save. Downloads go to the
//! cache, which every launch empties.

use crate::update::version::Version;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

const STATE_FILE: &str = "fermix-desktop/updates.json";
const DOWNLOADS: &str = "fermix-desktop/updates";
/// The most entries one launch's clean-up looks at in the downloads folder,
/// which holds one package and its partial at most.
const CLEAR_CAP: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    #[serde(default = "on")]
    pub automatic: bool,
    /// The newest `generated_at` accepted, in seconds since the epoch.
    #[serde(default)]
    pub accepted_generated_at: Option<u64>,
    #[serde(default)]
    pub last_check: Option<LastCheck>,
    #[serde(default)]
    pub skipped: Option<Version>,
}

impl Default for Remembered {
    fn default() -> Remembered {
        Remembered {
            automatic: true,
            accepted_generated_at: None,
            last_check: None,
            skipped: None,
        }
    }
}

fn on() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastCheck {
    /// Seconds since the epoch.
    pub at: u64,
    pub result: Checked,
}

/// How a check ended, as Settings and Home recall it after a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Checked {
    UpToDate,
    Development,
    Available {
        version: Version,
        critical: bool,
    },
    Stranded {
        latest: Version,
    },
    /// The sentence the failure was shown with.
    Failed {
        sentence: String,
    },
}

/// `$XDG_STATE_HOME/fermix-desktop/updates.json`, or under `~/.local/state`
/// when that is unset, empty or relative, which the XDG base directory
/// specification says to ignore.
pub fn state_path(state_home: Option<&OsStr>, home: &Path) -> PathBuf {
    xdg_dir(state_home, home, ".local/state").join(STATE_FILE)
}

/// `$XDG_CACHE_HOME/fermix-desktop/updates`, or under `~/.cache`.
pub fn download_dir(cache_home: Option<&OsStr>, home: &Path) -> PathBuf {
    xdg_dir(cache_home, home, ".cache").join(DOWNLOADS)
}

fn xdg_dir(value: Option<&OsStr>, home: &Path, fallback: &str) -> PathBuf {
    assert!(
        home.is_absolute(),
        "the home directory is absolute: {home:?}"
    );
    match value.map(Path::new) {
        Some(dir) if dir.is_absolute() => dir.to_path_buf(),
        _ => home.join(fallback),
    }
}

/// What was saved, or the defaults when nothing has been. A file that cannot
/// be read or parsed is an error for the caller to log.
pub fn load(path: &Path) -> Result<Remembered, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Remembered::default()),
        Err(e) => return Err(format!("{} could not be read: {e}", path.display())),
    };
    serde_json::from_str(&text)
        .map_err(|e| format!("{} is not the updater's state: {e}", path.display()))
}

/// Writes a temporary file beside `path`, flushes it to disk, and renames it
/// over `path`, so a reader sees the old state or the new, never a mix.
pub fn save(path: &Path, remembered: &Remembered) -> io::Result<()> {
    let dir = path
        .parent()
        .expect("the state file has a parent directory");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let text = serde_json::to_vec_pretty(remembered).expect("the state serializes");
    let written = write_synced(&temporary, &text).and_then(|()| fs::rename(&temporary, path));
    if let Err(e) = written {
        return Err(match fs::remove_file(&temporary) {
            Ok(()) => e,
            Err(left) if left.kind() == io::ErrorKind::NotFound => e,
            Err(left) => io::Error::new(
                e.kind(),
                format!("{e}, and {} is left: {left}", temporary.display()),
            ),
        });
    }
    File::open(dir)?.sync_all()
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Deletes every file in the downloads folder, which only this app writes:
/// a package installed, refused or left by a run that ended mid-download.
/// Returns how many went.
pub fn clear_downloads(dir: &Path) -> io::Result<usize> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for entry in entries.take(CLEAR_CAP) {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}
