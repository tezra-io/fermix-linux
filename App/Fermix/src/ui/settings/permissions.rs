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
use crate::models::secret_store::StoreKind;
use crate::models::{spawn, SettingsModel};
use crate::ui::plain;
use crate::ui::CaptionRow;

/// The Permissions pane.
pub struct PermissionsPane {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    ledger: Rc<PermissionLedger>,
    /// Where secrets live, and the way home from the file store.
    store: adw::ActionRow,
    /// What the file store means, in the present tense, because by the time
    /// this row is read the choice has already been made.
    store_detail: CaptionRow,
    store_group: adw::PreferencesGroup,
    return_to_keyring: gtk::Button,
    group: adw::PreferencesGroup,
    rows: Vec<(Right, adw::ExpanderRow, gtk::Label)>,
    notice: CaptionRow,
    /// Where the background service row goes, which is Home rather than a
    /// second switch for one thing.
    open_home: std::cell::RefCell<Option<Box<dyn Fn()>>>,
}

impl PermissionsPane {
    /// Build the pane over the one ledger.
    pub fn new(settings: Rc<SettingsModel>, ledger: Rc<PermissionLedger>) -> Rc<Self> {
        let group = adw::PreferencesGroup::new();
        let notice = CaptionRow::new();

        let mut rows = Vec::new();
        for right in RIGHTS {
            let standing = crate::ui::value_label("");
            let row = plain(
                adw::ExpanderRow::builder()
                    .title(copy::text(right.title()))
                    .subtitle(copy::text(right.principal()))
                    .expanded(false)
                    .build(),
            );
            row.add_suffix(&standing);
            row.add_row(&fact(right.revocation(), Key::PermissionsColumnRevoke));
            row.add_row(&fact(right.artifact(), Key::PermissionsColumnArtifact));
            group.add(&row);
            rows.push((*right, row, standing));
        }
        group.add(notice.row());

        let platform = adw::PreferencesGroup::new();
        platform.add(
            &crate::ui::folded_statement_row(
                &copy::text(Key::PermissionsPlatformFactLead),
                &copy::text(Key::PermissionsPlatformFact),
            )
            .row,
        );

        // Where secrets live is a fact about this machine, like the platform
        // fact above it, and it carries the one action that changes it.
        let store = plain(
            adw::ActionRow::builder()
                .title(copy::text(Key::SecretStoreRowLabel))
                .subtitle_lines(0)
                .activatable(false)
                .build(),
        );
        let return_to_keyring = gtk::Button::builder()
            .label(copy::text(Key::ActionUseKeyringInstead))
            .valign(gtk::Align::Center)
            .build();
        store.add_suffix(&return_to_keyring);
        let store_detail = CaptionRow::new();
        let store_group = adw::PreferencesGroup::new();
        store_group.add(&store);
        store_group.add(store_detail.row());

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&group);
        column.append(&store_group);
        column.append(&platform);

        let pane = Rc::new(Self {
            root: crate::ui::scrolled(&crate::ui::clamp(&column)).upcast(),
            settings,
            ledger,
            store,
            store_detail,
            store_group,
            return_to_keyring,
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

        let pane = Rc::downgrade(self);
        self.settings.observe(move |change| {
            // The store travels on the setup snapshot, so redraw when one lands.
            if !matches!(change, crate::models::Change::Setup) {
                return;
            }
            if let Some(pane) = pane.upgrade() {
                pane.draw_store();
            }
        });

        let pane = Rc::downgrade(self);
        self.return_to_keyring.connect_clicked(move |_| {
            let Some(pane) = pane.upgrade() else {
                return;
            };
            pane.return_to_keyring();
        });
    }

    /// Move every file-stored secret back into the keyring.
    ///
    /// The owner types nothing: this application cannot read a value back out
    /// of the file store, so the engine moves what it already holds. The wait
    /// and the giving-up are the save dialog's, because it is the same unlock
    /// and the owner should not meet two accounts of one thing.
    fn return_to_keyring(self: &Rc<Self>) {
        let settings = Rc::clone(&self.settings);
        let anchor = self.root.clone();
        crate::ui::settings::dialogs::secret::migrate_to_keyring(settings, &anchor);
    }

    /// Where secrets live, and whether there is anywhere to go from here.
    ///
    /// An engine that published nothing draws no row at all. Saying "no store"
    /// on its behalf would be the same kind of untrue statement as the message
    /// this pane's neighbour replaced: not knowing is not the same as knowing
    /// there is nothing.
    fn draw_store(&self) {
        let Some(kind) = self.settings.secret_store_kind() else {
            self.store_group.set_visible(false);
            self.store.set_visible(false);
            self.store_detail.set(None);
            return;
        };

        self.store_group.set_visible(true);
        self.store.set_visible(true);
        self.store.set_subtitle(&copy::text(kind.label()));
        // Only the file store has a cost to state. The keyring is the default
        // and `none` has nothing stored to say anything about.
        let detail = match kind {
            StoreKind::File => Some(copy::text(Key::SecretStoreThisComputerDetail)),
            _ => None,
        };
        self.store_detail.set(detail.as_deref());
        self.return_to_keyring
            .set_visible(kind.offers_return_to_keyring());
    }

    fn draw(&self) {
        self.draw_store();
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
    plain(
        adw::ActionRow::builder()
            .title(copy::text(label))
            .subtitle(copy::text(value))
            .subtitle_lines(0)
            .activatable(false)
            .build(),
    )
}
