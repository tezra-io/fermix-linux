//! The reply's audio on its way to the speaker: a bounded ring of PCM16 samples that the
//! socket's reader thread fills and GStreamer's streaming thread empties, through
//! `Arc<Mutex<PlaybackQueue>>`. It also counts what reached the pipeline since the utterance
//! anchor, which is what `interrupt` reports (spec §1.7), what has played of each playback id
//! (`PlaybackIds`, a Grok call's `playback_progress`), and holds the level maths.

use crate::realtime::playback_ids::PlaybackIds;
use crate::realtime::protocol::PlaybackProgress;
use std::collections::VecDeque;

/// PCM16 LE mono, in both directions.
pub const SAMPLE_RATE: u32 = 24_000;
/// 30 s. Beyond it the oldest samples drop.
pub const MAX_QUEUED_SAMPLES: usize = 30 * 24_000;
/// How far each new level moves the shown one (macOS `AudioOwner.swift:57`).
const LEVEL_SMOOTHING: f32 = 0.35;
const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1_000;

#[derive(Debug, Default)]
pub struct PlaybackQueue {
    samples: VecDeque<i16>,
    played: u64,
    /// The front samples that were already queued when the anchor reset: the tail of the
    /// utterance before it, so they are not counted when they play.
    before_anchor: usize,
    ids: PlaybackIds,
}

/// What one `pull` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pull {
    /// Samples that came from the queue; the rest of the buffer is silence.
    pub real: usize,
    /// The queue went from non-empty to empty in this pull.
    pub drained: bool,
}

impl PlaybackQueue {
    pub fn new() -> PlaybackQueue {
        PlaybackQueue::default()
    }

    /// Queues a decoded `audio_delta`, sent under `playback_id` if it carries one, and returns
    /// how many samples were dropped to stay within `MAX_QUEUED_SAMPLES`, oldest first. A
    /// trailing odd byte is not a sample and is ignored.
    ///
    /// A dropped sample counts as played: the daemon waits on every sample it sent under an id,
    /// and keeps about ten seconds on the app, so a Grok call never reaches the bound.
    pub fn push_pcm16(&mut self, bytes: &[u8], playback_id: Option<&str>) -> usize {
        let (pairs, _odd) = bytes.as_chunks::<2>();
        self.samples
            .extend(pairs.iter().map(|pair| i16::from_le_bytes(*pair)));
        self.ids.queued(playback_id, pairs.len() as u64);
        let dropped = self.samples.len().saturating_sub(MAX_QUEUED_SAMPLES);
        self.samples.drain(..dropped);
        self.before_anchor = self.before_anchor.saturating_sub(dropped);
        self.ids.played(dropped as u64);
        dropped
    }

    /// Fills `out` from the queue and with zeros past its end, so the pipeline never underruns
    /// and the echo probe always has a reference.
    pub fn pull(&mut self, out: &mut [i16]) -> Pull {
        let had_audio = !self.samples.is_empty();
        let real = out.len().min(self.samples.len());
        for (slot, sample) in out.iter_mut().zip(self.samples.drain(..real)) {
            *slot = sample;
        }
        out[real..].fill(0);
        let earlier = real.min(self.before_anchor);
        self.before_anchor -= earlier;
        self.played += (real - earlier) as u64;
        self.ids.played(real as u64);
        Pull {
            real,
            drained: had_audio && self.samples.is_empty(),
        }
    }

    /// Drops everything queued (`playback_stop`, Stop, End) and resets the anchor. A flush is
    /// not a drain: the next pull reports `drained: false`. What had played of each playback
    /// id stays played, and the rest is lost.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.ids.flushed();
        self.reset_anchor();
    }

    /// Empties the queue for a new call, whose playback ids are its own.
    pub fn start_call(&mut self) {
        self.clear();
        self.ids = PlaybackIds::default();
    }

    /// What changed for each playback id since the last call to this.
    pub fn take_progress(&mut self) -> Vec<PlaybackProgress> {
        self.ids.report()
    }

    /// Starts counting a new utterance, from after what is queued now.
    pub fn reset_anchor(&mut self) {
        self.played = 0;
        self.before_anchor = self.samples.len();
    }

    /// Real samples pulled since the anchor; silence does not count, nor does the tail that
    /// was still queued when the anchor reset.
    pub fn played_since_anchor(&self) -> u64 {
        self.played
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// How much of the utterance reached the speaker: the samples pulled, less what is still inside
/// the sink. Saturates at zero.
pub fn played_ms(played_samples: u64, sink_latency_ms: u64) -> u64 {
    (played_samples / SAMPLES_PER_MS).saturating_sub(sink_latency_ms)
}

/// The root-mean-square level of PCM16 LE audio, 0.0 to 1.0. A trailing odd byte is ignored.
pub fn rms(pcm16: &[u8]) -> f32 {
    let (pairs, _odd) = pcm16.as_chunks::<2>();
    if pairs.is_empty() {
        return 0.0;
    }
    let sum: f64 = pairs
        .iter()
        .map(|pair| f64::from(i16::from_le_bytes(*pair)) / f64::from(i16::MAX))
        .map(|sample| sample * sample)
        .sum();
    ((sum / pairs.len() as f64).sqrt() as f32).min(1.0)
}

/// The shown level after a new one arrives: chunk-by-chunk RMS jitters, the pet should swell.
pub fn smooth(previous: f32, next: f32) -> f32 {
    (1.0 - LEVEL_SMOOTHING) * previous + LEVEL_SMOOTHING * next
}
