//! Applying.
//!
//! Two rows at most: the write, and the restart the daemon says it needs. The
//! restart row is drawn only where a restart is actually owed or once one is
//! under way, because the daemon stops requiring a restart the moment it takes
//! one and a row that is running must not vanish out from under the person
//! watching it.
//!
//! A refusal stays on this screen with the sentence that refused it and the one
//! action that tries again. Nothing advances past a refused write.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy;
use crate::models::onboarding::{ApplyStep, OnboardingModel, Snapshot, Stage};
use crate::ui::widgets::checklist_row::ChecklistRow;
use crate::ui::CaptionRow;

use super::Screen;

/// The Applying screen.
pub struct ApplyingScreen {
    root: gtk::Widget,
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<(ApplyStep, ChecklistRow)>>,
    refusal: CaptionRow,
    blocked: CaptionRow,
    #[allow(dead_code)]
    model: Rc<OnboardingModel>,
}

impl ApplyingScreen {
    /// Build it over the assistant's model.
    pub fn new(model: &Rc<OnboardingModel>) -> Rc<Self> {
        let column = super::screen_column();
        let group = adw::PreferencesGroup::new();
        let refusal = CaptionRow::new();
        let blocked = CaptionRow::new();

        column.append(&group);
        column.append(&crate::ui::caption_group(&refusal));
        column.append(&crate::ui::caption_group(&blocked));

        Rc::new(Self {
            root: super::screen(&column),
            group,
            rows: RefCell::new(Vec::new()),
            refusal,
            blocked,
            model: Rc::clone(model),
        })
    }

    /// Draw the ladder the run actually takes. The rows are rebuilt only when
    /// the ladder's shape changes, which is when a restart joins it.
    fn draw_rows(&self, snapshot: &Snapshot) {
        let shape: Vec<ApplyStep> = snapshot.applying.iter().map(|(step, _)| *step).collect();
        let drawn: Vec<ApplyStep> = self.rows.borrow().iter().map(|(step, _)| *step).collect();

        if shape != drawn {
            for (_, row) in self.rows.borrow_mut().drain(..) {
                self.group.remove(row.row());
            }

            let mut built = Vec::with_capacity(shape.len());
            for (step, state) in &snapshot.applying {
                let row = ChecklistRow::new(&copy::text(step.key()));
                row.set(*state);
                self.group.add(row.row());
                built.push((*step, row));
            }
            self.rows.replace(built);
            return;
        }

        for (step, state) in &snapshot.applying {
            if let Some((_, row)) = self.rows.borrow().iter().find(|(named, _)| named == step) {
                if row.state() != *state {
                    row.set(*state);
                }
            }
        }
    }
}

impl Screen for ApplyingScreen {
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    fn draw(self: Rc<Self>, snapshot: &Snapshot) {
        self.draw_rows(snapshot);

        self.refusal.set(
            snapshot
                .refusal
                .as_ref()
                .map(|sentence| sentence.text.as_str()),
        );

        // A gap the assistant has no screen for keeps the person here with the
        // sentence that says so, rather than on a ladder that finished.
        self.blocked.set(
            snapshot
                .block
                .as_ref()
                .filter(|_| snapshot.stage == Stage::Applying)
                .map(|block| copy::text(block.key()))
                .as_deref(),
        );
    }
}
