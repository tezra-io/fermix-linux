//! The shipped mascot loads into the Rive runtime, takes every name the app
//! writes, and draws each pose offscreen. It needs EGL: on a machine without a
//! GPU, Mesa's llvmpipe draws it (the SDK and CI have it).

use fermix_client::mascot::{
    Expression, FRAMES_PER_SECOND, LEVEL, MODE, SETTLE_SECONDS, STATE_MACHINE,
};
use fermix_rive::{Frame, Scene, Stage};
use std::rc::Rc;

const MASCOT: &[u8] = include_bytes!("../../app/resources/pet/FermixMascot.riv");
const SIDE: u32 = 96;

fn stage() -> Rc<Stage> {
    Stage::new(MASCOT).unwrap_or_else(|e| panic!("the stage did not open: {e}"))
}

fn scene(stage: &Rc<Stage>) -> Scene {
    Scene::new(stage, STATE_MACHINE).unwrap_or_else(|e| panic!("the scene did not load: {e}"))
}

/// Plays `seconds` at the app's frame rate, so a pose's 0.45 s blend lands.
fn play(scene: &mut Scene, seconds: f32) {
    let step = (1.0 / FRAMES_PER_SECOND) as f32;
    for _ in 0..(seconds / step).ceil() as u32 {
        scene.advance(step);
    }
}

fn drawn(frame: &Frame) -> usize {
    let (pixels, _) = frame.rgba.as_chunks::<4>();
    pixels.iter().filter(|px| px[3] > 0).count()
}

/// Pixels drawn in the outer quarters of the frame, where only the speaking
/// pose's sound waves reach.
fn beside_the_body(frame: &Frame) -> usize {
    let width = frame.width as usize;
    let band = width / 4;
    let (pixels, _) = frame.rgba.as_chunks::<4>();
    pixels
        .chunks(width)
        .flat_map(|row| row[..band].iter().chain(&row[width - band..]))
        .filter(|px| px[3] > 8)
        .count()
}

#[test]
fn every_expression_is_a_pose_the_mascot_draws() {
    let stage = stage();
    let mut scene = scene(&stage);
    let mut frames = Vec::new();
    for expression in Expression::ALL {
        scene
            .set_enum(MODE, expression.mode())
            .expect("the mode is published");
        play(&mut scene, 1.5);
        let frame = scene.render(SIDE, SIDE).expect("a frame");
        assert_eq!(frame.rgba.len(), frame.stride() * SIDE as usize);
        assert!(drawn(&frame) > 100, "{expression:?} drew almost nothing");
        frames.push(frame);
    }
    for (i, a) in frames.iter().enumerate() {
        for b in &frames[i + 1..] {
            assert_ne!(a.rgba, b.rgba, "two poses drew the same frame");
        }
    }
}

#[test]
fn frames_are_premultiplied() {
    let stage = stage();
    let mut scene = scene(&stage);
    play(&mut scene, 0.5);
    let frame = scene.render(SIDE, SIDE).expect("a frame");
    let (pixels, _) = frame.rgba.as_chunks::<4>();
    for px in pixels {
        assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3], "{px:?}");
    }
}

#[test]
fn the_voice_level_moves_the_speaking_mouth() {
    let stage = stage();
    let mut scene = scene(&stage);
    scene
        .set_enum(MODE, Expression::Speaking.mode())
        .expect("mode");
    play(&mut scene, 1.5);
    let quiet = scene.render(SIDE, SIDE).expect("a frame");
    scene
        .set_number(LEVEL, 1.0)
        .expect("the level is published");
    play(&mut scene, 0.3);
    let loud = scene.render(SIDE, SIDE).expect("a frame");
    assert_ne!(quiet.rgba, loud.rgba);
}

/// The file's blends cannot be interrupted: a pose that arrives mid-blend
/// waits for it to finish, then blends in itself. A parked mascot must play
/// long enough for both, less the frame it loses at each end of its play.
#[test]
fn a_pose_that_arrives_mid_blend_lands_before_a_parked_mascot_stops() {
    let stage = stage();
    let mut scene = scene(&stage);
    let frame = 1.0 / FRAMES_PER_SECOND as f32;
    scene
        .set_enum(MODE, Expression::Thinking.mode())
        .expect("mode");
    play(&mut scene, 1.5);
    scene
        .set_enum(MODE, Expression::Speaking.mode())
        .expect("mode");
    play(&mut scene, frame);
    scene.set_enum(MODE, Expression::Idle.mode()).expect("mode");
    play(&mut scene, 0.3);
    let waiting = scene.render(SIDE, SIDE).expect("a frame");
    assert!(
        beside_the_body(&waiting) > 0,
        "idle did not wait for the blend to speaking"
    );
    play(&mut scene, SETTLE_SECONDS as f32 - 0.3 - 2.0 * frame);
    let parked = scene.render(SIDE, SIDE).expect("a frame");
    assert_eq!(
        beside_the_body(&parked),
        0,
        "idle had not landed when the mascot parked"
    );
}

#[test]
fn two_scenes_share_one_stage_and_draw_at_their_own_sizes() {
    let stage = stage();
    let mut small = scene(&stage);
    let mut large = scene(&stage);
    assert_eq!(small.render(48, 40).expect("small").rgba.len(), 48 * 40 * 4);
    assert_eq!(
        large.render(200, 176).expect("large").rgba.len(),
        200 * 176 * 4
    );
    assert_eq!(
        small.render(64, 64).expect("resized").rgba.len(),
        64 * 64 * 4
    );
}

#[test]
fn a_name_the_file_does_not_publish_is_an_error() {
    let stage = stage();
    let missing = Scene::new(&stage, "Nope")
        .err()
        .expect("no such state machine");
    assert!(missing.0.contains("Nope"), "{missing}");
    let mut scene = scene(&stage);
    assert!(scene.set_enum(MODE, "dancing").is_err());
    assert!(scene.set_enum("colour", "idle").is_err());
    assert!(scene.set_number("volume", 0.5).is_err());
}

#[test]
fn a_file_that_is_not_rive_is_an_error() {
    let refused = Stage::new(b"not a rive file").err().expect("refused");
    assert!(refused.0.contains("could not be read"), "{refused}");
}
