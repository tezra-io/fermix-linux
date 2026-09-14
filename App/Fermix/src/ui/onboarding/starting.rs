//! Starting.
//!
//! The five rows of the activation transaction and nothing else: no mascot, no
//! orb, no halo and no decorative animation (M38 section 5.5). A row moves only
//! on evidence the command line or the daemon published, so the ladder never
//! invents progress between observations.
//!
//! The only way off this screen is the bar's Cancel, which stops the
//! transaction. There is no decision on it to take.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy;
use crate::models::activation::Step;
use crate::models::onboarding::Snapshot;
use crate::ui::widgets::checklist_row::ChecklistRow;

use super::Screen;

/// The Starting screen.
pub struct StartingScreen {
    root: gtk::Widget,
    rows: Vec<(Step, ChecklistRow)>,
}

impl StartingScreen {
    /// Build it. The rows are the transaction's own steps, in its order.
    pub fn new() -> Rc<Self> {
        let column = super::screen_column();
        let group = adw::PreferencesGroup::new();

        let rows: Vec<(Step, ChecklistRow)> = Step::ALL
            .iter()
            .map(|step| (*step, ChecklistRow::new(&copy::text(step.key()))))
            .collect();

        for (_, row) in &rows {
            group.add(row.row());
        }
        column.append(&group);

        Rc::new(Self {
            root: super::screen(&column),
            rows,
        })
    }
}

impl Screen for StartingScreen {
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    fn draw(self: Rc<Self>, snapshot: &Snapshot) {
        for (step, row) in &self.rows {
            let Some((_, state)) = snapshot.steps.iter().find(|(named, _)| named == step) else {
                continue;
            };
            if row.state() != *state {
                row.set(*state);
            }
        }
    }
}
