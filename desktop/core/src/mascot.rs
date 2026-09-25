//! The voice mascot's motion and layers, as pure functions of time (spec §2.2).
//! Ported from the macOS pet: M `Pet/MascotMotion.swift`, `Pet/PetView.swift`,
//! `Pet/PetExpression.swift`, `Pet/PetAssetCache.swift`. The widget in
//! `app/src/mascot.rs` only draws what these return.
//!
//! Units: time in seconds. Lengths are points at the default size, where the
//! square artwork is drawn [`ART`] points tall; the widget scales them.
//!
//! macOS keys motion off the voice mode; this keys it off the expression, each
//! expression moving as the mode of the same name does. Offline, error and tool
//! use therefore move as idle, idle and thinking do.

use std::f64::consts::TAU;

/// The stage and the mascot inside it, at the default size (M `PetFeatureModel.swift:8-9`).
pub const STAGE: (f64, f64) = (132.0, 116.0);
pub const MASCOT: (f64, f64) = (116.0, 108.0);
/// The square artwork fits the mascot by height.
pub const ART: f64 = MASCOT.1;
/// Every layer is authored on one square canvas this many pixels wide.
pub const CANVAS: f64 = 1024.0;

/// An expression change crossfades (M `Motion.swift:155-156`, stepCrossfade),
/// popping from this scale (M `PetView.swift:193`). Smoothstep stands in for
/// SwiftUI's ease-in-out curve.
pub const CROSSFADE_SECONDS: f64 = 0.24;
pub const POP_SCALE: f64 = 0.97;
/// Motion eases from the old expression's to the new one's (M `PetView.swift:126-133`).
pub const BLEND_SECONDS: f64 = 0.5;

/// Every layer file the app ships, by stem. There is no idle or listening decor (spec §6).
pub const LAYERS: [&str; 15] = [
    "pet_ball",
    "pet_idle_body",
    "pet_idle_face",
    "pet_idle_ring",
    "pet_listening_body",
    "pet_listening_face",
    "pet_listening_ring",
    "pet_thinking_body",
    "pet_thinking_decor",
    "pet_thinking_face",
    "pet_thinking_ring",
    "pet_speaking_body",
    "pet_speaking_decor",
    "pet_speaking_face",
    "pet_speaking_ring",
];

/// The shared head ball, on top of every expression and outside the crossfade
/// so it stays put while the faces swap (M `PetView.swift:177-199`).
pub const BALL: Plate = Plate {
    layer: "pet_ball",
    scale: 1.0,
    opacity: 1.0,
    dx: 0.0,
    dy: -15.0,
};

/// The ring orbits behind the mascot, so it is drawn larger (M `PetView.swift:230-235`).
const RING_SCALE: f64 = 1.20;
const DECOR_OPACITY: f64 = 0.75;
/// The speaking face is baked 12 px right and 24 px up on the canvas; this undoes it
/// (M `PetView.swift:274-282`).
const SPEAKING_FACE_SHIFT: (f64, f64) = (-12.0, 24.0);
/// Every expression sways on this period (M `PetView.swift:172-174`).
const SWAY_PERIOD: f64 = 4.2;
/// Speaking swells by this much at full output level (M `PetView.swift:162`).
const SPEAKING_PULSE: f64 = 0.06;

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
}

/// The whole mascot's transform: scaled about its centre, moved down by `dy`,
/// then turned clockwise about its centre (M `PetView.swift:101-103`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub scale: f64,
    pub dy: f64,
    pub rotation_deg: f64,
}

impl Pose {
    pub const REST: Pose = Pose {
        scale: 1.0,
        dy: 0.0,
        rotation_deg: 0.0,
    };
}

/// One layer: drawn [`ART`] square and centred, scaled about the centre, then
/// moved by `(dx, dy)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plate {
    pub layer: &'static str,
    pub scale: f64,
    pub opacity: f64,
    pub dx: f64,
    pub dy: f64,
}

/// One expression's share of a crossfade: its weight in the mix (the two
/// weights sum to 1), and its plates scaled about the centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub expression: Expression,
    pub weight: f64,
    pub scale: f64,
}

/// Sine amplitudes and periods per axis (M `MascotMotion.swift:11-58`).
struct Motion {
    breath_amp: f64,
    breath_period: f64,
    bob_amp: f64,
    bob_period: f64,
    sway_amp: f64,
}

fn motion(expression: Expression) -> Motion {
    let (breath_amp, breath_period, bob_amp, bob_period, sway_amp) = match expression {
        Expression::Idle => (0.012, 2.4, 2.0, 3.1, 0.0),
        Expression::Listening => (0.025, 2.0, 2.0, 3.1, 0.0),
        Expression::Thinking => (0.012, 2.2, 1.5, 2.8, 1.4),
        Expression::Speaking => (0.020, 1.6, 3.0, 1.8, 0.0),
    };
    Motion {
        breath_amp,
        breath_period,
        bob_amp,
        bob_period,
        sway_amp,
    }
}

/// Where an expression is at time `t`. `level` is the smoothed output RMS, 0 to 1,
/// and only swells the speaking face (M `PetView.swift:159-174`).
pub fn pose(expression: Expression, t: f64, level: f32) -> Pose {
    assert!(t.is_finite(), "mascot time must be finite, got {t}");
    assert!(
        (0.0..=1.0).contains(&level),
        "output level must be 0 to 1, got {level}"
    );
    let m = motion(expression);
    let pulse = match expression {
        Expression::Speaking => SPEAKING_PULSE * f64::from(level),
        _ => 0.0,
    };
    Pose {
        scale: 1.0 + m.breath_amp * (TAU * t / m.breath_period).sin() + pulse,
        dy: -m.bob_amp * (TAU * t / m.bob_period).sin(),
        rotation_deg: m.sway_amp * (TAU * t / SWAY_PERIOD).sin(),
    }
}

/// How closed the eyes are at time `t`, 0 to 1: a jittered 2.8 s cadence with a
/// fast close, a brief hold and a slower open. Deterministic in `t`
/// (M `MascotMotion.swift:67-83`).
pub fn blink(t: f64) -> f64 {
    assert!(t.is_finite(), "mascot time must be finite, got {t}");
    let (period, close, hold, open) = (2.8, 0.06, 0.05, 0.10);
    let span = close + hold + open;
    let bucket = (t / period).floor();
    let jitter = fract((bucket * 12.9898).sin() * 43_758.545_3);
    let age = t - (bucket * period + jitter * (period - span));
    if !(0.0..span).contains(&age) {
        return 0.0;
    }
    if age < close {
        return smoothstep(age / close);
    }
    if age < close + hold {
        return 1.0;
    }
    1.0 - smoothstep((age - close - hold) / open)
}

/// An expression's plates, back to front: ring, body, face, the closed-eye face
/// at `blink` over the listening and thinking faces, then any decor. The ball is
/// [`BALL`], drawn once above the crossfade (M `PetView.swift:230-262`).
pub fn plates(expression: Expression, blink: f64) -> Vec<Plate> {
    assert!(
        (0.0..=1.0).contains(&blink),
        "blink must be 0 to 1, got {blink}"
    );
    let (ring, body, face, decor) = match expression {
        Expression::Idle => ("pet_idle_ring", "pet_idle_body", "pet_idle_face", None),
        Expression::Listening => (
            "pet_listening_ring",
            "pet_listening_body",
            "pet_listening_face",
            None,
        ),
        Expression::Thinking => (
            "pet_thinking_ring",
            "pet_thinking_body",
            "pet_thinking_face",
            Some("pet_thinking_decor"),
        ),
        Expression::Speaking => (
            "pet_speaking_ring",
            "pet_speaking_body",
            "pet_speaking_face",
            Some("pet_speaking_decor"),
        ),
    };
    let (dx, dy) = face_shift(expression);
    let mut stack = vec![
        plate(ring, RING_SCALE, 1.0),
        plate(body, 1.0, 1.0),
        Plate {
            dx,
            dy,
            ..plate(face, 1.0, 1.0)
        },
    ];
    let blinks = matches!(expression, Expression::Listening | Expression::Thinking);
    if blinks && blink > 0.0 {
        stack.push(Plate {
            dx,
            dy,
            ..plate("pet_idle_face", 1.0, blink)
        });
    }
    if let Some(decor) = decor {
        stack.push(plate(decor, 1.0, DECOR_OPACITY));
    }
    stack
}

fn plate(layer: &'static str, scale: f64, opacity: f64) -> Plate {
    Plate {
        layer,
        scale,
        opacity,
        dx: 0.0,
        dy: 0.0,
    }
}

/// The speaking face's registration fix, from canvas pixels to points.
fn face_shift(expression: Expression) -> (f64, f64) {
    match expression {
        Expression::Speaking => (
            SPEAKING_FACE_SHIFT.0 * ART / CANVAS,
            SPEAKING_FACE_SHIFT.1 * ART / CANVAS,
        ),
        _ => (0.0, 0.0),
    }
}

/// The expression on show and the one it is changing from. A change crossfades
/// the faces and eases the motion across (M `PetView.swift:116-157`, `:188-200`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub from: Expression,
    pub to: Expression,
    pub changed_at: f64,
}

impl Transition {
    /// An expression shown since forever, so nothing is fading.
    pub fn settled(expression: Expression) -> Transition {
        Transition {
            from: expression,
            to: expression,
            changed_at: f64::NEG_INFINITY,
        }
    }

    /// Changes to `to` at `now`. Asking for the expression already on show
    /// restarts nothing (M `PetView.swift:119-124`).
    pub fn change(self, to: Expression, now: f64) -> Transition {
        assert!(now.is_finite(), "mascot time must be finite, got {now}");
        if to == self.to {
            return self;
        }
        Transition {
            from: self.to,
            to,
            changed_at: now,
        }
    }

    /// The pose at `t`, eased from the old expression's to the new one's.
    pub fn pose(self, t: f64, level: f32) -> Pose {
        let target = pose(self.to, t, level);
        let progress = eased(t - self.changed_at, BLEND_SECONDS);
        if progress >= 1.0 {
            return target;
        }
        let from = pose(self.from, t, level);
        let lerp = |a: f64, b: f64| a + (b - a) * progress;
        Pose {
            scale: lerp(from.scale, target.scale),
            dy: lerp(from.dy, target.dy),
            rotation_deg: lerp(from.rotation_deg, target.rotation_deg),
        }
    }

    /// What to draw at `t`, back to front: the old expression fading out and
    /// shrinking to [`POP_SCALE`], under the new one fading in and growing from it.
    pub fn fades(self, t: f64) -> Vec<Fade> {
        let progress = eased(t - self.changed_at, CROSSFADE_SECONDS);
        let pop = 1.0 - POP_SCALE;
        if progress >= 1.0 || self.from == self.to {
            return vec![Fade {
                expression: self.to,
                weight: 1.0,
                scale: 1.0,
            }];
        }
        vec![
            Fade {
                expression: self.from,
                weight: 1.0 - progress,
                scale: 1.0 - pop * progress,
            },
            Fade {
                expression: self.to,
                weight: progress,
                scale: POP_SCALE + pop * progress,
            },
        ]
    }
}

/// Smoothstep progress `age` seconds into a `duration`: 0 at the start, 1 once
/// done. An age outside the window counts as done, as on macOS.
fn eased(age: f64, duration: f64) -> f64 {
    if !(0.0..duration).contains(&age) {
        return 1.0;
    }
    smoothstep(age / duration)
}

fn smoothstep(x: f64) -> f64 {
    let c = x.clamp(0.0, 1.0);
    c * c * (3.0 - 2.0 * c)
}

fn fract(x: f64) -> f64 {
    x - x.floor()
}
