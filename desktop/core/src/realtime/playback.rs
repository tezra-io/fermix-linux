//! The reply's audio on its way to the speaker: a bounded ring of PCM16 samples that the
//! socket's reader thread fills and GStreamer's streaming thread empties, through
//! `Arc<Mutex<PlaybackQueue>>`. It also counts what reached the pipeline since the utterance
//! anchor, which is what `interrupt` reports (spec §1.7), and holds the level maths.

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

    /// Queues a decoded `audio_delta` and returns how many samples were dropped to stay within
    /// `MAX_QUEUED_SAMPLES`, oldest first. A trailing odd byte is not a sample and is ignored.
    pub fn push_pcm16(&mut self, bytes: &[u8]) -> usize {
        let incoming = bytes.len() / 2;
        let dropped = (self.samples.len() + incoming).saturating_sub(MAX_QUEUED_SAMPLES);
        let from_queue = dropped.min(self.samples.len());
        self.samples.drain(..from_queue);
        let (pairs, _odd) = bytes.as_chunks::<2>();
        let kept = pairs
            .iter()
            .skip(dropped - from_queue)
            .map(|pair| i16::from_le_bytes(*pair));
        self.samples.extend(kept);
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
        self.played += real as u64;
        Pull {
            real,
            drained: had_audio && self.samples.is_empty(),
        }
    }

    /// Drops everything queued (`playback_stop`, Stop, End) and resets the anchor. A flush is
    /// not a drain: the next pull reports `drained: false`.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.reset_anchor();
    }

    /// Starts counting a new utterance.
    pub fn reset_anchor(&mut self) {
        self.played = 0;
    }

    /// Real samples pulled since the anchor; silence does not count.
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
