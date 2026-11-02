//! The desktop notifications of Always On (Always On setup §3.1): the engine names each
//! occurrence it raises (`summary.notice`: the server ready, the move done, the server out of
//! reach, a server to set up again, the newest backup a week old, Fermix back on this computer
//! from a server of the owner's own) by an id that stays while it stands. The app raises a notice
//! whose id it has not raised yet, and keeps the last sixteen ids it raised: an id can come back
//! after an outage clears, and the set stays bounded. The ids are kept in a small file under
//! `$XDG_STATE_HOME`, so a notice that still stands is not raised again at every launch.

use crate::home::Notice;
use crate::update::store::{replace, state_dir};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// How many raised ids are kept.
pub const POSTED: usize = 16;
const FILE: &str = "fermix-desktop/always-on-notices.json";
/// The most bytes the file is read for: sixteen ids are far less.
const FILE_BOUND: u64 = 16 * 1024;

/// The ids of the notices raised, oldest first.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Posted {
    #[serde(rename = "posted")]
    ids: VecDeque<String>,
}

/// `$XDG_STATE_HOME/fermix-desktop/always-on-notices.json`, or under `~/.local/state`.
pub fn notices_file(state_home: Option<&OsStr>, home: &Path) -> PathBuf {
    state_dir(state_home, home).join(FILE)
}

impl Posted {
    /// Whether `notice` is one to raise now; it is kept as raised.
    pub fn fresh(&mut self, notice: &Notice) -> bool {
        assert!(!notice.id.is_empty(), "a notice is named by its id");
        if self.ids.contains(&notice.id) {
            return false;
        }
        if self.ids.len() == POSTED {
            self.ids.pop_front();
        }
        self.ids.push_back(notice.id.clone());
        true
    }

    /// The ids kept, oldest first.
    pub fn ids(&self) -> Vec<String> {
        self.ids.iter().cloned().collect()
    }

    /// What earlier runs raised, the newest sixteen; none when nothing was saved. A file that
    /// cannot be read, is not this list or is larger than sixteen ids can be is an error for the
    /// caller to log.
    pub fn load(path: &Path) -> Result<Posted, String> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Posted::default()),
            Err(e) => return Err(format!("{} could not be read: {e}", path.display())),
        };
        let mut text = String::new();
        file.take(FILE_BOUND + 1)
            .read_to_string(&mut text)
            .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
        if text.len() as u64 > FILE_BOUND {
            return Err(format!(
                "{} is larger than {FILE_BOUND} bytes",
                path.display()
            ));
        }
        let mut posted: Posted = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not Always On's notices: {e}", path.display()))?;
        let extra = posted.ids.len().saturating_sub(POSTED);
        posted.ids.drain(..extra);
        Ok(posted)
    }

    /// Saves the ids in one step.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_vec_pretty(self).expect("the ids serialize");
        replace(path, &text)
    }
}
