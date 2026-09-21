//! Putting the icon on a panel, keeping it current, and deciding whether the
//! application may outlive its window.
//!
//! This is the one effectful place that joins the parts: it asks
//! `tray::state` what to draw, `tray::menu` what to offer, `tray::item` to
//! publish them, and the application to hold itself open only when an icon
//! actually reached a panel.
//!
//! WHAT IT DOES NOT DO. It never claims the icon is visible. A host can accept
//! a registration and still not draw us -- see `tray::item` -- so the strongest
//! thing said anywhere here is that a watcher took the item. The hold follows
//! that, because it is the best fact available, and the journal line says
//! registered rather than shown so nobody reads it as more than it is.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;

use crate::app::FermixApplication;
use crate::models::home::{self, StatusWord};
use crate::models::SettingsModel;
use crate::tray::item::{self, Outcome, TrayItem};
use crate::tray::state;
use crate::tray::{menu, state::TrayGlyph};

/// The bus name the item owns, which is what the watcher is handed.
///
/// The process id is in it because the specification says a name of this shape
/// is what a watcher expects, and because two Fermix processes on one bus must
/// not collide -- a second instance normally raises the first window rather
/// than existing, but a stale name from a crashed run would otherwise be taken
/// for ours.
fn bus_name() -> String {
    format!(
        "org.freedesktop.StatusNotifierItem-{}-1",
        std::process::id()
    )
}

/// The tray, for as long as the application runs.
pub struct TrayController {
    application: FermixApplication,
    settings: Rc<SettingsModel>,
    connection: gio::DBusConnection,
    item: Rc<TrayItem>,
    /// What was last drawn, so that a poll which changes nothing emits nothing.
    shown: RefCell<Option<(TrayGlyph, StatusWord)>>,
}

impl TrayController {
    /// Publish the item, try once to register it, and watch for the tray
    /// arriving or leaving.
    ///
    /// Returns the controller even when there is no tray: the watch is what
    /// makes a panel enabled later still get an icon, and without the
    /// controller alive there would be nothing to register when it appears.
    pub fn start(
        application: &FermixApplication,
        settings: Rc<SettingsModel>,
    ) -> Result<Rc<Self>, glib::Error> {
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        let name = bus_name();

        // Own the name before the watcher is told about it. A watcher handed a
        // name nobody owns looks us up, finds nothing, and drops the item.
        gio::bus_own_name_on_connection(
            &connection,
            &name,
            gio::BusNameOwnerFlags::NONE,
            |_, _| {},
            |_, _| {},
        );

        let (glyph, status) = Self::read(&settings);
        let item = item::publish(&connection, &name, glyph, menu::rows(status))?;

        let controller = Rc::new(Self {
            application: application.clone(),
            settings,
            connection,
            item,
            shown: RefCell::new(Some((glyph, status))),
        });

        controller.try_to_register();
        controller.watch_for_the_tray();

        Ok(controller)
    }

    /// What the icon and the menu should say right now.
    ///
    /// One read of the shared model, so the glyph and the state line describe
    /// the same moment. Home's own snapshot is the source; nothing here asks
    /// the daemon anything.
    fn read(settings: &Rc<SettingsModel>) -> (TrayGlyph, StatusWord) {
        // The same two calls Home's own snapshot makes, against the same
        // borrowed state, so the icon and Home cannot disagree. `build::alignment`
        // is what Home passes, and passing anything else here would make the
        // tray a second opinion rather than a second view.
        let state = settings.state();
        let status = home::status_word(&state);
        let attention = home::attention(&state, crate::session::build::alignment()).len();

        (state::glyph(status, attention), status)
    }

    /// Redraw if anything changed. Call it whenever the model moves.
    pub fn refresh(&self) {
        let current = Self::read(&self.settings);
        if self.shown.borrow().as_ref() == Some(&current) {
            return;
        }

        let (glyph, status) = current;
        if let Err(error) = self.item.update(glyph, menu::rows(status)) {
            // Not swallowed: a tray that stops tracking the daemon is a tray
            // that lies, and the journal is the only place that can say so.
            glib::g_warning!(
                crate::app::TEXT_DOMAIN,
                "the tray could not publish its new state: {error}"
            );
            return;
        }

        self.shown.replace(Some(current));
    }

    /// Ask the watcher to take the item, and hold the application open only if
    /// it did.
    fn try_to_register(&self) {
        let outcome = item::register(
            &self.connection,
            self.item.bus_name(),
            gio::Cancellable::NONE,
        );

        if state::may_outlive_its_window(&outcome) {
            self.application.hold_for_tray();
        }

        match &outcome {
            // Registered, not shown. The host may still drop us, and nothing
            // here can see that.
            Outcome::Registered => glib::g_debug!(
                crate::app::TEXT_DOMAIN,
                "the tray accepted the status item; closing the window will not quit"
            ),
            // Not a fault. Fedora's GNOME ships no tray, and saying this once
            // is the difference between a known absence and a silent one.
            Outcome::NoWatcher => glib::g_message!(
                crate::app::TEXT_DOMAIN,
                "this desktop has no status area, so Fermix shows no icon and closing the window quits"
            ),
            Outcome::Refused(reason) => glib::g_warning!(
                crate::app::TEXT_DOMAIN,
                "the status area refused the icon, so closing the window quits: {reason}"
            ),
        }
    }

    /// Re-register when a panel appears, and notice when it goes.
    ///
    /// Name-watching rather than polling: GIO delivers the two edges and
    /// nothing runs in between. A panel extension can be enabled, disabled or
    /// restarted at any time, so a one-shot registration at startup would mean
    /// the icon never comes back after a shell restart.
    fn watch_for_the_tray(self: &Rc<Self>) {
        let appeared = Rc::downgrade(self);
        let vanished = Rc::downgrade(self);

        item::watch_for_host(
            move || {
                if let Some(controller) = appeared.upgrade() {
                    controller.try_to_register();
                }
            },
            move || {
                if let Some(controller) = vanished.upgrade() {
                    // The icon is gone, so the only way back to a windowless
                    // application is gone with it. The hold must go too, or the
                    // user is left with a process they cannot reach -- the same
                    // rule as a desktop that never had a tray, arrived at from
                    // the other direction.
                    controller.application.release_tray_hold_for_lost_tray();
                    glib::g_message!(
                        crate::app::TEXT_DOMAIN,
                        "the status area went away, so closing the window will quit again"
                    );
                }
            },
        );
    }
}
