//! The update feed, format 1 (update-feed-spec). The served file is a strict
//! envelope: a key id, the payload's bytes in base64, and an ed25519 signature
//! over exactly those bytes. Only a payload whose signature verifies against a
//! compiled-in key is read, and it is read leniently, so fields a later
//! release adds stay readable, but held to every limit the spec sets.

use crate::update::version::Version;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use std::fmt;
use std::ops::RangeInclusive;

/// Where the feed is served. Package URLs inside it may be on any https
/// origin: trust comes from the signature and each file's sha256.
pub const FEED_URL: &str = "https://fermix.ai/linux/appcast.json";
/// The most of the served file the app reads. A longer one is refused.
pub const FEED_CAP: usize = 1024 * 1024;
/// The largest package the feed may name.
pub const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const PRODUCT: &str = "fermix-desktop";
const RELEASES: RangeInclusive<usize> = 1..=200;
const NOTES: RangeInclusive<usize> = 1..=8;
const NOTE_CHARS: RangeInclusive<usize> = 1..=300;
const KEY_ID_LENGTH: RangeInclusive<usize> = 1..=64;
/// An engine version or tag is a short word, never a paragraph.
const ENGINE_WORD: usize = 64;

/// A public key the app trusts, by the `key_id` an envelope names it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedKey {
    pub id: String,
    pub public: [u8; 32],
}

/// Why a served feed was refused. Each says what was wrong, for the log; the
/// sentence a person reads is `update::view`'.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    /// Longer than `FEED_CAP`.
    TooLarge,
    /// Not the envelope format 1 describes.
    Envelope(String),
    /// A well-formed key id the app has no key for.
    UnknownKey(String),
    /// The signature does not verify.
    Signature,
    /// Signed, but not a payload format 1 allows.
    Payload(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Amd64,
    Arm64,
}

impl Arch {
    /// From Rust's name for a target's architecture (`std::env::consts::ARCH`).
    pub fn from_target(arch: &str) -> Option<Arch> {
        match arch {
            "x86_64" => Some(Arch::Amd64),
            "aarch64" => Some(Arch::Arm64),
            _ => None,
        }
    }

    /// The machine this window was built for, which its package was.
    pub fn this_machine() -> Option<Arch> {
        Arch::from_target(std::env::consts::ARCH)
    }

    fn rpm(self) -> &'static str {
        match self {
            Arch::Amd64 => "x86_64",
            Arch::Arm64 => "aarch64",
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Arch::Amd64 => "amd64",
            Arch::Arm64 => "arm64",
        })
    }
}

/// How `fermix-desktop` was installed, which decides the package to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Deb,
    Rpm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    /// Seconds since the epoch. It only ever moves forward (rule 2).
    pub generated_at: u64,
    /// Newest first, with strictly decreasing versions.
    pub releases: Vec<Release>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub published_at: u64,
    pub critical: bool,
    pub minimum_installed_version: Option<Version>,
    pub engine: Engine,
    pub notes: Vec<String>,
    pub packages: Packages,
}

/// The engine the release carries, as `desktop/engine/PIN.json` names it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Engine {
    pub version: String,
    pub tag: String,
    pub source_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Packages {
    pub amd64: Pair,
    pub arm64: Pair,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pair {
    pub deb: Package,
    pub rpm: Package,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Package {
    pub file: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

impl Packages {
    pub fn pick(&self, arch: Arch, format: Format) -> &Package {
        let pair = match arch {
            Arch::Amd64 => &self.amd64,
            Arch::Arm64 => &self.arm64,
        };
        match format {
            Format::Deb => &pair.deb,
            Format::Rpm => &pair.rpm,
        }
    }
}

/// The name `build_packages.sh` gives a release's package.
pub fn package_file(version: Version, arch: Arch, format: Format) -> String {
    match format {
        Format::Deb => format!("fermix-desktop_{version}_{arch}.deb"),
        Format::Rpm => format!("fermix-desktop-{version}-1.{}.rpm", arch.rpm()),
    }
}

/// The served file, as format 1 has it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: u64,
    key_id: String,
    payload: String,
    signature: String,
}

#[derive(Deserialize)]
struct RawPayload {
    format: u64,
    product: String,
    generated_at: String,
    releases: Vec<RawRelease>,
}

#[derive(Deserialize)]
struct RawRelease {
    version: String,
    published_at: String,
    critical: bool,
    minimum_installed_version: Option<String>,
    engine: Engine,
    notes: Vec<String>,
    packages: Packages,
}

/// The served bytes, checked and decoded: the envelope, then the signature
/// against `keys`, then the payload.
pub fn read_feed(bytes: &[u8], keys: &[FeedKey]) -> Result<Feed, FeedError> {
    let payload = verified_payload(bytes, keys)?;
    decode_payload(&payload)
}

fn verified_payload(bytes: &[u8], keys: &[FeedKey]) -> Result<Vec<u8>, FeedError> {
    if bytes.len() > FEED_CAP {
        return Err(FeedError::TooLarge);
    }
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|e| FeedError::Envelope(e.to_string()))?;
    if envelope.format != 1 {
        let format = envelope.format;
        return Err(FeedError::Envelope(format!("format {format} is not 1")));
    }
    if !key_id_allowed(&envelope.key_id) {
        let id = &envelope.key_id;
        return Err(FeedError::Envelope(format!(
            "key_id {id:?} is not [a-z0-9-]{{1,64}}"
        )));
    }
    let key = keys
        .iter()
        .find(|k| k.id == envelope.key_id)
        .ok_or_else(|| FeedError::UnknownKey(envelope.key_id.clone()))?;
    let payload = base64(&envelope.payload, "payload")?;
    let signature: [u8; 64] = base64(&envelope.signature, "signature")?
        .try_into()
        .map_err(|_| FeedError::Envelope("the signature is not 64 bytes".into()))?;
    verify(&key.public, &payload, &signature)?;
    Ok(payload)
}

fn key_id_allowed(id: &str) -> bool {
    KEY_ID_LENGTH.contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Standard base64 with its padding, and nothing else.
fn base64(text: &str, what: &str) -> Result<Vec<u8>, FeedError> {
    STANDARD
        .decode(text)
        .map_err(|e| FeedError::Envelope(format!("the {what} is not standard base64: {e}")))
}

/// RFC 8032 with the strict checks, so neither a small-order key nor a
/// malleated signature passes.
fn verify(public: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> Result<(), FeedError> {
    let key = VerifyingKey::from_bytes(public).map_err(|_| FeedError::Signature)?;
    key.verify_strict(message, &Signature::from_bytes(signature))
        .map_err(|_| FeedError::Signature)
}

fn decode_payload(bytes: &[u8]) -> Result<Feed, FeedError> {
    let raw: RawPayload = serde_json::from_slice(bytes)
        .map_err(|e| FeedError::Payload(format!("not a payload: {e}")))?;
    check(raw.format == 1, || {
        format!("format {} is not 1", raw.format)
    })?;
    check(raw.product == PRODUCT, || {
        format!("product {:?} is not {PRODUCT}", raw.product)
    })?;
    let generated_at = timestamp("generated_at", &raw.generated_at)?;
    let count = raw.releases.len();
    check(RELEASES.contains(&count), || {
        format!("{count} releases, not 1 to 200")
    })?;
    let releases = raw
        .releases
        .into_iter()
        .map(release)
        .collect::<Result<Vec<Release>, FeedError>>()?;
    let ordered = releases.windows(2).all(|p| p[0].version > p[1].version);
    check(ordered, || {
        "the releases are not newest first, with strictly decreasing versions".into()
    })?;
    Ok(Feed {
        generated_at,
        releases,
    })
}

fn release(raw: RawRelease) -> Result<Release, FeedError> {
    let version = Version::parse(&raw.version)
        .ok_or_else(|| FeedError::Payload(format!("version {:?} is not X.Y.Z", raw.version)))?;
    let minimum = match raw.minimum_installed_version.as_deref() {
        None => None,
        Some(text) => Some(Version::parse(text).ok_or_else(|| {
            FeedError::Payload(format!(
                "{version}: minimum_installed_version {text:?} is not X.Y.Z"
            ))
        })?),
    };
    let published_at = timestamp("published_at", &raw.published_at)?;
    check_engine(version, &raw.engine)?;
    check_notes(version, &raw.notes)?;
    check_packages(version, &raw.packages)?;
    Ok(Release {
        version,
        published_at,
        critical: raw.critical,
        minimum_installed_version: minimum,
        engine: raw.engine,
        notes: raw.notes,
        packages: raw.packages,
    })
}

fn check_engine(version: Version, engine: &Engine) -> Result<(), FeedError> {
    let word = |text: &str| {
        !text.is_empty() && text.len() <= ENGINE_WORD && text.bytes().all(|b| b.is_ascii_graphic())
    };
    check(word(&engine.version) && word(&engine.tag), || {
        format!("{version}: the engine's version or tag is not a short word")
    })?;
    check(lower_hex(&engine.source_commit, 40), || {
        format!("{version}: the engine's source_commit is not 40 lowercase hex")
    })
}

fn check_notes(version: Version, notes: &[String]) -> Result<(), FeedError> {
    check(NOTES.contains(&notes.len()), || {
        format!("{version}: {} notes, not 1 to 8", notes.len())
    })?;
    match notes
        .iter()
        .map(|n| n.chars().count())
        .find(|n| !NOTE_CHARS.contains(n))
    {
        Some(chars) => Err(FeedError::Payload(format!(
            "{version}: a note of {chars} characters, not 1 to 300"
        ))),
        None => Ok(()),
    }
}

fn check_packages(version: Version, packages: &Packages) -> Result<(), FeedError> {
    for arch in [Arch::Amd64, Arch::Arm64] {
        for format in [Format::Deb, Format::Rpm] {
            let file = package_file(version, arch, format);
            check_package(packages.pick(arch, format), &file)?;
        }
    }
    Ok(())
}

fn check_package(package: &Package, file: &str) -> Result<(), FeedError> {
    check(package.file == file, || {
        format!("file {:?} is not {file}", package.file)
    })?;
    check(url_allowed(&package.url, file), || {
        format!(
            "{file}: url {:?} is not https ending in /{file}",
            package.url
        )
    })?;
    check((1..=MAX_PACKAGE).contains(&package.size), || {
        format!(
            "{file}: size {} is not 1 to {MAX_PACKAGE} bytes",
            package.size
        )
    })?;
    check(lower_hex(&package.sha256, 64), || {
        format!("{file}: sha256 is not 64 lowercase hex")
    })
}

/// https, a host, no query or fragment, nothing but visible ASCII, and the
/// file's own name as the last path segment.
fn url_allowed(url: &str, file: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    !host.is_empty()
        && url.bytes().all(|b| b.is_ascii_graphic())
        && !url.contains(['?', '#'])
        && rest.ends_with(&format!("/{file}"))
}

fn lower_hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn timestamp(field: &str, text: &str) -> Result<u64, FeedError> {
    parse_utc(text)
        .ok_or_else(|| FeedError::Payload(format!("{field} {text:?} is not YYYY-MM-DDTHH:MM:SSZ")))
}

fn check(holds: bool, why: impl FnOnce() -> String) -> Result<(), FeedError> {
    if holds {
        Ok(())
    } else {
        Err(FeedError::Payload(why()))
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`, from 1970 on, as seconds since the epoch. No other
/// form is read: the feed's writer writes only this one.
pub fn parse_utc(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    let separators = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'Z'),
    ];
    if b.len() != 20 || separators.iter().any(|&(at, c)| b[at] != c) {
        return None;
    }
    let field = |from: usize, to: usize| -> Option<u64> {
        let digits = &b[from..to];
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(digits.iter().fold(0, |n, d| n * 10 + u64::from(d - b'0')))
    };
    let (year, month, day) = (field(0, 4)?, field(5, 7)?, field(8, 10)?);
    let (hour, minute, second) = (field(11, 13)?, field(14, 16)?, field(17, 19)?);
    let valid = year >= 1970
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60;
    valid.then(|| days_since_epoch(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// The UTC date, `(year, month, day)`, of a time in seconds since the epoch
/// (H. Hinnant's civil_from_days).
pub fn utc_date(seconds: u64) -> (u64, u64, u64) {
    let shifted = seconds / 86_400 + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

fn days_in_month(year: u64, month: u64) -> u64 {
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a date from 1970 on, counted in eras of 400 years
/// that start on 1 March (H. Hinnant's days_from_civil).
fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
