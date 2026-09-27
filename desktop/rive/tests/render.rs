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

/// Light, nearly opaque pixels: the pet's white body. The body and the pearl
/// on its head are PNG images inside the file; without them only the navy
/// face is drawn.
fn light(frame: &Frame) -> usize {
    let (pixels, _) = frame.rgba.as_chunks::<4>();
    pixels
        .iter()
        .filter(|px| px[3] > 200 && px[0] > 180 && px[1] > 180 && px[2] > 180)
        .count()
}

/// Pixels drawn in the side bands at mid-height, where the idle body never
/// reaches: the other poses bend the body out there, and speaking draws its
/// sound waves there.
fn beside_idle(frame: &Frame) -> usize {
    let width = frame.width as usize;
    let rows = frame.height as usize;
    let band = width * 18 / 100;
    let (pixels, _) = frame.rgba.as_chunks::<4>();
    pixels
        .chunks(width)
        .skip(rows * 35 / 100)
        .take(rows * 30 / 100)
        .flat_map(|row| row[..band].iter().chain(&row[width - band..]))
        .filter(|px| px[3] > 8)
        .count()
}

/// The most idle draws in those bands over one loop of its animation (4.8 s in
/// the file), once its first pose has landed.
fn idle_at_most(stage: &Rc<Stage>) -> usize {
    let mut scene = scene(stage);
    scene.set_enum(MODE, Expression::Idle.mode()).expect("mode");
    play(&mut scene, SETTLE_SECONDS as f32);
    let frame = 1.0 / FRAMES_PER_SECOND as f32;
    (0..FRAMES_PER_SECOND as usize * 5)
        .map(|_| {
            play(&mut scene, frame);
            beside_idle(&scene.render(SIDE, SIDE).expect("a frame"))
        })
        .max()
        .expect("frames")
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
    let idle = idle_at_most(&stage);
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
        beside_idle(&waiting) > idle,
        "idle did not wait for the blend to speaking"
    );
    play(&mut scene, SETTLE_SECONDS as f32 - 0.3 - 2.0 * frame);
    let parked = scene.render(SIDE, SIDE).expect("a frame");
    assert!(
        beside_idle(&parked) <= idle,
        "idle had not landed when the mascot parked: {} pixels beside it, idle draws at most {idle}",
        beside_idle(&parked)
    );
}

/// Until its state machine has blended to a pose, a new scene draws the
/// file's setup, every pose's parts at once. One advance by the settle time
/// lands the first pose, which is what the app does before its first frame.
#[test]
fn one_settle_lands_a_new_scenes_first_pose() {
    let stage = stage();
    let idle = idle_at_most(&stage);
    let mut scene = scene(&stage);
    let setup = scene.render(SIDE, SIDE).expect("a frame");
    assert!(
        beside_idle(&setup) > idle,
        "a new scene no longer starts from the setup"
    );
    scene.set_enum(MODE, Expression::Idle.mode()).expect("mode");
    scene.advance(SETTLE_SECONDS as f32);
    let first = scene.render(SIDE, SIDE).expect("a frame");
    assert!(
        beside_idle(&first) <= idle,
        "the first pose had not landed: {} pixels beside it, idle draws at most {idle}",
        beside_idle(&first)
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
fn the_pets_body_is_drawn_from_the_image_inside_the_file() {
    let stage = stage();
    let mut scene = scene(&stage);
    play(&mut scene, 0.5);
    let frame = scene.render(SIDE, SIDE).expect("a frame");
    let share = light(&frame) as f64 / f64::from(SIDE * SIDE);
    assert!(
        share > 0.1,
        "only {share:.3} of the frame is the white body"
    );
}

#[test]
fn a_file_whose_image_does_not_decode_is_an_error() {
    let mut broken = MASCOT.to_vec();
    let png = broken
        .windows(8)
        .position(|bytes| bytes == b"\x89PNG\r\n\x1a\n")
        .expect("the file embeds a PNG");
    broken[png + 1] = b'X';
    let refused = Stage::new(&broken).err().expect("refused");
    assert!(refused.0.contains("pearl"), "{refused}");
}

#[test]
fn a_file_that_is_not_rive_is_an_error() {
    let refused = Stage::new(b"not a rive file").err().expect("refused");
    assert!(refused.0.contains("could not be read"), "{refused}");
}
