//! Versions as the update feed writes them, X.Y.Z, and the installed package's,
//! which a development build marks with `+dev` (update-feed-spec, rule 1).

use serde::{Deserialize, Serialize};
use std::fmt;

/// A release version: three plain numbers with no leading zeros and no
/// pre-release or build part, ordered by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let version = Version {
            major: number(parts.next()?)?,
            minor: number(parts.next()?)?,
            patch: number(parts.next()?)?,
        };
        parts.next().is_none().then_some(version)
    }
}

/// One part of X.Y.Z: ASCII digits, and no leading zero unless it is 0.
fn number(part: &str) -> Option<u64> {
    let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let leading_zero = part.len() > 1 && part.starts_with('0');
    if !digits || leading_zero {
        return None;
    }
    part.parse().ok()
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl TryFrom<String> for Version {
    type Error = String;
    fn try_from(text: String) -> Result<Version, String> {
        Version::parse(&text).ok_or_else(|| format!("{text:?} is not X.Y.Z"))
    }
}

impl From<Version> for String {
    fn from(version: Version) -> String {
        version.to_string()
    }
}

/// The package database's version of `fermix-desktop`: a release's X.Y.Z, or a
/// development package's `<X.Y.Z>+dev.<time>.<commit>`, which never updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    Release(Version),
    Development(Version),
}

impl Installed {
    /// `None` for a version this project's packages never carry.
    pub fn parse(package_version: &str) -> Option<Installed> {
        match package_version.split_once('+') {
            None => Version::parse(package_version).map(Installed::Release),
            Some((base, build)) if build.starts_with("dev") => {
                Version::parse(base).map(Installed::Development)
            }
            Some(_) => None,
        }
    }

    /// The X.Y.Z itself, for a development build its base.
    pub fn version(self) -> Version {
        match self {
            Installed::Release(version) | Installed::Development(version) => version,
        }
    }
}
