//! The voice mascot is one Rive animation, as on macOS (M `Pet/MascotRendering.swift`,
//! `FermixRive/RiveMascot.swift`; the M34 design record's decision 34). Its state
//! machine takes the expression through the `mode` enum and the voice through the
//! `level` number, 0 to 1. Each mode has its own reactions inside the file (M
//! 59989d6): headphones while listening, glasses and a pearl that becomes a
//! bulb while thinking, a smiling face and a talking mouth while speaking, and
//! idle actions picked at random. A change of mode lands in about 0.6 s,
//! choreographed rather than cross-faded. Each time the state machine starts,
//! a jelly sphere swells into the pet over two seconds (M eeaaffb), unless
//! `skipIntro` is written before its first advance.
//!
//! This module is the contract between the file and the code, and the rules for
//! when the animation plays and what it is told. `app/src/mascot.rs` plays it
//! through `fermix-rive`, and draws only what these decide.

/// The file, in the app's resources (`app/resources/pet/FermixMascot.riv`).
pub const RESOURCE: &str = "/io/tezra/Fermix/pet/FermixMascot.riv";
/// The names the file publishes (M `MascotAnimation`).
pub const STATE_MACHINE: &str = "Pet";
pub const MODE: &str = "mode";
pub const LEVEL: &str = "level";
pub const SKIP_INTRO: &str = "skipIntro";

/// How often a frame is drawn while the animation plays (M `framesPerSecond`).
pub const FRAMES_PER_SECOND: f64 = 30.0;
/// How long a change of pose takes to land in the file, measured frame by
/// frame. A change cannot be interrupted: a pose that arrives mid-change
/// waits for it to finish.
const BLEND_SECONDS: f64 = 0.6;
/// A parked mascot still takes a new pose: it plays this long, then holds
/// still. That lands a pose that waited behind a blend just begun, with a
/// margin for the frames lost at either end of the play. (macOS's `settle` is
/// 0.6 s, which such a pose outlasts.)
pub const SETTLE_SECONDS: f64 = 2.0 * BLEND_SECONDS + 0.2;
/// How long the file's intro plays. Once started it cannot be skipped, and it
/// lands only when played frame by frame.
const INTRO_SECONDS: f64 = 2.0;
/// A parked mascot lets an intro it started finish: it plays this long from
/// the start, with the same margin as a pose.
pub const INTRO_SETTLE_SECONDS: f64 = INTRO_SECONDS + 0.2;
/// A level change smaller than this is not written; the mouth cannot show it
/// (M `levelStep`).
pub const LEVEL_STEP: f32 = 0.01;
/// The most one frame advances the animation, so a late frame resumes it
/// rather than jumping ahead.
pub const MAX_STEP_SECONDS: f64 = 0.1;
/// A frame due this close to its time is drawn now: a 60 Hz clock ticks every
/// 16.7 ms, and two ticks must make a 30 Hz frame.
const FRAME_SLACK_SECONDS: f64 = 0.002;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expression {
    Idle,
    Listening,
    Thinking,
    Speaking,
}

impl Expression {
    pub const ALL: [Expression; 4] = [
        Expression::Idle,
        Expression::Listening,
        Expression::Thinking,
        Expression::Speaking,
    ];

    /// The value of the file's `mode` enum (M `PetExpression.rawValue`).
    pub fn mode(self) -> &'static str {
        match self {
            Expression::Idle => "idle",
            Expression::Listening => "listening",
            Expression::Thinking => "thinking",
            Expression::Speaking => "speaking",
        }
    }

    /// Only listening and speaking move with the voice; the other poses hold
    /// the level at rest (M `listensToLevel`).
    pub fn follows_level(self) -> bool {
        matches!(self, Expression::Listening | Expression::Speaking)
    }
}

/// How a mascot's scene starts. Every time the mascot appears its scene is new,
/// so the intro is decided again (M `RiveMascot`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opening {
    /// Written to the file's `skipIntro` before the scene's first advance,
    /// which is when the file reads it.
    pub skip_intro: bool,
    /// Played in one advance as the scene starts, before the first frame is
    /// drawn.
    pub lead_seconds: f64,
    /// How long, from the start, a mascot that parks keeps playing, so that
    /// what the scene started lands.
    pub landing_seconds: Option<f64>,
}

/// An animating mascot plays the intro, which starts from nothing, so it
/// draws from the first frame. A parked one (animations off) skips it and
/// takes its pose at once: without the intro, a new scene draws the file's
/// setup, every pose's parts at once, until its first pose has blended in, so
/// that plays out before the first frame.
pub fn opening(animates: bool) -> Opening {
    if animates {
        return Opening {
            skip_intro: false,
            lead_seconds: 0.0,
            landing_seconds: Some(INTRO_SETTLE_SECONDS),
        };
    }
    Opening {
        skip_intro: true,
        lead_seconds: SETTLE_SECONDS,
        landing_seconds: None,
    }
}

/// What to write to the file's `level` for the voice at `level` in `expression`,
/// when `written` was written last: `None` when the mouth could not show the
/// difference. A return to rest is always written.
pub fn level_to_write(expression: Expression, level: f32, written: f32) -> Option<f32> {
    assert!(
        (0.0..=1.0).contains(&level),
        "the level must be 0 to 1, got {level}"
    );
    let wanted = if expression.follows_level() {
        level
    } else {
        0.0
    };
    let at_rest = wanted == 0.0 && written != 0.0;
    ((wanted - written).abs() >= LEVEL_STEP || at_rest).then_some(wanted)
}

/// Whether the animation plays at `now`: always while it `animates`; parked
/// (the view is off screen, or animations are off), only until what it shows
/// has landed at `lands_at`: a new pose, or an intro it started.
pub fn plays(animates: bool, lands_at: Option<f64>, now: f64) -> bool {
    assert!(
        now.is_finite(),
        "the mascot's time must be finite, got {now}"
    );
    animates || lands_at.is_some_and(|at| now < at)
}

/// When a parked mascot may stop, once something that takes `seconds` to land
/// starts at `now`: never before what it was already landing, so a pose that
/// arrives mid-intro does not cut the intro short.
pub fn lands_at(landing: Option<f64>, now: f64, seconds: f64) -> f64 {
    assert!(
        now.is_finite() && seconds >= 0.0,
        "a landing needs a finite time and a duration, got {now} and {seconds}"
    );
    let at = now + seconds;
    landing.map_or(at, |landing| landing.max(at))
}

/// When frames are drawn, on a frame clock that may tick faster than
/// [`FRAMES_PER_SECOND`], and how far each advances the animation.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Pacing {
    last: Option<f64>,
}

impl Pacing {
    /// At frame-clock time `now`, in seconds: the seconds to advance, when a
    /// frame is due. The first frame after a start or a pause advances nothing.
    pub fn frame(&mut self, now: f64) -> Option<f64> {
        assert!(
            now.is_finite(),
            "the mascot's time must be finite, got {now}"
        );
        let Some(last) = self.last else {
            self.last = Some(now);
            return Some(0.0);
        };
        let elapsed = now - last;
        if elapsed < 1.0 / FRAMES_PER_SECOND - FRAME_SLACK_SECONDS {
            return None;
        }
        self.last = Some(now);
        Some(elapsed.min(MAX_STEP_SECONDS))
    }

    /// The animation stopped: the next frame starts afresh.
    pub fn pause(&mut self) {
        self.last = None;
    }
}
