//! What has played of each playback id a Grok call's audio arrived under (realtime v3). The
//! daemon paces a Grok reply by what the app reports played, cuts a reply the operator stopped
//! at what was heard, and ends a call whose sent audio goes unreported, so every sample queued
//! under an id is accounted for: played, or lost to a stop.
//!
//! The playback queue is the player's timeline here: it holds one `PlaybackIds` and tells it
//! what it queued, what the pipeline pulled and what a stop flushed, in queue order. Played is
//! what reached the pipeline, which runs a sink's latency ahead of the speaker (the macOS app
//! reads the player's position; GStreamer pulls in 20 ms blocks).

use crate::realtime::protocol::PlaybackProgress;
use std::collections::VecDeque;

const SAMPLES_PER_MS: u64 = 24;

/// One id's account, in samples.
#[derive(Debug)]
struct Account {
    id: String,
    /// Queued and not yet pulled or flushed.
    queued: u64,
    played: u64,
    /// Flushed by a stop. An id that lost any never reads as drained again.
    dropped: u64,
    /// What the last report said of it.
    reported: Option<(u64, bool)>,
}

impl Account {
    fn progress(&self) -> (u64, bool) {
        (
            self.played / SAMPLES_PER_MS,
            self.queued == 0 && self.dropped == 0,
        )
    }
}

/// A stretch of the queue, front first: `samples` under one account, or under none (an OpenAI
/// call's audio, which carries no id).
#[derive(Debug)]
struct Run {
    account: Option<usize>,
    samples: u64,
}

/// One call's playback ids. Ids are call-local, so a new call starts a new one.
#[derive(Debug, Default)]
pub struct PlaybackIds {
    /// In the order each id's audio first arrived, which is the order reports go out in.
    accounts: Vec<Account>,
    runs: VecDeque<Run>,
}

impl PlaybackIds {
    /// `samples` were queued at the back, under `id`.
    pub fn queued(&mut self, id: Option<&str>, samples: u64) {
        if samples == 0 {
            return;
        }
        let account = id.map(|id| self.account(id));
        if let Some(index) = account {
            self.accounts[index].queued += samples;
        }
        match self.runs.back_mut() {
            Some(last) if last.account == account => last.samples += samples,
            _ => self.runs.push_back(Run { account, samples }),
        }
    }

    /// `samples` left the front of the queue for the pipeline.
    pub fn played(&mut self, samples: u64) {
        let mut left = samples;
        while left > 0 {
            let Some(front) = self.runs.front_mut() else {
                break;
            };
            let taken = left.min(front.samples);
            front.samples -= taken;
            left -= taken;
            let account = front.account;
            if front.samples == 0 {
                self.runs.pop_front();
            }
            if let Some(index) = account {
                let account = &mut self.accounts[index];
                account.queued -= taken;
                account.played += taken;
            }
        }
        debug_assert_eq!(left, 0, "the queue pulled samples it never queued");
    }

    /// A stop flushed the queue: what had not played never will.
    pub fn flushed(&mut self) {
        for account in &mut self.accounts {
            account.dropped += std::mem::take(&mut account.queued);
        }
        self.runs.clear();
    }

    /// Each id whose progress changed since the last report, in the order its audio began.
    pub fn report(&mut self) -> Vec<PlaybackProgress> {
        let mut changed = Vec::new();
        for account in &mut self.accounts {
            let now = account.progress();
            if account.reported == Some(now) {
                continue;
            }
            account.reported = Some(now);
            changed.push(PlaybackProgress {
                playback_id: account.id.clone(),
                played_ms: now.0,
                drained: now.1,
            });
        }
        changed
    }

    /// The account for `id`, opened on its first audio. Audio arrives under the newest id, so
    /// the search starts from the back.
    fn account(&mut self, id: &str) -> usize {
        if let Some(index) = self.accounts.iter().rposition(|account| account.id == id) {
            return index;
        }
        self.accounts.push(Account {
            id: id.to_owned(),
            queued: 0,
            played: 0,
            dropped: 0,
            reported: None,
        });
        self.accounts.len() - 1
    }
}
