//! The count of what reads each phone-side file, and the `media_fetch` and `media_release` the
//! count asks for (macOS `MediaGrants`, M66 decision 1). A ref is held for the length of a read:
//! a thumbnail's decode, the viewer's, a copy, an app opening it. It is fetched when its first
//! read comes and released once, when its last one goes, and only where the daemon granted it.
//! The first answer goes to every read waiting on it, and a later read takes it at once.
//!
//! A connection's grants end with it: after every handshake each ref still read is fetched
//! again, and a read while there is no connection waits for the next one. Pure: the caller sends
//! what each call returns, and keeps the readers' callbacks by their tickets.

use super::protocol::{is_media_ref, ClientEvent, Grant, ServerEvent};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// What asking for a ref's file came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The daemon's managed file of the ref's bytes, checked to be one (`verify`).
    Granted(PathBuf),
    /// Nothing to grant: the bytes are gone, no row names the ref, or it is not one.
    Missing,
    /// The daemon could not make the file, or granted one this app refuses: why.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Now(Outcome),
    /// When the daemon answers, to the read's ticket (`Answered::tickets`).
    Later,
}

/// One read's place in the line for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ticket(u64);

/// An answer to a fetch, for the reads that waited on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub r#ref: String,
    pub outcome: Outcome,
    /// The reads waiting, in the order they came.
    pub tickets: Vec<Ticket>,
    /// To send: the release of a grant nothing reads any more.
    pub events: Vec<ClientEvent>,
    /// Whether this connection asked for it. An answer nobody asked for is worth a log line.
    pub asked: bool,
}

#[derive(Debug, Default)]
struct Uses {
    count: u32,
    answer: Option<Outcome>,
    waiting: Vec<Ticket>,
}

#[derive(Debug)]
pub struct Grants {
    /// `$FERMIX_HOME/companion/media`, where every grant must be.
    media_dir: PathBuf,
    /// A negotiated connection is up to ask on.
    online: bool,
    uses: BTreeMap<String, Uses>,
    /// Fetched on this connection and not answered yet.
    asked: BTreeSet<String>,
    tickets: u64,
}

impl Grants {
    pub fn new(media_dir: PathBuf) -> Grants {
        debug_assert!(
            media_dir.is_absolute(),
            "the media directory must be absolute: {}",
            media_dir.display()
        );
        Grants {
            media_dir,
            online: false,
            uses: BTreeMap::new(),
            asked: BTreeSet::new(),
            tickets: 0,
        }
    }

    /// One more read of `r#ref`, paired with exactly one `release`, whatever it answered. A ref
    /// that is not one is missing, counted for nothing.
    pub fn hold(&mut self, r#ref: &str) -> (Ticket, Answer, Vec<ClientEvent>) {
        self.tickets += 1;
        let ticket = Ticket(self.tickets);
        if !is_media_ref(r#ref) {
            return (ticket, Answer::Now(Outcome::Missing), Vec::new());
        }
        let uses = self.uses.entry(r#ref.to_owned()).or_default();
        uses.count += 1;
        if let Some(known) = &uses.answer {
            return (ticket, Answer::Now(known.clone()), Vec::new());
        }
        uses.waiting.push(ticket);
        let first = uses.count == 1;
        let events = if first { self.fetch(r#ref) } else { Vec::new() };
        (ticket, Answer::Later, events)
    }

    /// A read that stopped waiting before its answer; its `release` follows.
    pub fn forget(&mut self, ticket: Ticket, r#ref: &str) {
        if let Some(uses) = self.uses.get_mut(r#ref) {
            uses.waiting.retain(|waiting| *waiting != ticket);
        }
    }

    /// One read ended. The last lets the file go, where the daemon granted it; a release before
    /// the answer sends nothing, and the grant is let go when it comes (`answered`).
    pub fn release(&mut self, r#ref: &str) -> Vec<ClientEvent> {
        let Some(uses) = self.uses.get_mut(r#ref) else {
            return Vec::new();
        };
        uses.count = uses.count.saturating_sub(1);
        if uses.count > 0 {
            return Vec::new();
        }
        let granted = matches!(uses.answer, Some(Outcome::Granted(_)));
        self.uses.remove(r#ref);
        if granted {
            vec![ClientEvent::media_release(r#ref)]
        } else {
            Vec::new()
        }
    }

    /// The daemon's answer to a fetch, where `event` is one: a grant, checked; a ref it has
    /// nothing for; or a copy it failed to make. Any other event, an error without a ref among
    /// them, is not about a file.
    pub fn answered(&mut self, event: &ServerEvent) -> Option<Answered> {
        let (r#ref, outcome) = match event {
            ServerEvent::MediaPath(grant) => (grant.r#ref.clone(), self.granted(grant)),
            ServerEvent::MediaMissing { r#ref } => (r#ref.clone(), Outcome::Missing),
            ServerEvent::Error(error) if error.reason == "request_failed" => {
                let why = error
                    .message
                    .clone()
                    .unwrap_or_else(|| error.reason.clone());
                (error.r#ref.clone()?, Outcome::Failed(why))
            }
            _ => return None,
        };
        let asked = self.asked.remove(&r#ref);
        // The daemon holds what it granted for this connection, whether or not this app takes
        // it (PROTOCOL rules 2 and 3). A grant refused here is never read, so it goes at once;
        // stored as Failed, its last read releases nothing more.
        let held = matches!(event, ServerEvent::MediaPath(_));
        let refused = held && !matches!(outcome, Outcome::Granted(_));
        let Some(uses) = self.uses.get_mut(&r#ref) else {
            // Nothing reads it any more, or nothing ever asked.
            return Some(Answered {
                events: released(held, &r#ref),
                r#ref,
                outcome,
                tickets: Vec::new(),
                asked,
            });
        };
        uses.answer = Some(outcome.clone());
        let tickets = std::mem::take(&mut uses.waiting);
        Some(Answered {
            events: released(refused, &r#ref),
            r#ref,
            outcome,
            tickets,
            asked,
        })
    }

    /// A handshake completed: every ref still read is asked for again, once.
    pub fn negotiated(&mut self) -> Vec<ClientEvent> {
        self.online = true;
        let refs: Vec<String> = self.uses.keys().cloned().collect();
        refs.iter().flat_map(|r#ref| self.fetch(r#ref)).collect()
    }

    /// The connection ended, and with it every grant on it: the paths may be gone. The reads
    /// stay counted, and those waiting wait on.
    pub fn disconnected(&mut self) {
        self.online = false;
        self.asked.clear();
        for uses in self.uses.values_mut() {
            uses.answer = None;
        }
    }

    /// How many reads of `r#ref` are counted.
    pub fn count(&self, r#ref: &str) -> u32 {
        self.uses.get(r#ref).map_or(0, |uses| uses.count)
    }

    /// Asks for `r#ref` once per connection until it is answered, marked before it is sent.
    fn fetch(&mut self, r#ref: &str) -> Vec<ClientEvent> {
        if !self.online || !self.asked.insert(r#ref.to_owned()) {
            return Vec::new();
        }
        vec![ClientEvent::media_fetch(r#ref)]
    }

    fn granted(&self, grant: &Grant) -> Outcome {
        match verify(grant, &self.media_dir) {
            Ok(path) => Outcome::Granted(path),
            Err(why) => Outcome::Failed(format!("broken grant: {why}")),
        }
    }
}

/// The release of `r#ref`, where it is to go.
fn released(goes: bool, r#ref: &str) -> Vec<ClientEvent> {
    if goes {
        vec![ClientEvent::media_release(r#ref)]
    } else {
        Vec::new()
    }
}

/// The grant's file, where it is one: a ref whose digest the grant says it is, at an absolute
/// path straight inside `media_dir`, named by the ref. The daemon is another process: a grant
/// that is anything else is refused, never followed.
pub fn verify(grant: &Grant, media_dir: &Path) -> Result<PathBuf, String> {
    if grant.sha256 != grant.r#ref || !is_media_ref(&grant.r#ref) {
        return Err(format!("{} is not the ref's digest", grant.sha256));
    }
    let path = Path::new(&grant.path);
    let plain = grant
        .path
        .split('/')
        .skip(1)
        .all(|part| !matches!(part, "" | "." | ".."));
    let normal = path
        .components()
        .skip(1)
        .all(|part| matches!(part, Component::Normal(_)));
    if !path.is_absolute() || !plain || !normal {
        return Err(format!("{} is not a plain absolute path", grant.path));
    }
    if path.parent() != Some(media_dir) {
        return Err(format!("{} is not in {}", grant.path, media_dir.display()));
    }
    if path.file_stem().and_then(|stem| stem.to_str()) != Some(grant.r#ref.as_str()) {
        return Err(format!("{} is not named by its ref", grant.path));
    }
    Ok(path.to_path_buf())
}
