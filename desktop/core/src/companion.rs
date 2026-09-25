//! The companion window's rules, as the macOS pet has them (`PetView.swift`): when
//! its call controls show, and which parts of the see-through window take the
//! pointer. Everywhere else a click reaches whatever is under the window.

use crate::mascot::Expression;

/// How tall each band of the mascot's outline is, in pixels.
pub const BAND: i32 = 4;
/// How far the controls' part reaches up towards the mascot's, in pixels, so
/// the pointer crosses the gap between them without leaving the window.
pub const JOIN: i32 = 12;

/// A rectangle on the window's surface, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The call controls show while the pointer is over the pet, through a call,
/// and while Fermix speaks (macOS `shouldShowControls`).
pub fn controls_shown(hovered: bool, in_call: bool, expression: Expression) -> bool {
    hovered || in_call || expression == Expression::Speaking
}

/// The parts of the window that take the pointer: the ellipse inscribed in the
/// mascot's `stage`, as bands `BAND` tall that each cover the ellipse across
/// their height, and the `controls` while they show, reaching up `JOIN` towards
/// the mascot.
pub fn input_region(stage: Rect, controls: Option<Rect>) -> Vec<Rect> {
    assert!(
        stage.width > 0 && stage.height > 0,
        "the mascot's stage needs a size, got {stage:?}"
    );
    let a = f64::from(stage.width) / 2.0;
    let b = f64::from(stage.height) / 2.0;
    let mut region = Vec::new();
    for top in (0..stage.height).step_by(BAND as usize) {
        let bottom = (top + BAND).min(stage.height);
        // Across the band the ellipse is widest nearest its centre line.
        let nearest = b.clamp(f64::from(top), f64::from(bottom));
        let dy = (nearest - b) / b;
        let half = a * (1.0 - dy * dy).max(0.0).sqrt();
        let inset = (a - half).floor() as i32;
        region.push(Rect {
            x: stage.x + inset,
            y: stage.y + top,
            width: stage.width - 2 * inset,
            height: bottom - top,
        });
    }
    if let Some(controls) = controls {
        region.push(Rect {
            y: controls.y - JOIN,
            height: controls.height + JOIN,
            ..controls
        });
    }
    region
}
