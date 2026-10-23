//! A voice note's waveform, ported from the macOS app (`SoundLevels`, `SoundPower`,
//! `ChatWaveformLayout`): the note's loudness in `BUCKETS` equal stretches of what was really
//! decoded, each stretch's mean power on a decibel curve from a floor of silence up to the loudest
//! stretch, merged to the bars that fit, each bar the loudest it covers. A quiet room is drawn
//! against `QUIET_TOP`, never stretched into a wave. The length is the frames decoded over the
//! rate: a container's own duration can lie. No GStreamer: the app's levels pass adds what its
//! appsink pulls and asks `heard` what it came to.

use std::fmt;
use std::ops::Range;

/// How many stretches a note is read in: as many bars as fit the widest chip, so drawing only
/// ever merges them.
pub const BUCKETS: usize = 80;
/// The quietest a stretch is heard at, in dB below full scale: anything quieter is silence.
pub const FLOOR: f32 = -60.0;
/// The least the loudest stretch is taken to be: a note with nothing louder is a quiet room.
pub const QUIET_TOP: f32 = -30.0;
/// The rate a note is decoded at for its levels, mono: plenty for loudness.
pub const RATE: u32 = 8_000;
/// Frames in a window of power: 10 ms at `RATE`.
pub const WINDOW_FRAMES: u64 = RATE as u64 / 100;
/// Windows held at most: a power of two, so every merge is of pairs.
pub const CAPACITY: usize = 1_024;
/// The longest a note is read for: ten minutes. Past it the pass stops and the note rests.
pub const MAX_FRAMES: u64 = 10 * 60 * RATE as u64;
/// The largest note file read at all.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;

/// A bar's width, the gap after it, and the shortest a bar is drawn, in pixels.
pub const BAR_WIDTH: f64 = 2.0;
pub const BAR_GAP: f64 = 2.0;
pub const BAR_MINIMUM: f64 = 3.0;
/// The level every bar rests at until the note's levels are read.
pub const RESTING: f32 = 0.15;

/// A note's levels, each from 0, silence, to 1, its loudest, and its decoded length.
#[derive(Debug, Clone, PartialEq)]
pub struct Levels {
    values: Vec<f32>,
    seconds: f64,
}

impl Levels {
    pub fn new(values: Vec<f32>, seconds: f64) -> Levels {
        Levels { values, seconds }
    }

    /// Levels from each stretch's mean power, on a decibel curve from the floor up to the loudest
    /// stretch, or `QUIET_TOP` where nothing is louder. A power that is no number is silence.
    pub fn from_power(power: &[f64], seconds: f64) -> Levels {
        let decibels: Vec<f32> = power
            .iter()
            .map(|p| {
                if p.is_finite() && *p > 0.0 {
                    (10.0 * p.log10()) as f32
                } else {
                    FLOOR
                }
            })
            .collect();
        let top = decibels
            .iter()
            .copied()
            .fold(FLOOR, f32::max)
            .max(QUIET_TOP);
        let values = decibels
            .iter()
            .map(|db| ((db - FLOOR) / (top - FLOOR)).clamp(0.0, 1.0))
            .collect();
        Levels { values, seconds }
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn seconds(&self) -> f64 {
        self.seconds
    }

    /// The levels merged to `count` bars, each the loudest of the stretches it covers, so a peak
    /// is never lost.
    pub fn bars(&self, count: usize) -> Vec<f32> {
        spans(self.values.len(), count)
            .into_iter()
            .map(|span| self.values[span].iter().copied().fold(0.0, f32::max))
            .collect()
    }
}

/// `count` things shared out in order among `parts`, as evenly as they go: every part has at least
/// one where there is any, a part may share one with its neighbour where there are fewer things
/// than parts, and none reaches past the end.
pub fn spans(count: usize, parts: usize) -> Vec<Range<usize>> {
    (0..parts)
        .map(|part| {
            let lower = part * count / parts;
            let upper = ((part + 1) * count / parts).max(lower + 1).min(count);
            lower..upper
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
struct Window {
    sum: f64,
    frames: u64,
}

/// A note's power as it is decoded, summed in windows of equal length and merged in pairs
/// whenever there would be `CAPACITY` of them: a long note costs no more to hold than a short one,
/// and nothing about its length need be known before it ends.
#[derive(Debug, Clone)]
pub struct Power {
    window_frames: u64,
    frames: u64,
    windows: Vec<Window>,
    open: Window,
}

impl Power {
    pub fn new(window_frames: u64) -> Power {
        assert!(window_frames > 0, "a window holds at least one frame");
        Power {
            window_frames,
            frames: 0,
            windows: Vec::with_capacity(CAPACITY),
            open: Window::default(),
        }
    }

    /// Every frame added so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Frames in a window now, doubled at each merge.
    pub fn window_frames(&self) -> u64 {
        self.window_frames
    }

    pub fn add(&mut self, samples: &[f32]) {
        self.frames += samples.len() as u64;
        for sample in samples {
            self.open.sum += f64::from(sample * sample);
            self.open.frames += 1;
            if self.open.frames == self.window_frames {
                self.close();
            }
        }
    }

    /// Each of `count` equal stretches' mean power, the window still open among them; none where
    /// nothing was decoded.
    pub fn stretches(&self, count: usize) -> Vec<f64> {
        let mut all = self.windows.clone();
        if self.open.frames > 0 {
            all.push(self.open);
        }
        if all.is_empty() {
            return Vec::new();
        }
        spans(all.len(), count)
            .into_iter()
            .map(|span| mean(&all[span]))
            .collect()
    }

    fn close(&mut self) {
        self.windows.push(std::mem::take(&mut self.open));
        if self.windows.len() < CAPACITY {
            return;
        }
        self.windows = self
            .windows
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[first, second]| Window {
                sum: first.sum + second.sum,
                frames: first.frames + second.frames,
            })
            .collect();
        self.window_frames *= 2;
    }
}

fn mean(windows: &[Window]) -> f64 {
    let sum: f64 = windows.iter().map(|window| window.sum).sum();
    let frames: u64 = windows.iter().map(|window| window.frames).sum();
    if frames == 0 {
        return 0.0;
    }
    sum / frames as f64
}

/// Why a note has no levels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unheard {
    /// Its file is larger than `MAX_BYTES`: not read.
    TooBig { bytes: u64 },
    /// It runs past `MAX_FRAMES`: the pass stopped there.
    TooLong,
    /// It decoded to nothing.
    Silent,
    /// It is no Ogg Opus this app decodes, or is broken: the pipeline's own words.
    Undecodable(String),
}

impl Unheard {
    /// Whether the note shows as its file tile instead of a player. One that is merely long, big
    /// or silent keeps its resting bars and still plays.
    pub fn falls_back(&self) -> bool {
        matches!(self, Unheard::Undecodable(_))
    }
}

impl fmt::Display for Unheard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unheard::TooBig { bytes } => write!(f, "{bytes} bytes is past {MAX_BYTES}"),
            Unheard::TooLong => write!(f, "longer than {MAX_FRAMES} frames"),
            Unheard::Silent => write!(f, "decoded to nothing"),
            Unheard::Undecodable(why) => write!(f, "not decoded: {why}"),
        }
    }
}

/// Whether a note's file of `bytes` is read for levels at all.
pub fn admit(bytes: u64) -> Result<(), Unheard> {
    if bytes > MAX_BYTES {
        return Err(Unheard::TooBig { bytes });
    }
    Ok(())
}

/// What a levels pass came to: its failure, where the pipeline failed, and otherwise the levels
/// of every frame it decoded, which are the note's length at `RATE`.
pub fn heard(power: &Power, failure: Option<String>) -> Result<Levels, Unheard> {
    if let Some(why) = failure {
        return Err(Unheard::Undecodable(why));
    }
    if power.frames() > MAX_FRAMES {
        return Err(Unheard::TooLong);
    }
    if power.frames() == 0 {
        return Err(Unheard::Silent);
    }
    let seconds = power.frames() as f64 / f64::from(RATE);
    Ok(Levels::from_power(&power.stretches(BUCKETS), seconds))
}

/// How many bars fit `width`, the last with no gap after it.
pub fn bars_fitting(width: f64) -> usize {
    let fitting = ((width + BAR_GAP) / (BAR_WIDTH + BAR_GAP)).floor();
    if fitting.is_nan() || fitting < 0.0 {
        return 0;
    }
    fitting as usize
}

/// Each bar's level: the note's own merged to `count` bars, or the resting bars until they are
/// read. Either way there are `count`, so nothing moves when the levels come.
pub fn bar_levels(levels: Option<&Levels>, count: usize) -> Vec<f32> {
    match levels {
        Some(levels) => levels.bars(count),
        None => vec![RESTING; count],
    }
}

/// How many of `count` bars a note that has played `fraction` of itself has filled: a bar fills
/// once the place passes its middle.
pub fn played(fraction: f64, count: usize) -> usize {
    if fraction.is_nan() {
        return 0;
    }
    (fraction.clamp(0.0, 1.0) * count as f64).round() as usize
}

/// A bar's height in a waveform `height` tall, never under `BAR_MINIMUM`, so silence still reads
/// as a line.
pub fn bar_height(level: f32, height: f64) -> f64 {
    let level = if level.is_nan() {
        0.0
    } else {
        level.clamp(0.0, 1.0)
    };
    BAR_MINIMUM + f64::from(level) * (height - BAR_MINIMUM).max(0.0)
}
