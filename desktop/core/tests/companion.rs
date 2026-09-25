use fermix_client::companion::{controls_shown, input_region, Rect, BAND, JOIN};
use fermix_client::mascot::Expression;

/// The mascot's stage in the companion (the macOS pet's), at the window's corner margin.
const STAGE: Rect = Rect {
    x: 6,
    y: 6,
    width: 132,
    height: 116,
};

fn covers(rects: &[Rect], x: i32, y: i32) -> bool {
    rects
        .iter()
        .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
}

#[test]
fn the_controls_show_on_hover_through_a_call_and_while_fermix_speaks() {
    assert!(!controls_shown(false, false, Expression::Idle));
    assert!(controls_shown(true, false, Expression::Idle));
    assert!(controls_shown(false, true, Expression::Listening));
    assert!(controls_shown(false, false, Expression::Speaking));
    assert!(!controls_shown(false, false, Expression::Thinking));
}

#[test]
fn the_mascot_takes_the_pointer_everywhere_inside_its_ellipse() {
    let region = input_region(STAGE, None);
    let (a, b) = (66.0, 58.0);
    let (cx, cy) = (f64::from(STAGE.x) + a, f64::from(STAGE.y) + b);
    for y in STAGE.y..STAGE.y + STAGE.height {
        for x in STAGE.x..STAGE.x + STAGE.width {
            // The pixel's centre, inside the ellipse inscribed in the stage.
            let (dx, dy) = ((f64::from(x) + 0.5 - cx) / a, (f64::from(y) + 0.5 - cy) / b);
            if dx * dx + dy * dy <= 1.0 {
                assert!(covers(&region, x, y), "({x}, {y}) is inside the ellipse");
            }
        }
    }
}

#[test]
fn the_corners_around_the_mascot_let_clicks_through() {
    let region = input_region(STAGE, None);
    let right = STAGE.x + STAGE.width - 1;
    let bottom = STAGE.y + STAGE.height - 1;
    for (x, y) in [
        (STAGE.x, STAGE.y),
        (right, STAGE.y),
        (STAGE.x, bottom),
        (right, bottom),
    ] {
        assert!(!covers(&region, x, y), "({x}, {y}) is a see-through corner");
    }
    // Most of each corner's square, not just its last pixel.
    assert!(!covers(&region, STAGE.x + 12, STAGE.y + 12));
    assert!(!covers(&region, right - 12, bottom - 12));
}

#[test]
fn the_bands_stay_on_the_stage_and_tile_it_from_top_to_bottom() {
    let region = input_region(STAGE, None);
    let mut y = STAGE.y;
    for band in &region {
        assert_eq!(band.y, y, "no gap and no overlap between bands");
        assert!(band.height > 0 && band.height <= BAND);
        assert!(band.x >= STAGE.x && band.x + band.width <= STAGE.x + STAGE.width);
        assert_eq!(
            band.x - STAGE.x,
            STAGE.x + STAGE.width - (band.x + band.width),
            "each band is centred"
        );
        y += band.height;
    }
    assert_eq!(y, STAGE.y + STAGE.height);
    let widest = region.iter().map(|r| r.width).max();
    assert_eq!(
        widest,
        Some(STAGE.width),
        "the ring reaches the stage's sides"
    );
}

#[test]
fn the_controls_join_the_mascot_so_the_pointer_can_cross_to_them() {
    let dock = Rect {
        x: 28,
        y: STAGE.y + STAGE.height + 2,
        width: 88,
        height: 34,
    };
    let region = input_region(STAGE, Some(dock));
    let without = input_region(STAGE, None);
    assert_eq!(region.len(), without.len() + 1);
    let joined = region.last().expect("the controls come last");
    assert_eq!(
        *joined,
        Rect {
            y: dock.y - JOIN,
            height: dock.height + JOIN,
            ..dock
        }
    );
    // Straight down the middle, every pixel from the mascot's centre to the
    // controls' bottom edge takes the pointer.
    let x = STAGE.x + STAGE.width / 2;
    for y in STAGE.y + STAGE.height / 2..dock.y + dock.height {
        assert!(covers(&region, x, y), "({x}, {y})");
    }
}

#[test]
#[should_panic(expected = "stage")]
fn an_empty_stage_is_a_bug() {
    input_region(
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 116,
        },
        None,
    );
}
