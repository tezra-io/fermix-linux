//! One update check: fetch the feed, verify and decode it, refuse an older
//! one than already accepted (rule 2), and decide what it offers. Blocking;
//! the app runs it off its main loop and remembers how it ended.

use crate::update::feed::{read_feed, FeedError, FeedKey};
use crate::update::fetch::{FetchError, Fetcher};
use crate::update::policy::{decide, fresh, Outcome};
use crate::update::state::Installation;
use crate::update::store::{Checked, LastCheck, Remembered};
use crate::update::version::Installed;
use crate::update::view::check_failure;

/// Automatic checks run at launch, then once this long after the last.
pub const CHECK_EVERY: u64 = 24 * 60 * 60;

/// Whether an automatic check is due at `now`: the switch is on, and a day
/// has passed since the last check, or none is known. A last check dated
/// after `now` was taken by a wrong clock, so it does not hold the next off.
pub fn due(remembered: &Remembered, now: u64) -> bool {
    let Some(last) = &remembered.last_check else {
        return remembered.automatic;
    };
    let waited = now
        .checked_sub(last.at)
        .is_none_or(|ago| ago >= CHECK_EVERY);
    remembered.automatic && waited
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub outcome: Outcome,
    /// The feed's `generated_at`; `None` when nothing was fetched.
    pub generated_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    Fetch(FetchError),
    Feed(FeedError),
    /// Rule 2: the feed is older than one already accepted.
    Stale {
        accepted: u64,
        generated_at: u64,
    },
    /// The installed version is not one this project's packages carry.
    Unknown(String),
}

/// Checks `url` for this installation. A development build never fetches.
pub fn check(
    fetcher: &Fetcher,
    url: &str,
    keys: &[FeedKey],
    installation: &Installation,
    remembered: &Remembered,
) -> Result<Found, CheckError> {
    let installed = match installation {
        Installation::Package { version, .. } => Installed::Release(*version),
        Installation::Development { .. } => {
            return Ok(Found {
                outcome: Outcome::Development,
                generated_at: None,
            })
        }
        Installation::Unknown { version } => return Err(CheckError::Unknown(version.clone())),
    };
    let bytes = fetcher.feed(url).map_err(CheckError::Fetch)?;
    let feed = read_feed(&bytes, keys).map_err(CheckError::Feed)?;
    if let Some(accepted) = remembered.accepted_generated_at {
        if !fresh(feed.generated_at, Some(accepted)) {
            return Err(CheckError::Stale {
                accepted,
                generated_at: feed.generated_at,
            });
        }
    }
    Ok(Found {
        outcome: decide(&feed, installed, remembered.skipped),
        generated_at: Some(feed.generated_at),
    })
}

/// What a check leaves remembered: when it ran and how it ended, and the
/// feed it accepted. A failure moves nothing forward, and nothing forgets a
/// skip.
pub fn remember(remembered: &mut Remembered, at: u64, answer: &Result<Found, CheckError>) {
    let result = match answer {
        Ok(found) => {
            if let Some(generated_at) = found.generated_at {
                let newest = remembered.accepted_generated_at.max(Some(generated_at));
                remembered.accepted_generated_at = newest;
            }
            checked(&found.outcome)
        }
        Err(e) => Checked::Failed {
            sentence: check_failure(e),
        },
    };
    remembered.last_check = Some(LastCheck { at, result });
}

fn checked(outcome: &Outcome) -> Checked {
    match outcome {
        Outcome::Development => Checked::Development,
        Outcome::UpToDate => Checked::UpToDate,
        Outcome::Available(offer) => Checked::Available {
            version: offer.release.version,
            critical: offer.critical,
        },
        Outcome::Stranded { latest } => Checked::Stranded { latest: *latest },
    }
}
