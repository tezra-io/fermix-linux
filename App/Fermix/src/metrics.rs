//! The one metrics module.
//!
//! Every measurement the application arranges for itself lives here, in logical
//! pixels at the default text scale. Toolkit padding, radii, focus rings and
//! control minimums are the toolkit's and are never overridden; what is below
//! is only what the application itself places.
//!
//! The stylesheet repeats two of these numbers because CSS cannot read a Rust
//! constant. `tests/structure.rs` asserts the two agree, so the repetition
//! cannot become a drift.

/// The spacing scale. Nothing in the interface uses a gap outside it.
pub const SPACE_TIGHT: i32 = 6;
/// A heading and the content under it.
pub const SPACE_HEADING: i32 = 12;
/// The outer gutter while navigation is collapsed.
pub const SPACE_GUTTER_COLLAPSED: i32 = 18;
/// The outer gutter with the sidebar shown, and the gap between groups.
pub const SPACE_GUTTER: i32 = 24;
/// The leading artwork slot, and the widest step of the scale before a page
/// break.
pub const SPACE_ARTWORK: i32 = 36;
/// The widest step of the scale.
pub const SPACE_WIDE: i32 = 48;

/// The sidebar's width.
pub const SIDEBAR_WIDTH: i32 = 216;
/// The detail column's readable budget. In single-column mode this is a
/// readability target, never a hard width.
pub const DETAIL_BUDGET: i32 = 420;

/// The content clamp: Home, Settings panes, Doctor, Recovery and every
/// assistant screen sit inside it. Logs uses the full detail width.
pub const CLAMP_MAXIMUM: i32 = 660;
/// Where the clamp starts tightening its margins.
pub const CLAMP_TIGHTENING: i32 = 480;

/// A control row's minimum height, or the toolkit's larger natural minimum.
/// Text growth is never clipped to it.
pub const ROW_MINIMUM_HEIGHT: i32 = 48;

/// The shared leading artwork slot every list that draws a mark aligns on.
pub const ARTWORK_SLOT: i32 = SPACE_ARTWORK;

/// The prefix glyph a checklist row draws at, which is the toolkit's own normal
/// symbolic icon size. It is declared here because the spinner that replaces
/// that glyph while a step runs has no icon size of its own, and a row that
/// changed height when its work started would be the list moving under
/// somebody watching it.
pub const PREFIX_GLYPH: i32 = 16;

/// One progress dot in the Setup assistant's header. Small enough that four of
/// them read as a progress indicator rather than as four icons.
pub const PROGRESS_DOT: i32 = 8;

/// Vertical padding on a row the application arranges itself, as opposed to a
/// preferences row whose padding belongs to the toolkit.
pub const ARRANGED_ROW_PADDING: i32 = SPACE_HEADING;

/// The window's default size.
pub const WINDOW_DEFAULT_WIDTH: i32 = 880;
/// The window's default size.
pub const WINDOW_DEFAULT_HEIGHT: i32 = 560;
/// The window's minimum size.
pub const WINDOW_MINIMUM_WIDTH: i32 = 460;
/// The window's minimum size.
pub const WINDOW_MINIMUM_HEIGHT: i32 = 440;

/// The two-column width budget: sidebar, readable detail and two outer
/// gutters. Below it the split view collapses. It is expressed in `sp` at the
/// breakpoint so a larger text size collapses navigation sooner.
pub const TWO_COLUMN_BUDGET: i32 = SIDEBAR_WIDTH + DETAIL_BUDGET + 2 * SPACE_GUTTER;

/// The breakpoint condition the window declares, in text-scaled units.
pub fn collapse_condition() -> String {
    format!("max-width: {TWO_COLUMN_BUDGET}sp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_column_budget_is_its_parts() {
        assert_eq!(TWO_COLUMN_BUDGET, 684);
    }

    #[test]
    fn the_breakpoint_is_expressed_in_text_scaled_units() {
        assert_eq!(collapse_condition(), "max-width: 684sp");
    }

    #[test]
    fn the_window_minimum_fits_inside_the_default() {
        const _: () = assert!(WINDOW_MINIMUM_WIDTH < WINDOW_DEFAULT_WIDTH);
        const _: () = assert!(WINDOW_MINIMUM_HEIGHT < WINDOW_DEFAULT_HEIGHT);
        assert_eq!(
            (
                WINDOW_DEFAULT_WIDTH,
                WINDOW_DEFAULT_HEIGHT,
                WINDOW_MINIMUM_WIDTH,
                WINDOW_MINIMUM_HEIGHT
            ),
            (880, 560, 460, 440),
            "the window sizes the redlines fix"
        );
    }
}
