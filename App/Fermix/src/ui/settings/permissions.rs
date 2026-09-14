//! The Permissions pane.
//!
//! One row per right, each naming the identity that holds it, how a person
//! takes it back and where the durable artifact lives, and one row underneath
//! them all stating the platform fact. The table is M38 section 7.4's and this
//! pane renders it rather than restating it.
//!
//! Nothing prompts on render. The two rights the helper holds are read from the
//! probe on the way in and on the explicit Refresh, and the probe never
//! prompts; every other row is a statement rather than a control.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::ledger::{LedgerRow, PermissionLedger, Right, RIGHTS};
use crate::models::spawn;
use crate::ui::CaptionRow;

/// The Permissions pane.
pub struct PermissionsPane {
    root: gtk::Widget,
    ledger: Rc<PermissionLedger>,
    group: adw::PreferencesGroup,
    rows: Vec<(Right, adw::ExpanderRow, gtk::Label)>,
    notice: CaptionRow,
    /// Where the background service row goes, which is Home rather than a
    /// second switch for one thing.
    open_home: std::cell::RefCell<Option<Box<dyn Fn()>>>,
}

impl PermissionsPane {
    /// Build the pane over the one ledger.
    pub fn new(ledger: Rc<PermissionLedger>) -> Rc<Self> {
        let group = adw::PreferencesGroup::new();
        let notice = CaptionRow::new();

        let mut rows = Vec::new();
        for right in RIGHTS {
            let standing = crate::ui::value_label("");
            let row = adw::ExpanderRow::builder()
                .title(copy::text(right.title()))
                .subtitle(copy::text(right.principal()))
                .expanded(false)
                .build();
            row.add_suffix(&standing);
            row.add_row(&fact(right.revocation(), Key::PermissionsColumnRevoke));
            row.add_row(&fact(right.artifact(), Key::PermissionsColumnArtifact));
            group.add(&row);
            rows.push((*right, row, standing));
        }
        group.add(notice.row());

        let platform = adw::PreferencesGroup::new();
        platform.add(
            &adw::ActionRow::builder()
                .title(copy::text(Key::PermissionsPlatformFact))
                .title_lines(0)
                .activatable(false)
                .build(),
        );

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&group);
        column.append(&platform);

        let pane = Rc::new(Self {
            root: crate::ui::scrolled(&crate::ui::clamp(&column)).upcast(),
            ledger,
            group,
            rows,
            notice,
            open_home: std::cell::RefCell::new(None),
        });

        pane.connect();
        pane.draw();
        pane
    }

    /// The widget the pane stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// Where the background service row goes.
    pub fn on_open_home(self: &Rc<Self>, open: impl Fn() + 'static) {
        self.open_home.replace(Some(Box::new(open)));

        let button = gtk::Button::builder()
            .label(copy::text(Key::PageHome))
            .valign(gtk::Align::Center)
            .build();

        let pane = Rc::clone(self);
        button.connect_clicked(move |_| {
            if let Some(open) = pane.open_home.borrow().as_ref() {
                open();
            }
        });

        if let Some((_, row, _)) = self
            .rows
            .iter()
            .find(|(right, _, _)| *right == Right::BackgroundService)
        {
            row.add_suffix(&button);
        }
    }

    /// Read the probe. On the way in, and on nothing else.
    pub fn load(self: &Rc<Self>) {
        let ledger = Rc::clone(&self.ledger);
        spawn(async move {
            ledger.refresh().await;
        });
    }

    fn connect(self: &Rc<Self>) {
        let pane = Rc::downgrade(self);
        self.ledger.observe(move || {
            if let Some(pane) = pane.upgrade() {
                pane.draw();
            }
        });
    }

    fn draw(&self) {
        for LedgerRow { right, standing } in self.ledger.rows() {
            let Some((_, _, label)) = self.rows.iter().find(|(held, _, _)| *held == right) else {
                continue;
            };
            // A right with nothing to report says nothing: that is the honest
            // answer, and it is not the same as off.
            crate::ui::set_value(label, &standing.map(copy::text).unwrap_or_default());
        }

        self.notice.set(
            self.ledger
                .refusal()
                .map(|sentence| sentence.text)
                .as_deref(),
        );
    }

    /// The group, for the tests that walk it.
    pub fn group(&self) -> adw::PreferencesGroup {
        self.group.clone()
    }
}

/// One labelled fact under a right.
fn fact(value: Key, label: Key) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(copy::text(label))
        .subtitle(copy::text(value))
        .subtitle_lines(0)
        .activatable(false)
        .build()
}
