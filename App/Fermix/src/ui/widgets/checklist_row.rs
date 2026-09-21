//! One row of a progress checklist.
//!
//! The two mechanical screens of the Setup assistant are a list of these and
//! nothing else: a row per step the transaction behind them actually takes,
//! with a prefix that is one of exactly four things — a static pending glyph,
//! a spinner, a tick or an error mark. There is no mascot, no halo and no
//! decorative animation on these screens, and the spinner runs only on a row
//! whose work is running.
//!
//! The state is never colour alone: the glyph is a shape, and the row carries
//! the state's own word as its accessible description.

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy;
use crate::metrics;
use crate::models::activation::StepState;
use crate::ui::plain;

/// A step nobody has started: the same dot the assistant's progress marks are
/// drawn with, dimmed. It is the application's own glyph because the platform
/// theme has no name for a neutral dot, and it draws on every desktop for the
/// same reason.
const WAITING_ICON: &str = "fermix-progress-symbolic";
/// A step that finished. The toolkit's own tick.
const DONE_ICON: &str = "object-select-symbolic";
/// A step that did not.
const FAILED_ICON: &str = "dialog-error-symbolic";

/// One row, and the prefix that says where it has got to.
pub struct ChecklistRow {
    row: adw::ActionRow,
    prefix: gtk::Stack,
    state: std::cell::Cell<StepState>,
}

impl ChecklistRow {
    /// A row for one step, waiting.
    pub fn new(title: &str) -> Self {
        let prefix = gtk::Stack::new();
        let waiting = glyph(WAITING_ICON);
        // Dimmed rather than a second glyph: a step that has not started is the
        // same mark as a step that has, with the toolkit's own distinction
        // between a thing in force and a thing that is not.
        waiting.add_css_class("dim-label");
        prefix.add_named(&waiting, Some(WAITING));
        prefix.add_named(&spinner(), Some(WORKING));
        prefix.add_named(&glyph(DONE_ICON), Some(DONE));
        prefix.add_named(&glyph(FAILED_ICON), Some(FAILED));
        prefix.set_valign(gtk::Align::Center);

        let row = plain(
            adw::ActionRow::builder()
                .title(title)
                .activatable(false)
                .build(),
        );
        row.add_prefix(&prefix);
        // The title is the step; rows wrap rather than clip when the text grows.
        row.set_title_lines(0);

        let built = Self {
            row,
            prefix,
            state: std::cell::Cell::new(StepState::Waiting),
        };
        built.set(StepState::Waiting);
        built
    }

    /// The row, to add to a group.
    pub fn row(&self) -> &adw::ActionRow {
        &self.row
    }

    /// Where this step has got to.
    pub fn state(&self) -> StepState {
        self.state.get()
    }

    /// Move it.
    pub fn set(&self, state: StepState) {
        self.state.set(state);
        self.prefix.set_visible_child_name(match state {
            StepState::Waiting => WAITING,
            StepState::Working => WORKING,
            StepState::Done => DONE,
            StepState::Failed => FAILED,
        });

        // The state is a word as well as a shape, so a screen reader hears
        // which row is running and which one stopped.
        // Through the widget rather than the row: the toolkit floor this crate
        // compiles against does not declare the row itself accessible, and
        // every row is a widget.
        self.row.upcast_ref::<gtk::Widget>().update_property(&[
            gtk::accessible::Property::Description(&copy::text(state.key())),
        ]);
    }
}

const WAITING: &str = "waiting";
const WORKING: &str = "working";
const DONE: &str = "done";
const FAILED: &str = "failed";

fn glyph(name: &str) -> gtk::Image {
    let image = gtk::Image::from_icon_name(name);
    image.set_icon_size(gtk::IconSize::Normal);
    image
}

/// The toolkit's own spinner, at the size the glyphs beside it draw at, so a
/// row does not change height when its work starts.
fn spinner() -> adw::Spinner {
    let spinner = adw::Spinner::new();
    spinner.set_size_request(metrics::PREFIX_GLYPH, metrics::PREFIX_GLYPH);
    spinner
}
