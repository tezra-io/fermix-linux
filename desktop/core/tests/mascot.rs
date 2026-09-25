//! The mascot moves as the macOS pet does, and every layer it draws ships with the app.
//!
//! Expected numbers were computed from the Swift formulas (M `Pet/MascotMotion.swift`,
//! `Pet/PetView.swift`) in a separate Python port, not from this crate.

use fermix_client::mascot::{
    blink, plates, pose, Expression, Fade, Plate, Pose, Transition, BALL, LAYERS,
};
use std::path::Path;

const PET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../app/resources/pet");
const GRESOURCE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../app/resources/resources.gresource.xml"
);

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

fn same_pose(actual: Pose, expected: (f64, f64, f64)) {
    close(actual.scale, expected.0);
    close(actual.dy, expected.1);
    close(actual.rotation_deg, expected.2);
}

fn names(plates: &[Plate]) -> Vec<&'static str> {
    plates.iter().map(|plate| plate.layer).collect()
}

// Motion: breath, bob and sway are independent sines (M `PetView.swift:159-174`).

#[test]
fn each_expression_breathes_bobs_and_sways_on_its_own_periods() {
    let t = 1.3;
    same_pose(
        pose(Expression::Idle, t, 0.0),
        (0.9968941714587698, -0.9706039250621621, 0.0),
    );
    same_pose(
        pose(Expression::Listening, t, 0.0),
        (0.9797745751406263, -0.9706039250621621, 0.0),
    );
    same_pose(
        pose(Expression::Thinking, t, 0.0),
        (0.9935123101905329, -0.33378140093447173, 1.303223248101886),
    );
    same_pose(
        pose(Expression::Speaking, t, 0.0),
        (0.9815224093497743, 2.954423259036624, 0.0),
    );
}

#[test]
fn speaking_swells_with_the_output_level() {
    same_pose(
        pose(Expression::Speaking, 1.3, 0.5),
        (1.0115224093497743, 2.954423259036624, 0.0),
    );
}

#[test]
fn the_level_moves_only_the_speaking_face() {
    assert_eq!(
        pose(Expression::Listening, 1.3, 1.0),
        pose(Expression::Listening, 1.3, 0.0)
    );
}

#[test]
fn time_zero_is_the_rest_pose() {
    for expression in Expression::ALL {
        assert_eq!(pose(expression, 0.0, 0.0), Pose::REST);
    }
}

#[test]
#[should_panic(expected = "level")]
fn a_level_outside_zero_to_one_is_a_bug() {
    pose(Expression::Speaking, 1.0, 1.5);
}

#[test]
#[should_panic(expected = "time")]
fn a_time_that_is_not_finite_is_a_bug() {
    pose(Expression::Idle, f64::NAN, 0.0);
}

// Blink: a jittered 2.8 s cadence, closing in 60 ms, holding 50 ms, opening in 100 ms
// (M `MascotMotion.swift:67-83`).

#[test]
fn the_first_blink_starts_at_time_zero() {
    close(blink(0.0), 0.0);
    close(blink(0.03), 0.5);
    close(blink(0.08), 1.0);
    close(blink(0.16), 0.5);
    close(blink(0.25), 0.0);
}

#[test]
fn later_blinks_start_at_a_jittered_point_in_their_period() {
    // Period 1 starts at 5.187178109623237, period 3 at 9.845796504424722.
    close(blink(5.177178109623237), 0.0);
    close(blink(5.217178109623237), 0.5);
    close(blink(5.267178109623237), 1.0);
    close(blink(5.347178109623237), 0.5);
    close(blink(5.437178109623237), 0.0);
    close(blink(9.875796504424722), 0.5);
    close(blink(9.925796504424722), 1.0);
    close(blink(10.095796504424722), 0.0);
}

#[test]
fn blinking_is_a_function_of_time_alone() {
    assert_eq!(blink(7.77), blink(7.77));
}

// Layers (M `PetView.swift:188-292`, `PetAssetCache.swift:143-156`).

#[test]
fn idle_is_ring_body_face_with_no_decor_and_no_blink() {
    let idle = plates(Expression::Idle, 1.0);
    assert_eq!(
        names(&idle),
        ["pet_idle_ring", "pet_idle_body", "pet_idle_face"]
    );
    assert_eq!(idle[0].scale, 1.2);
    assert!(idle.iter().all(|plate| plate.opacity == 1.0));
}

#[test]
fn listening_blinks_by_laying_the_closed_eye_face_over_its_own() {
    let listening = plates(Expression::Listening, 0.4);
    assert_eq!(
        names(&listening),
        [
            "pet_listening_ring",
            "pet_listening_body",
            "pet_listening_face",
            "pet_idle_face"
        ]
    );
    assert_eq!(listening[3].opacity, 0.4);
}

#[test]
fn an_open_eye_draws_no_blink_plate() {
    assert_eq!(plates(Expression::Listening, 0.0).len(), 3);
}

#[test]
fn thinking_blinks_under_its_decor_and_the_decor_is_translucent() {
    let thinking = plates(Expression::Thinking, 1.0);
    assert_eq!(
        names(&thinking),
        [
            "pet_thinking_ring",
            "pet_thinking_body",
            "pet_thinking_face",
            "pet_idle_face",
            "pet_thinking_decor"
        ]
    );
    assert_eq!(thinking[4].opacity, 0.75);
}

#[test]
fn the_speaking_face_is_moved_back_into_register_and_nothing_else_is() {
    let speaking = plates(Expression::Speaking, 1.0);
    assert_eq!(
        names(&speaking),
        [
            "pet_speaking_ring",
            "pet_speaking_body",
            "pet_speaking_face",
            "pet_speaking_decor"
        ]
    );
    // (-12, +24) on the 1024 canvas, drawn 108 pt tall.
    close(speaking[2].dx, -1.265625);
    close(speaking[2].dy, 2.53125);
    for plate in [speaking[0], speaking[1], speaking[3]] {
        assert_eq!((plate.dx, plate.dy), (0.0, 0.0), "{}", plate.layer);
    }
}

#[test]
fn the_ball_sits_fifteen_points_up_at_full_size() {
    assert_eq!(BALL.layer, "pet_ball");
    assert_eq!(
        (BALL.dx, BALL.dy, BALL.scale, BALL.opacity),
        (0.0, -15.0, 1.0, 1.0)
    );
}

#[test]
#[should_panic(expected = "blink")]
fn a_blink_outside_zero_to_one_is_a_bug() {
    plates(Expression::Listening, 2.0);
}

// Transition: a 0.24 s crossfade that pops from 0.97, and motion eased over 0.5 s.

fn fade(expression: Expression, weight: f64, scale: f64) -> Fade {
    Fade {
        expression,
        weight,
        scale,
    }
}

#[test]
fn a_settled_expression_draws_once_at_full_strength() {
    let settled = Transition::settled(Expression::Listening);
    assert_eq!(settled.fades(3.0), [fade(Expression::Listening, 1.0, 1.0)]);
    assert_eq!(
        settled.pose(3.0, 0.0),
        pose(Expression::Listening, 3.0, 0.0)
    );
}

#[test]
fn a_change_fades_the_old_face_out_and_pops_the_new_one_in() {
    let change = Transition::settled(Expression::Idle).change(Expression::Thinking, 10.0);
    assert_eq!(
        change.fades(10.0),
        [
            fade(Expression::Idle, 1.0, 1.0),
            fade(Expression::Thinking, 0.0, 0.97)
        ]
    );
    let halfway = change.fades(10.12);
    assert_eq!(halfway.len(), 2);
    close(halfway[0].weight, 0.5);
    close(halfway[0].scale, 0.985);
    close(halfway[1].weight, 0.5);
    close(halfway[1].scale, 0.985);
    let early = change.fades(10.06);
    close(early[1].weight, 0.15625);
    assert_eq!(change.fades(10.24), [fade(Expression::Thinking, 1.0, 1.0)]);
}

#[test]
fn motion_eases_from_the_old_expression_to_the_new_over_half_a_second() {
    let change = Transition::settled(Expression::Idle).change(Expression::Thinking, 10.0);
    same_pose(
        change.pose(10.1, 0.0),
        (1.0097109147440755, -1.6924355717605641, 0.08201940045406374),
    );
    same_pose(
        change.pose(10.25, 0.0),
        (1.0009011479712557, -0.3027089827258689, 0.25573871705647844),
    );
    assert_eq!(
        change.pose(10.5, 0.0),
        pose(Expression::Thinking, 10.5, 0.0)
    );
}

#[test]
fn asking_for_the_same_expression_again_restarts_nothing() {
    let change = Transition::settled(Expression::Idle).change(Expression::Speaking, 10.0);
    assert_eq!(change.change(Expression::Speaking, 10.1), change);
}

#[test]
fn a_change_mid_fade_starts_from_the_expression_being_faded_in() {
    let first = Transition::settled(Expression::Idle).change(Expression::Listening, 10.0);
    let second = first.change(Expression::Speaking, 10.1);
    assert_eq!(second.from, Expression::Listening);
    assert_eq!(second.to, Expression::Speaking);
    assert_eq!(second.changed_at, 10.1);
}

// Artwork: a missing layer is a packaging defect (spec §6, M `MascotArtwork.swift:59-65`).

#[test]
fn fifteen_distinct_layers_ship() {
    let mut sorted = LAYERS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 15);
}

#[test]
fn every_layer_a_face_can_draw_is_one_that_ships() {
    let mut drawn: Vec<&str> = vec![BALL.layer];
    for expression in Expression::ALL {
        drawn.extend(names(&plates(expression, 1.0)));
    }
    for layer in drawn {
        assert!(LAYERS.contains(&layer), "{layer} is drawn but not shipped");
    }
}

/// Width, height, bit depth and colour type from a PNG's IHDR.
fn png_header(path: &Path) -> (u32, u32, u8, u8) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(
        &bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "{} is not a PNG",
        path.display()
    );
    assert_eq!(
        &bytes[12..16],
        b"IHDR",
        "{} has no IHDR first",
        path.display()
    );
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (width, height, bytes[24], bytes[25])
}

#[test]
fn every_layer_is_a_256_px_rgba_png_in_the_app_resources() {
    for layer in LAYERS {
        let path = Path::new(PET_DIR).join(format!("{layer}.png"));
        assert_eq!(png_header(&path), (256, 256, 8, 6), "{}", path.display());
    }
}

#[test]
fn every_layer_is_compiled_into_the_resource_bundle() {
    let xml = std::fs::read_to_string(GRESOURCE).unwrap();
    for layer in LAYERS {
        let entry = format!("<file>pet/{layer}.png</file>");
        assert!(
            xml.contains(&entry),
            "{entry} is missing from the gresource"
        );
    }
}
