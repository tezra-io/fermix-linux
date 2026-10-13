//! The mascot is one Rive animation, as on macOS: the file the app ships
//! publishes every name the code writes, it is the only pet artwork that ships,
//! and the rules for starting it, writing the level, and playing and parking hold.
//! Loading and drawing the file is tested in `rive/tests/render.rs`.

use fermix_client::mascot::{
    lands_at, level_to_write, opening, plays, Expression, Opening, Pacing, FRAMES_PER_SECOND,
    INTRO_SETTLE_SECONDS, LEVEL, MAX_STEP_SECONDS, MODE, RESOURCE, SETTLE_SECONDS, SKIP_INTRO,
    STATE_MACHINE,
};
use std::path::Path;

const RESOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../app/resources");

fn shipped_file() -> Vec<u8> {
    let path = Path::new(RESOURCES).join(RESOURCE.trim_start_matches("/io/tezra/Fermix/"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

// The file.

#[test]
fn the_shipped_animation_publishes_every_name_the_code_writes() {
    let file = shipped_file();
    assert!(
        file.starts_with(b"RIVE"),
        "the mascot is not a Rive runtime file"
    );
    let modes = Expression::ALL.map(Expression::mode);
    for name in [STATE_MACHINE, MODE, LEVEL, SKIP_INTRO]
        .iter()
        .chain(modes.iter())
    {
        assert!(
            contains(&file, name),
            "the animation does not publish {name}"
        );
    }
}

#[test]
fn each_expression_is_one_of_the_files_four_modes() {
    let modes = Expression::ALL.map(Expression::mode);
    assert_eq!(modes, ["idle", "listening", "thinking", "speaking"]);
}

#[test]
fn the_animation_is_the_only_pet_artwork_that_ships() {
    let pet = Path::new(RESOURCES).join("pet");
    let mut files: Vec<String> = std::fs::read_dir(&pet)
        .expect("resources/pet")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    files.sort();
    assert_eq!(files, ["FermixMascot.riv"]);
    let bundle = std::fs::read_to_string(Path::new(RESOURCES).join("resources.gresource.xml"))
        .expect("the resource list");
    assert!(bundle.contains("<file>pet/FermixMascot.riv</file>"));
    assert!(
        !bundle.contains("pet/pet_"),
        "a painted pose is still bundled"
    );
}

// Starting.

#[test]
fn an_animating_mascot_opens_with_the_intro() {
    assert_eq!(
        opening(true),
        Opening {
            skip_intro: false,
            lead_seconds: 0.0,
            landing_seconds: Some(INTRO_SETTLE_SECONDS),
        }
    );
}

#[test]
fn a_parked_mascot_skips_the_intro_and_lands_its_pose_before_the_first_frame() {
    assert_eq!(
        opening(false),
        Opening {
            skip_intro: true,
            lead_seconds: SETTLE_SECONDS,
            landing_seconds: None,
        }
    );
}

// The level.

#[test]
fn only_listening_and_speaking_follow_the_voice() {
    let following: Vec<Expression> = Expression::ALL
        .into_iter()
        .filter(|e| e.follows_level())
        .collect();
    assert_eq!(following, [Expression::Listening, Expression::Speaking]);
}

#[test]
fn a_pose_that_does_not_follow_the_voice_holds_the_level_at_rest() {
    assert_eq!(level_to_write(Expression::Thinking, 0.8, 0.5), Some(0.0));
    assert_eq!(level_to_write(Expression::Idle, 0.8, 0.0), None);
}

#[test]
fn a_change_the_mouth_cannot_show_is_not_written() {
    assert_eq!(level_to_write(Expression::Speaking, 0.505, 0.5), None);
    assert_eq!(level_to_write(Expression::Speaking, 0.52, 0.5), Some(0.52));
    assert_eq!(level_to_write(Expression::Listening, 0.3, 0.5), Some(0.3));
}

#[test]
fn the_level_always_comes_back_to_rest() {
    assert_eq!(level_to_write(Expression::Speaking, 0.0, 0.004), Some(0.0));
    assert_eq!(level_to_write(Expression::Speaking, 0.0, 0.0), None);
}

#[test]
#[should_panic(expected = "level must be 0 to 1")]
fn a_level_out_of_range_is_a_bug() {
    level_to_write(Expression::Speaking, 1.5, 0.0);
}

// Playing and parking.

#[test]
fn an_animating_mascot_plays() {
    assert!(plays(true, None, 10.0));
    assert!(plays(true, Some(0.0), 10.0));
}

#[test]
fn a_parked_mascot_plays_only_until_what_it_shows_has_landed() {
    assert!(!plays(false, None, 10.0));
    assert!(plays(false, Some(10.0), 9.99));
    assert!(!plays(false, Some(10.0), 10.01));
}

#[test]
fn a_new_pose_lands_after_the_settle() {
    assert_eq!(lands_at(None, 10.0, SETTLE_SECONDS), 10.0 + SETTLE_SECONDS);
    assert_eq!(
        lands_at(Some(9.0), 10.0, SETTLE_SECONDS),
        10.0 + SETTLE_SECONDS
    );
}

#[test]
fn a_new_pose_does_not_cut_a_started_intro_short() {
    let intro = 10.0 + INTRO_SETTLE_SECONDS;
    assert_eq!(lands_at(Some(intro), 10.5, SETTLE_SECONDS), intro);
}

// Frames.

#[test]
fn the_first_frame_draws_the_pose_as_it_stands() {
    let mut pacing = Pacing::default();
    assert_eq!(pacing.frame(5.0), Some(0.0));
}

#[test]
fn frames_come_thirty_times_a_second_on_a_faster_clock() {
    let mut pacing = Pacing::default();
    pacing.frame(5.0);
    let tick = 1.0 / 60.0;
    assert_eq!(pacing.frame(5.0 + tick), None);
    let step = pacing.frame(5.0 + 2.0 * tick).expect("a frame is due");
    assert!(
        (step - 1.0 / FRAMES_PER_SECOND).abs() < 1e-9,
        "stepped {step}"
    );
}

#[test]
fn a_late_frame_advances_by_at_most_one_step() {
    let mut pacing = Pacing::default();
    pacing.frame(5.0);
    assert_eq!(pacing.frame(9.0), Some(MAX_STEP_SECONDS));
}

#[test]
fn after_a_pause_the_animation_resumes_where_it_stood() {
    let mut pacing = Pacing::default();
    pacing.frame(5.0);
    pacing.pause();
    assert_eq!(pacing.frame(60.0), Some(0.0));
}
