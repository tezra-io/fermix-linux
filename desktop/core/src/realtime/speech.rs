//! The speech gate (spec_voice §3.3): only the user's voice reaches the daemon. The capture's
//! 48 kHz, already through the echo canceller, goes down to the wire's 24 kHz and, beside it, to
//! the 16 kHz the voice detector (earshot) reads. A 16 ms frame with no voice near it goes out as
//! digital silence, so key clicks, a fan or the room between words never reach the provider's
//! own turn detection, and every frame of speech goes out exactly as the wire's decimation of it.
//!
//! While the microphone is muted or not yet armed, the detector still hears the room, so it is
//! warm when the microphone opens: a detector that starts cold takes the first frames of any
//! steady noise for a voice. The wire gets silence instead, and nothing it held is ever sent.
//!
//! Nothing here touches GStreamer: the capture branch's appsink hands its samples to `Uplink`
//! on its streaming thread and sends what comes back while the microphone is open.

use std::collections::VecDeque;
use std::f64::consts::PI;

/// The capture's rate: webrtcdsp's, which takes 48 kHz but no rate nearer the wire's 24.
pub const CAPTURE_RATE: u32 = 48_000;
/// 100 ms of the wire's PCM16 at 24 kHz: one uplink block (spec §1.5).
pub const CHUNK_BYTES: usize = 4_800;
/// One detector frame, 16 ms, at the capture's rate.
pub const CAPTURE_FRAME: usize = 768;
/// The same 16 ms at the wire's 24 kHz.
pub const WIRE_FRAME: usize = 384;
/// The same 16 ms at earshot's 16 kHz: the only frame it reads.
const DETECTOR_FRAME: usize = 256;
const CHUNK_SAMPLES: usize = CHUNK_BYTES / 2;
/// earshot's documented threshold: a frame scoring this or more is voice.
pub const VOICE_SCORE: f32 = 0.5;
/// How far a frame looks ahead for voice: 160 ms. earshot scores the start of a word a few
/// frames late, most of all a soft consonant ("s", "f", "h"), and the pre-roll keeps it.
/// It is also how far behind the capture the wire runs.
pub const PRE_ROLL_FRAMES: u64 = 10;
/// How long a frame looks back for voice: 240 ms. It keeps a word's fading tail and the short
/// stops inside words; a longer pause goes out as silence, which the provider's own turn
/// detection (800 ms) reads as a pause, not an end.
pub const HANGOVER_FRAMES: u64 = 15;
/// The taps of both anti-alias filters. Equal, so both delay a sound alike (63 samples at
/// 48 kHz) and the detector's frame k is the wire's frame k; odd, for a whole-sample delay;
/// enough for a Blackman window to fall to its ~74 dB floor within ±1 kHz of each cutoff.
const TAPS: usize = 127;
/// The wire's band edge, 1 kHz below its 12 kHz Nyquist, where the filter has fallen away.
const WIRE_CUTOFF_HZ: f64 = 11_000.0;
/// The detector's band edge, 1 kHz below its 8 kHz Nyquist. Speech keeps its formants.
const DETECTOR_CUTOFF_HZ: f64 = 7_000.0;

/// A windowed-sinc low-pass that keeps every `factor`-th sample of the capture. It is
/// polyphase, so it computes only the samples it keeps, and it carries its last `TAPS - 1`
/// inputs across pushes, so its output is the same however the capture is split.
pub struct Decimator {
    taps: Vec<f32>,
    factor: usize,
    history: Vec<f32>,
    /// Where in the next push the next kept sample is.
    next_kept: usize,
}

impl Decimator {
    /// 48 kHz to the wire's 24 kHz.
    pub fn wire() -> Decimator {
        Decimator::new(2, WIRE_CUTOFF_HZ)
    }

    /// 48 kHz to the detector's 16 kHz.
    pub fn detector() -> Decimator {
        Decimator::new(3, DETECTOR_CUTOFF_HZ)
    }

    fn new(factor: usize, cutoff_hz: f64) -> Decimator {
        assert!(cutoff_hz < f64::from(CAPTURE_RATE) / (2.0 * factor as f64));
        Decimator {
            taps: lowpass(cutoff_hz),
            factor,
            history: vec![0.0; TAPS - 1],
            next_kept: 0,
        }
    }

    /// Filters `capture` and appends the samples it keeps to `out`.
    pub fn push(&mut self, capture: &[i16], out: &mut Vec<i16>) {
        let mut window = Vec::with_capacity(self.history.len() + capture.len());
        window.extend_from_slice(&self.history);
        window.extend(capture.iter().map(|&s| f32::from(s)));
        let mut at = self.next_kept;
        while at < capture.len() {
            // `window[at + TAPS - 1]` is `capture[at]`: the newest sample under the filter.
            let under = &window[at..at + TAPS];
            let sum: f32 = under
                .iter()
                .zip(self.taps.iter().rev())
                .map(|(x, h)| x * h)
                .sum();
            out.push(sum.round().clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16);
            at += self.factor;
        }
        self.next_kept = at - capture.len();
        let keep = window.len() - (TAPS - 1);
        self.history.copy_from_slice(&window[keep..]);
    }

    /// Forgets the capture it holds, as though it had been silence. Its phase stays, so it
    /// stays aligned with the other decimator.
    fn silence(&mut self) {
        self.history.fill(0.0);
    }
}

/// A Blackman-windowed sinc at `cutoff_hz` of the capture's rate, scaled to unity gain.
fn lowpass(cutoff_hz: f64) -> Vec<f32> {
    let cycles = cutoff_hz / f64::from(CAPTURE_RATE);
    let middle = (TAPS - 1) as f64 / 2.0;
    let raw: Vec<f64> = (0..TAPS)
        .map(|t| {
            let x = t as f64 - middle;
            let sinc = if x == 0.0 {
                2.0 * cycles
            } else {
                (2.0 * PI * cycles * x).sin() / (PI * x)
            };
            let r = t as f64 / (TAPS - 1) as f64;
            let blackman = 0.42 - 0.5 * (2.0 * PI * r).cos() + 0.08 * (4.0 * PI * r).cos();
            sinc * blackman
        })
        .collect();
    let gain: f64 = raw.iter().sum();
    raw.iter().map(|h| (h / gain) as f32).collect()
}

/// Which frames are voice. A frame is when a voiced frame lies within `PRE_ROLL_FRAMES` after
/// it or `HANGOVER_FRAMES` before it, so the decision for frame k waits for frame
/// k + `PRE_ROLL_FRAMES`. Its one fact is the latest voiced frame.
#[derive(Debug, Default)]
pub struct SpeechGate {
    frames: u64,
    latest_voice: Option<u64>,
}

impl SpeechGate {
    pub fn new() -> SpeechGate {
        SpeechGate::default()
    }

    /// Takes the next frame's verdict and decides the frame `PRE_ROLL_FRAMES` before it; `None`
    /// until there is one.
    pub fn push(&mut self, voiced: bool) -> Option<bool> {
        let frame = self.frames;
        self.frames += 1;
        if voiced {
            self.latest_voice = Some(frame);
        }
        let decided = frame.checked_sub(PRE_ROLL_FRAMES)?;
        Some(
            self.latest_voice
                .is_some_and(|v| v + HANGOVER_FRAMES >= decided),
        )
    }
}

/// The capture's way to the wire: decimated, gated to voice, and cut into `CHUNK_BYTES` blocks.
pub struct Uplink {
    wire: Decimator,
    heard: Decimator,
    detector: Box<earshot::Detector>,
    gate: SpeechGate,
    /// Decimated samples not yet a whole frame.
    wire_samples: Vec<i16>,
    heard_samples: Vec<i16>,
    /// Wire frames waiting for their decision: `PRE_ROLL_FRAMES` of them once running.
    waiting: VecDeque<Vec<i16>>,
    /// Decided samples not yet a whole block.
    decided: Vec<i16>,
}

impl Default for Uplink {
    fn default() -> Uplink {
        Uplink::new()
    }
}

impl Uplink {
    pub fn new() -> Uplink {
        Uplink {
            wire: Decimator::wire(),
            heard: Decimator::detector(),
            detector: earshot::Detector::default_boxed(),
            gate: SpeechGate::new(),
            wire_samples: Vec::new(),
            heard_samples: Vec::new(),
            waiting: VecDeque::new(),
            decided: Vec::new(),
        }
    }

    /// Takes PCM16 mono at `CAPTURE_RATE` and returns the whole PCM16 LE blocks now decided,
    /// oldest first. While the microphone is not `audible` (muted or not yet armed) the
    /// detector still hears the capture but the wire gets silence, and everything it already
    /// holds is silenced too; so what comes back is silence, and nothing captured before the
    /// microphone opens is ever sent, not even as pre-roll.
    pub fn push(&mut self, capture: &[i16], audible: bool) -> Vec<Vec<u8>> {
        if audible {
            self.wire.push(capture, &mut self.wire_samples);
        } else {
            self.silence_held();
            self.wire
                .push(&vec![0; capture.len()], &mut self.wire_samples);
        }
        self.heard.push(capture, &mut self.heard_samples);
        let frames =
            (self.wire_samples.len() / WIRE_FRAME).min(self.heard_samples.len() / DETECTOR_FRAME);
        for frame in 0..frames {
            let wire = self.wire_samples[frame * WIRE_FRAME..(frame + 1) * WIRE_FRAME].to_vec();
            let heard_at = frame * DETECTOR_FRAME;
            let voiced = self.score(heard_at) >= VOICE_SCORE;
            self.decide(wire, voiced);
        }
        self.wire_samples.drain(..frames * WIRE_FRAME);
        self.heard_samples.drain(..frames * DETECTOR_FRAME);
        self.whole_blocks()
    }

    /// Silences every wire sample held: the filter's memory, the partial frame, the frames
    /// waiting for their decision and the partial block.
    fn silence_held(&mut self) {
        self.wire.silence();
        self.wire_samples.fill(0);
        for frame in &mut self.waiting {
            frame.fill(0);
        }
        self.decided.fill(0);
    }

    fn score(&mut self, at: usize) -> f32 {
        let frame = &self.heard_samples[at..at + DETECTOR_FRAME];
        let score = self.detector.predict_i16(frame);
        assert!(
            (0.0..=1.0).contains(&score),
            "earshot scores a 256-sample frame from 0 to 1, not {score}"
        );
        score
    }

    /// Queues one wire frame and sends on the frame the gate has now decided, or silence.
    fn decide(&mut self, wire: Vec<i16>, voiced: bool) {
        self.waiting.push_back(wire);
        let Some(voice) = self.gate.push(voiced) else {
            return;
        };
        let frame = self
            .waiting
            .pop_front()
            .expect("the gate decides only a frame it was given");
        if voice {
            self.decided.extend_from_slice(&frame);
        } else {
            self.decided.resize(self.decided.len() + WIRE_FRAME, 0);
        }
    }

    fn whole_blocks(&mut self) -> Vec<Vec<u8>> {
        let (whole, _partial) = self.decided.as_chunks::<CHUNK_SAMPLES>();
        let blocks: Vec<Vec<u8>> = whole
            .iter()
            .map(|block| block.iter().flat_map(|s| s.to_le_bytes()).collect())
            .collect();
        self.decided.drain(..blocks.len() * CHUNK_SAMPLES);
        blocks
    }
}
