//! What a verified feed means for this installation (update-feed-spec, rules
//! 1 to 6): whether anything is offered, which release, whether it is
//! critical or skipped, and which package fits. Pure; the app fetches and
//! installs.

use crate::update::feed::{Feed, Format, Release};
use crate::update::version::{Installed, Version};

/// The package that installed the window: `/usr/bin/fermix-desktop`, as the
/// package databases are asked about it.
pub const PACKAGED_PATH: &str = "/usr/bin/fermix-desktop";
const PACKAGE: &str = "fermix-desktop";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Rule 1: a development build never offers an update.
    Development,
    UpToDate,
    Available(Box<Offer>),
    /// Every newer release needs a newer installed version than this one,
    /// so none can be installed over it.
    Stranded {
        latest: Version,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub release: Release,
    /// Rule 4: some release past the installed version, up to the offer, is.
    pub critical: bool,
    /// Rule 3: the newest release, when it is newer than the offer and needs
    /// the offer installed first.
    pub held_back: Option<Version>,
    /// Rule 5: the person skipped this version. A critical offer never is.
    pub skipped: bool,
}

/// Rule 2: a feed generated before the newest one accepted is a replay or a
/// freeze. An equal one is the same feed again.
pub fn fresh(generated_at: u64, accepted: Option<u64>) -> bool {
    accepted.is_none_or(|accepted| generated_at >= accepted)
}

/// Rules 1, 3, 4 and 5, for the installed package's version.
pub fn decide(feed: &Feed, installed: Installed, skipped: Option<Version>) -> Outcome {
    let Installed::Release(current) = installed else {
        return Outcome::Development;
    };
    let newer: Vec<&Release> = feed
        .releases
        .iter()
        .filter(|r| r.version > current)
        .collect();
    let Some(latest) = newer.first() else {
        return Outcome::UpToDate;
    };
    let installable = |r: &&&Release| r.minimum_installed_version.is_none_or(|m| m <= current);
    let Some(release) = newer.iter().find(installable) else {
        return Outcome::Stranded {
            latest: latest.version,
        };
    };
    let critical = newer
        .iter()
        .any(|r| r.critical && r.version <= release.version);
    Outcome::Available(Box::new(Offer {
        release: (*release).clone(),
        critical,
        held_back: (latest.version > release.version).then_some(latest.version),
        skipped: !critical && skipped == Some(release.version),
    }))
}

/// Rule 6's second half: which package database owns `PACKAGED_PATH` for
/// `fermix-desktop`, from what `dpkg-query -S` and `rpm -qf --qf '%{NAME}\n'`
/// printed (`None` where the tool did not run). Both claiming it, or neither,
/// leaves the in-app install off.
pub fn origin(dpkg: Option<&str>, rpm: Option<&str>) -> Option<Format> {
    let deb_line = format!("{PACKAGE}: {PACKAGED_PATH}");
    let deb = dpkg.is_some_and(|out| out.lines().any(|line| line == deb_line));
    let rpm = rpm.is_some_and(|out| out.lines().any(|line| line == PACKAGE));
    match (deb, rpm) {
        (true, false) => Some(Format::Deb),
        (false, true) => Some(Format::Rpm),
        _ => None,
    }
}
