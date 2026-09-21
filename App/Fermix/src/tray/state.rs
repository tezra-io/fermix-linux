//! What the tray icon shows, derived from what Home already knows.
//!
//! The glyph is a glance and the line under it is the answer, so they are
//! deliberately not the same width. Three glyphs say roughly where things
//! stand; the line says which of the five states it actually is. macOS makes
//! the same split, and for the same reason: an icon that tried to distinguish
//! "setup required" from "restart to finish updating" at 18 pixels would
//! distinguish neither.
//!
//! Nothing here reads the daemon. `StatusWord` and the attention rows are
//! computed once in `models::home` from the service status the app already
//! polls, and this module is a pure function of them, so the icon cannot
//! disagree with Home about whether Fermix is running. That is the whole
//! reason this is a separate, pure module: a second read would be a second
//! opinion.

use crate::models::home::StatusWord;
use crate::tray::item::Outcome;

/// The three marks the tray can draw.
///
/// Deliberately three and not five. See the module comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayGlyph {
    /// The daemon is up and nothing wants the operator.
    Running,
    /// Nothing has been read yet. The mark in lighter ink.
    Starting,
    /// Something needs a person: the daemon is down, setup is unfinished, an
    /// update is half-applied, or a row is waiting under Attention.
    Attention,
}

impl TrayGlyph {
    /// The icon name the host theme resolves for this glyph.
    ///
    /// One name today, because the package installs one symbolic icon and a
    /// name the theme cannot resolve draws nothing at all. The three states are
    /// still distinct in the accessibility text and the state line, so the
    /// distinction is not lost on anyone who cannot see 18 pixels of ink
    /// either. When per-state symbolics are drawn and packaged, this is the one
    /// place that changes.
    pub fn icon_name(self) -> &'static str {
        "io.tezra.Fermix-symbolic"
    }
}

/// The glyph for a status word and whatever is waiting under Attention.
///
/// `attention_rows` is the count Home computed, not a second reading of it. A
/// running daemon with a row waiting still shows the attention mark, which is
/// the rule macOS applies: anything the operator must look at wins over how the
/// service itself is doing.
pub fn glyph(status: StatusWord, attention_rows: usize) -> TrayGlyph {
    if attention_rows > 0 {
        return TrayGlyph::Attention;
    }

    match status {
        StatusWord::Running => TrayGlyph::Running,
        StatusWord::Unknown => TrayGlyph::Starting,
        StatusWord::SetupRequired
        | StatusWord::RestartToFinishUpdating
        | StatusWord::NotRunning => TrayGlyph::Attention,
    }
}

/// Whether an icon actually reached a panel, and so whether the application may
/// outlive its last window.
///
/// This is the whole of the Fedora case, written as a function so it can be
/// argued with in a test rather than buried in a branch at a call site.
///
/// Only a CONFIRMED registration earns the hold -- not an attempt, and not a
/// refusal:
///
///   * `Registered`: a watcher took the item, so there is something on a panel
///     to reach the application by. Holding is safe.
///   * `NoWatcher`: this desktop has no tray. Correct on Fedora's GNOME rather
///     than a fault, and there is nothing to click. Holding would leave a
///     process running with no window and no way back to it.
///   * `Refused`: a tray was there and would not take us. That is a defect, and
///     it leaves the user in exactly the situation no tray leaves them in --
///     no icon. So it must not hold either. A defect that strands an invisible
///     process is worse than one that merely loses an icon.
///
/// The release is deliberately not the mirror of this: it runs unconditionally
/// on the quit path, because a release that is harmless when nothing was held
/// is safer than one that is correct only while two facts agree.
pub fn may_outlive_its_window(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Registered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_confirmed_registration_lets_the_application_outlive_its_window() {
        assert!(may_outlive_its_window(&Outcome::Registered));
    }

    #[test]
    fn a_desktop_with_no_tray_must_still_quit_with_its_last_window() {
        // Fedora's GNOME. There is no icon, so a held process would be
        // unreachable: no window, and nothing on any panel to open one.
        assert!(!may_outlive_its_window(&Outcome::NoWatcher));
    }

    #[test]
    fn a_tray_that_refused_us_must_still_quit_with_its_last_window() {
        // The case that is easy to get wrong, because it is a defect and the
        // instinct is to treat a defect as "we tried". The user sees what the
        // Fedora user sees -- no icon -- so the consequence must match.
        assert!(!may_outlive_its_window(&Outcome::Refused(
            "the watcher said no".to_string()
        )));
    }

    #[test]
    fn exactly_one_outcome_earns_the_hold() {
        // Asserting the premise rather than three examples of it: a fourth
        // outcome added later fails here rather than silently earning a hold.
        let outcomes = [
            Outcome::Registered,
            Outcome::NoWatcher,
            Outcome::Refused("any reason".to_string()),
        ];
        let holding = outcomes
            .iter()
            .filter(|outcome| may_outlive_its_window(outcome))
            .count();

        assert_eq!(holding, 1, "exactly one outcome may earn the hold");
    }

    #[test]
    fn a_running_daemon_with_nothing_waiting_shows_the_running_mark() {
        assert_eq!(glyph(StatusWord::Running, 0), TrayGlyph::Running);
    }

    #[test]
    fn nothing_read_yet_is_starting_rather_than_a_claim_that_it_is_down() {
        // The distinction that matters: we have not heard from the daemon, which
        // is not the same fact as the daemon being stopped, and drawing the
        // attention mark here would tell the operator something we do not know.
        assert_eq!(glyph(StatusWord::Unknown, 0), TrayGlyph::Starting);
    }

    #[test]
    fn a_stopped_daemon_asks_for_a_person() {
        assert_eq!(glyph(StatusWord::NotRunning, 0), TrayGlyph::Attention);
    }

    #[test]
    fn unfinished_setup_asks_for_a_person() {
        assert_eq!(glyph(StatusWord::SetupRequired, 0), TrayGlyph::Attention);
    }

    #[test]
    fn a_half_applied_update_asks_for_a_person() {
        assert_eq!(
            glyph(StatusWord::RestartToFinishUpdating, 0),
            TrayGlyph::Attention
        );
    }

    #[test]
    fn a_waiting_row_outranks_a_healthy_daemon() {
        // The case the rule exists for: everything about the service is fine and
        // there is still something for the operator to do.
        assert_eq!(glyph(StatusWord::Running, 1), TrayGlyph::Attention);
    }

    #[test]
    fn a_waiting_row_outranks_every_status_word() {
        // Asserting the premise rather than one example of it: if any word ever
        // escaped the attention rule, this fails rather than passing quietly.
        for status in [
            StatusWord::Running,
            StatusWord::Unknown,
            StatusWord::SetupRequired,
            StatusWord::RestartToFinishUpdating,
            StatusWord::NotRunning,
        ] {
            assert_eq!(
                glyph(status, 3),
                TrayGlyph::Attention,
                "{status:?} with rows waiting should show the attention mark"
            );
        }
    }

    #[test]
    fn every_glyph_names_an_icon() {
        // A glyph whose name is empty draws nothing and reports no error, which
        // is the silent-nothing failure this whole feature is prone to.
        for glyph in [
            TrayGlyph::Running,
            TrayGlyph::Starting,
            TrayGlyph::Attention,
        ] {
            assert!(
                !glyph.icon_name().is_empty(),
                "{glyph:?} must name an icon the theme can resolve"
            );
        }
    }
}
