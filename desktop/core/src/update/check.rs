//! One update check: fetch the feed, verify and decode it, refuse one more
//! than 180 days behind the local clock or more than a day ahead of it, refuse
//! an older one than already accepted (rule 2), and decide what it offers.
//! Blocking; the app runs it off its main loop and remembers how it ended.
//!
//! The age bound stops a freeze: whoever controls fermix.ai, or holds a
//! mis-issued certificate for it, can serve the newest signed feed forever,
//! and without the bound the app would keep saying it is up to date. A freeze
//! inside 180 days still goes unseen, except for the feed's date beside "Last
//! checked" in Settings; closing it needs a feed re-signed on a schedule with
//! an expiry, which needs the hosting automation.

use crate::update::feed::{read_feed, FeedError, FeedKey};
use crate::update::fetch::{FetchError, Fetcher};
use crate::update::policy::{decide, fresh, Outcome};
use crate::update::state::Installation;
use crate::update::store::{Checked, LastCheck, Remembered};
use crate::update::version::Installed;
use crate::update::view::check_failure;

/// Automatic checks run at launch, then once this long after the last.
pub const CHECK_EVERY: u64 = 24 * 60 * 60;
/// A feed written more than this long before the local clock is out of date.
/// Releases can be weeks apart, so a feed is never expected to be new.
pub const STALE_AFTER: u64 = 180 * 24 * 60 * 60;
/// A feed dated more than this far ahead of the local clock is out of date
/// too; up to it is the two clocks' skew.
pub const AHEAD_BY: u64 = 24 * 60 * 60;

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
    /// The feed was written more than `STALE_AFTER` before `now`, or more
    /// than `AHEAD_BY` after it.
    OutOfDate {
        generated_at: u64,
        now: u64,
    },
    /// The installed version is not one this project's packages carry.
    Unknown(String),
}

/// Checks `url` for this installation at `now`, the local clock in seconds
/// since the epoch. A development build never fetches.
pub fn check(
    fetcher: &Fetcher,
    url: &str,
    keys: &[FeedKey],
    installation: &Installation,
    remembered: &Remembered,
    now: u64,
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
    if !in_date(feed.generated_at, now) {
        return Err(CheckError::OutOfDate {
            generated_at: feed.generated_at,
            now,
        });
    }
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

/// Whether a feed written at `generated_at` is in date at `now`.
fn in_date(generated_at: u64, now: u64) -> bool {
    match now.checked_sub(generated_at) {
        Some(behind) => behind <= STALE_AFTER,
        None => generated_at - now <= AHEAD_BY,
    }
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
