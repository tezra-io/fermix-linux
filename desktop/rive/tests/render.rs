//! The shipped mascot loads into the Rive runtime, takes every name the app
//! writes, swells out of its intro, and draws each pose offscreen. It needs EGL: on a machine without a
//! GPU, Mesa's llvmpipe draws it (the SDK and CI have it).

use fermix_client::mascot::{
    Expression, FRAMES_PER_SECOND, INTRO_SETTLE_SECONDS, LEVEL, MODE, SETTLE_SECONDS, SKIP_INTRO,
    STATE_MACHINE,
};
use fermix_rive::{Frame, Scene, Stage};
use std::rc::Rc;

const MASCOT: &[u8] = include_bytes!("../../app/resources/pet/FermixMascot.riv");
const SIDE: u32 = 96;

fn stage() -> Rc<Stage> {
    Stage::new(MASCOT).unwrap_or_else(|e| panic!("the stage did not open: {e}"))
}

/// A scene with `skipIntro` written and its state machine not yet run: the
/// test writes what it reads at its start, then starts it.
fn unstarted(stage: &Rc<Stage>, intro: bool) -> Scene {
    let mut scene =
        Scene::new(stage, STATE_MACHINE).unwrap_or_else(|e| panic!("the scene did not load: {e}"));
    scene
        .set_boolean(SKIP_INTRO, !intro)
        .expect("skipIntro is published");
    scene
}

/// A scene started without the intro, as the app starts one: the pose tests
/// start from the poses.
fn scene(stage: &Rc<Stage>) -> Scene {
    let mut scene = unstarted(stage, false);
    scene.start(0.0);
    scene
}

/// Plays `seconds` at the app's frame rate, so a change of pose lands.
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

/// Pixels whose colour differs visibly between two frames of one size.
fn differing(a: &Frame, b: &Frame) -> usize {
    let (left, _) = a.rgba.as_chunks::<4>();
    let (right, _) = b.rgba.as_chunks::<4>();
    left.iter()
        .zip(right)
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 24))
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

/// A change of pose cannot be interrupted: a pose that arrives mid-change
/// waits for it to finish, then lands itself. A parked mascot must play
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

/// Without the intro, until its state machine has blended to a pose, a new
/// scene draws the file's setup, every pose's parts at once. Started with the
/// settle time as its lead, as a parked mascot's is, it draws its first pose
/// at once: the frame that playing the same time frame by frame reaches.
#[test]
fn a_parked_start_lands_its_first_pose() {
    let stage = stage();
    let idle = idle_at_most(&stage);
    let started = |lead: f32| {
        let mut scene = unstarted(&stage, false);
        scene.set_enum(MODE, Expression::Idle.mode()).expect("mode");
        scene.start(lead);
        scene
    };
    let setup = started(0.0).render(SIDE, SIDE).expect("a frame");
    assert!(
        beside_idle(&setup) > idle,
        "a new scene no longer starts from the setup"
    );
    let first = started(SETTLE_SECONDS as f32)
        .render(SIDE, SIDE)
        .expect("a frame");
    assert!(
        beside_idle(&first) <= idle,
        "the first pose had not landed: {} pixels beside it, idle draws at most {idle}",
        beside_idle(&first)
    );
    let mut played = started(0.0);
    play(&mut played, SETTLE_SECONDS as f32);
    let reached = played.render(SIDE, SIDE).expect("a frame");
    assert!(
        differing(&first, &reached) * 50 < (SIDE * SIDE) as usize,
        "the parked start drew {} pixels unlike the pose played frame by frame",
        differing(&first, &reached)
    );
}

/// The intro starts before any of the painted body is there, swells a sphere
/// into the pet, and ends, two seconds in, on the very frame a scene that
/// skipped it draws.
#[test]
fn the_intro_swells_a_sphere_into_the_pose() {
    let stage = stage();
    let mut intro = unstarted(&stage, true);
    let mut skipped = unstarted(&stage, false);
    for scene in [&mut intro, &mut skipped] {
        scene.set_enum(MODE, Expression::Idle.mode()).expect("mode");
        scene.start(0.0);
    }
    assert_eq!(light(&intro.render(SIDE, SIDE).expect("a frame")), 0);
    play(&mut intro, 0.2);
    play(&mut skipped, 0.2);
    let sphere = intro.render(SIDE, SIDE).expect("a frame");
    let pose = skipped.render(SIDE, SIDE).expect("a frame");
    assert!(
        drawn(&sphere) > 100 && drawn(&sphere) < drawn(&pose) * 2 / 3,
        "0.2 s in, the intro drew {} pixels, the skipped scene {}",
        drawn(&sphere),
        drawn(&pose)
    );
    play(&mut intro, INTRO_SETTLE_SECONDS as f32 - 0.2);
    play(&mut skipped, INTRO_SETTLE_SECONDS as f32 - 0.2);
    assert_eq!(
        intro.render(SIDE, SIDE).expect("a frame").rgba,
        skipped.render(SIDE, SIDE).expect("a frame").rgba,
        "the intro had not landed on the pose"
    );
}

/// The file reads `skipIntro` on the state machine's first advance, so the
/// app writes it first: written later, the intro plays all the same.
#[test]
fn skip_intro_counts_only_before_the_first_advance() {
    let stage = stage();
    let mut intro = unstarted(&stage, true);
    let mut late = unstarted(&stage, true);
    intro.start(0.0);
    late.start(0.0);
    late.set_boolean(SKIP_INTRO, true).expect("skipIntro");
    play(&mut intro, 0.2);
    play(&mut late, 0.2);
    assert_eq!(
        intro.render(SIDE, SIDE).expect("a frame").rgba,
        late.render(SIDE, SIDE).expect("a frame").rgba
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
    assert!(scene.set_boolean("muted", true).is_err());
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
