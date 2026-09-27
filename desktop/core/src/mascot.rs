//! The voice mascot is one Rive animation, as on macOS (M `Pet/MascotRendering.swift`,
//! `FermixRive/RiveMascot.swift`; the M34 design record's decision 34). Its state
//! machine takes the expression through the `mode` enum and the voice through the
//! `level` number, 0 to 1: the poses blend into each other over 0.45 s, the body
//! bends, the eyes morph, and the mouth follows the voice.
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

/// How often a frame is drawn while the animation plays (M `framesPerSecond`).
pub const FRAMES_PER_SECOND: f64 = 30.0;
/// How long the file blends one pose into the next. A blend cannot be
/// interrupted: a pose that arrives mid-blend waits for it to finish.
const BLEND_SECONDS: f64 = 0.45;
/// A parked mascot still takes a new pose: it plays this long, then holds
/// still. That lands a pose that waited behind a blend just begun, with a
/// margin for the frames lost at either end of the play. (macOS's `settle` is
/// 0.6 s, which such a pose outlasts.)
pub const SETTLE_SECONDS: f64 = 2.0 * BLEND_SECONDS + 0.2;
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
/// (the view is off screen, or animations are off), only until the pose
/// changed at `pose_changed_at` has landed.
pub fn plays(animates: bool, pose_changed_at: Option<f64>, now: f64) -> bool {
    assert!(
        now.is_finite(),
        "the mascot's time must be finite, got {now}"
    );
    animates || pose_changed_at.is_some_and(|at| now - at < SETTLE_SECONDS)
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
