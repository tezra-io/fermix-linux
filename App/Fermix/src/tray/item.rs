//! The StatusNotifierItem, and the handshake that puts it on a panel.
//!
//! Everything here is effectful and decides nothing. What the icon shows comes
//! from `tray::state`, what the menu offers from `tray::menu`, and how the menu
//! is serialised from `tray::dbusmenu`; this module owns the bus objects, the
//! registration, and the watching.
//!
//! THE FACT THIS MODULE MUST NOT OVERSTATE. Registering with the watcher is not
//! the same as being drawn, and the two are easy to confuse because both end in
//! silence. A host may take our registration and then drop us for a menu it
//! could not read, and a desktop may have no tray at all. So `Registration`
//! reports only what it actually observed -- that a watcher existed and
//! accepted the call -- and nothing in this file claims the icon is visible.
//! The distinction is why `Outcome` has three values rather than two.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gio;
use gtk4::gio::prelude::{ActionGroupExt, ActionMapExt};
use gtk4::glib;
use gtk4::glib::prelude::*;
use gtk4::glib::variant::Variant;

use crate::tray::dbusmenu;
use crate::tray::menu::TrayRow;
use crate::tray::state::TrayGlyph;

/// Where the item and its menu live on our own connection.
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/com/canonical/dbusmenu";

/// The watcher we register with.
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";

/// What came of trying to put an icon on a panel.
///
/// Three values, not two, because "there is no tray on this desktop" and "the
/// tray refused us" are different facts that call for different behaviour: the
/// first is correct on Fedora's GNOME and must not keep the application alive,
/// and the second is a defect worth a journal line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A watcher was on the bus and accepted the registration. The icon may
    /// still not be drawn -- see the module comment -- so this says registered,
    /// not shown.
    Registered,
    /// No watcher is on the bus. This desktop has no tray, which is not a
    /// fault.
    NoWatcher,
    /// A watcher was there and the registration failed.
    Refused(String),
}

/// The interface the panel reads our item through.
///
/// Only the properties a host actually reads are published. `ItemIsMenu` is
/// false and `Menu` names the dbusmenu object, which together are what the
/// GNOME extension requires before it will draw anything at all.
const ITEM_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <signal name="NewIcon"/>
    <signal name="NewStatus"><arg name="status" type="s"/></signal>
  </interface>
</node>
"#;

/// The part of `com.canonical.dbusmenu` the hosts actually call.
///
/// `AboutToShow`, `AboutToShowGroup`, `GetProperty` and `EventGroup` are
/// deliberately absent: no host we target calls them, and a stub that answered
/// them agreeably would turn a host we have never tested into a silently empty
/// menu rather than a plain unknown-method error we could read.
const MENU_XML: &str = r#"
<node>
  <interface name="com.canonical.dbusmenu">
    <property name="Version" type="u" access="read"/>
    <method name="GetLayout">
      <arg type="i" name="parentId" direction="in"/>
      <arg type="i" name="recursionDepth" direction="in"/>
      <arg type="as" name="propertyNames" direction="in"/>
      <arg type="u" name="revision" direction="out"/>
      <arg type="(ia{sv}av)" name="layout" direction="out"/>
    </method>
    <method name="GetGroupProperties">
      <arg type="ai" name="ids" direction="in"/>
      <arg type="as" name="propertyNames" direction="in"/>
      <arg type="a(ia{sv})" name="properties" direction="out"/>
    </method>
    <method name="Event">
      <arg type="i" name="id" direction="in"/>
      <arg type="s" name="eventId" direction="in"/>
      <arg type="v" name="data" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
    <signal name="LayoutUpdated">
      <arg type="u" name="revision"/>
      <arg type="i" name="parent"/>
    </signal>
    <signal name="ItemsPropertiesUpdated">
      <arg type="a(ia{sv})" name="updatedProps"/>
      <arg type="a(ias)" name="removedProps"/>
    </signal>
  </interface>
</node>
"#;

/// What the item publishes right now.
///
/// Held in one place so the properties a host reads and the menu it fetches
/// cannot describe different moments.
pub struct TrayItem {
    glyph: RefCell<TrayGlyph>,
    rows: RefCell<Vec<TrayRow>>,
    revision: RefCell<u32>,
    connection: gio::DBusConnection,
    /// The well-known name we own, which is what we hand the watcher.
    bus_name: String,
}

impl TrayItem {
    /// The icon name the host should draw.
    pub fn icon_name(&self) -> &'static str {
        self.glyph.borrow().icon_name()
    }

    /// Replace what the item shows.
    ///
    /// Emits only when something changed: a host that is told the icon is new
    /// re-reads every property, and saying so on every poll turns a one-second
    /// status poll into one-second bus traffic forever.
    pub fn update(&self, glyph: TrayGlyph, rows: Vec<TrayRow>) -> Result<(), glib::Error> {
        let glyph_changed = *self.glyph.borrow() != glyph;
        let rows_changed = *self.rows.borrow() != rows;

        if glyph_changed {
            *self.glyph.borrow_mut() = glyph;
            self.emit(ITEM_PATH, "org.kde.StatusNotifierItem", "NewIcon", None)?;
        }

        if rows_changed {
            *self.rows.borrow_mut() = rows;
            let revision = {
                let mut revision = self.revision.borrow_mut();
                *revision = revision.wrapping_add(1);
                *revision
            };

            self.emit(
                MENU_PATH,
                "com.canonical.dbusmenu",
                "LayoutUpdated",
                Some(&(revision, dbusmenu::ROOT_ID).to_variant()),
            )?;
        }

        Ok(())
    }

    /// The name this item owns on the bus.
    pub fn bus_name(&self) -> &str {
        &self.bus_name
    }

    fn emit(
        &self,
        path: &str,
        interface: &str,
        signal: &str,
        body: Option<&Variant>,
    ) -> Result<(), glib::Error> {
        self.connection
            .emit_signal(None, path, interface, signal, body)
    }
}

/// Ask the watcher to take this item.
///
/// Returns what actually happened rather than a bare bool, because the caller
/// has to tell "no tray here" from "the tray refused us" -- one is Fedora
/// behaving correctly and the other is a defect.
pub fn register(
    connection: &gio::DBusConnection,
    bus_name: &str,
    cancellable: Option<&gio::Cancellable>,
) -> Outcome {
    let reply = connection.call_sync(
        Some(WATCHER_NAME),
        WATCHER_PATH,
        WATCHER_INTERFACE,
        "RegisterStatusNotifierItem",
        Some(&(bus_name,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2_000,
        cancellable,
    );

    match reply {
        Ok(_) => Outcome::Registered,
        Err(error) if is_no_such_name(&error) => Outcome::NoWatcher,
        Err(error) => Outcome::Refused(error.to_string()),
    }
}

/// Whether this error means nobody owns the watcher name.
///
/// Told apart from every other failure because it is the one that is not a
/// fault. Matched on the D-Bus error name rather than on the message, which is
/// translated and reworded between releases.
fn is_no_such_name(error: &glib::Error) -> bool {
    gio::DBusError::remote_error(error)
        .map(|name| {
            name == "org.freedesktop.DBus.Error.ServiceUnknown"
                || name == "org.freedesktop.DBus.Error.NameHasNoOwner"
        })
        .unwrap_or(false)
}

/// Watch for the tray arriving or going away, and act on each.
///
/// A panel extension can be enabled, disabled or restarted long after we start,
/// so the registration cannot be a one-shot at launch. This is name-watching
/// rather than a poll: GIO delivers the two edges and nothing runs in between.
/// `on_appeared` is expected to re-register; `on_vanished` is how the caller
/// learns the icon is gone and, with it, the only way back to a windowless app.
/// The watch lasts for the life of the process and is deliberately not
/// cancellable. Two reasons, and the second is the honest one: the tray is
/// wanted for as long as the application runs, and `bus_watch_name` in this
/// binding version returns a `WatcherId` from a module whose name is shadowed
/// by another export, so the type cannot be written down here to hand back. If
/// a caller ever needs to stop watching, that wants a binding fix rather than a
/// cast.
pub fn watch_for_host<A, V>(on_appeared: A, on_vanished: V)
where
    A: Fn() + 'static,
    V: Fn() + 'static,
{
    gio::bus_watch_name(
        gio::BusType::Session,
        WATCHER_NAME,
        gio::BusNameWatcherFlags::NONE,
        move |_, _, _| on_appeared(),
        move |_, _| on_vanished(),
    );
}

/// Put the item and its menu on the connection.
///
/// Both objects are registered before the watcher is told about us. A host that
/// is told first and reads second can find nothing there, and what it does
/// about that is to drop the item without saying so.
pub fn publish(
    connection: &gio::DBusConnection,
    bus_name: &str,
    glyph: TrayGlyph,
    rows: Vec<TrayRow>,
) -> Result<Rc<TrayItem>, glib::Error> {
    let item = Rc::new(TrayItem {
        glyph: RefCell::new(glyph),
        rows: RefCell::new(rows),
        revision: RefCell::new(1),
        connection: connection.clone(),
        bus_name: bus_name.to_string(),
    });

    register_item_object(connection, &item)?;
    register_menu_object(connection, &item)?;

    Ok(item)
}

/// `MENU_PATH` as the object path the `Menu` property must be.
///
/// Typed as an object path rather than as a string: the host reads this
/// property as `o`, and a plain `s` there is a type mismatch it reports as a
/// missing menu -- which is the one failure that makes the icon vanish without
/// a word.
fn object_path(path: &str) -> Variant {
    glib::variant::ObjectPath::try_from(path)
        .expect("the menu path is a constant and a valid object path")
        .to_variant()
}

fn interface(xml: &str, name: &str) -> Result<gio::DBusInterfaceInfo, glib::Error> {
    let node = gio::DBusNodeInfo::for_xml(xml)?;

    node.lookup_interface(name).ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::Failed,
            &format!("{name} is not in the interface description"),
        )
    })
}

fn register_item_object(
    connection: &gio::DBusConnection,
    item: &Rc<TrayItem>,
) -> Result<(), glib::Error> {
    let info = interface(ITEM_XML, "org.kde.StatusNotifierItem")?;
    let item = Rc::clone(item);

    connection
        .register_object(ITEM_PATH, &info)
        .property(move |_, _, _, _, name| match name {
            "Category" => "ApplicationStatus".to_variant(),
            "Id" => "io.tezra.Fermix".to_variant(),
            "Title" => "Fermix".to_variant(),
            // Always Active. `NeedsAttention` makes some panels pull the icon
            // out of its usual place, which moves a target the user aims at.
            // The mark says what needs attention; the position should not.
            "Status" => "Active".to_variant(),
            "IconName" => item.icon_name().to_variant(),
            "ItemIsMenu" => false.to_variant(),
            "Menu" => object_path(MENU_PATH),
            _ => "".to_variant(),
        })
        .build()
        .map(|_| ())
}

fn register_menu_object(
    connection: &gio::DBusConnection,
    item: &Rc<TrayItem>,
) -> Result<(), glib::Error> {
    let info = interface(MENU_XML, "com.canonical.dbusmenu")?;
    let for_property = Rc::clone(item);
    let for_call = Rc::clone(item);

    connection
        .register_object(MENU_PATH, &info)
        .property(move |_, _, _, _, name| match name {
            "Version" => 3u32.to_variant(),
            _ => {
                let _ = &for_property;
                "".to_variant()
            }
        })
        .method_call(move |_, _, _, _, method, parameters, invocation| {
            handle_menu_call(&for_call, method, &parameters, invocation);
        })
        .build()
        .map(|_| ())
}

/// Answer one dbusmenu call.
///
/// An unknown method is answered with an error rather than with an empty
/// success. A host calling something we did not implement should see that it
/// did, because the alternative is a menu that is empty for no stated reason.
fn handle_menu_call(
    item: &Rc<TrayItem>,
    method: &str,
    parameters: &Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "GetLayout" => {
            let rows = item.rows.borrow();
            let revision = *item.revision.borrow();
            invocation.return_value(Some(&dbusmenu::layout(revision, &rows)));
        }
        "GetGroupProperties" => {
            let rows = item.rows.borrow();
            invocation.return_value(Some(&dbusmenu::group_properties(&rows)));
        }
        "Event" => {
            let id = parameters.child_value(0).get::<i32>().unwrap_or(-1);
            let event = parameters
                .child_value(1)
                .str()
                .unwrap_or_default()
                .to_string();
            let rows = item.rows.borrow().clone();

            // Only "clicked" acts. "hovered" and "opened" arrive too, and a
            // command that ran on hover would fire under a passing pointer.
            if event == "clicked" {
                if let Some(command) = dbusmenu::command_for(&rows, id) {
                    activate(command);
                }
            }

            invocation.return_value(None);
        }
        other => {
            invocation.return_error(
                gio::IOErrorEnum::NotSupported,
                &format!("com.canonical.dbusmenu.{other} is not implemented"),
            );
        }
    }
}

/// Activate the application action a row names.
///
/// The row names an `app.` action and this looks it up on the default
/// application, so the tray cannot reach anything the menus cannot. A missing
/// action is logged rather than ignored: it means the table and the actions
/// have drifted, which is a defect and not a user error.
fn activate(command: crate::tray::menu::TrayCommand) {
    let Some(application) = gio::Application::default() else {
        glib::g_warning!("fermix", "a tray row fired with no application to act on");
        return;
    };

    let action = command.action();
    let Some(name) = action.strip_prefix("app.") else {
        glib::g_warning!("fermix", "{action} is not an application action");
        return;
    };

    if application.lookup_action(name).is_none() {
        glib::g_warning!("fermix", "the tray names {action}, which does not exist");
        return;
    }

    application.activate_action(name, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_interface_descriptions_parse_and_carry_their_interface() {
        // Malformed XML here fails at registration, at runtime, on a desktop --
        // far from the edit that broke it.
        assert!(interface(ITEM_XML, "org.kde.StatusNotifierItem").is_ok());
        assert!(interface(MENU_XML, "com.canonical.dbusmenu").is_ok());
    }

    #[test]
    fn an_interface_the_description_does_not_carry_is_refused() {
        // The absent-name control: proves the lookup above is looking.
        assert!(interface(ITEM_XML, "org.kde.NoSuchInterface").is_err());
        assert!(interface(MENU_XML, "com.canonical.NoSuchThing").is_err());
    }

    #[test]
    fn the_item_publishes_the_two_properties_the_gnome_host_demands() {
        // Measured requirement, not a guess: the AppIndicator extension drops
        // an item that has no Menu property, and never says why.
        let info = interface(ITEM_XML, "org.kde.StatusNotifierItem").expect("the item interface");

        assert!(info.lookup_property("Menu").is_some(), "no Menu property");
        assert!(info.lookup_property("ItemIsMenu").is_some());
        assert!(info.lookup_property("IconName").is_some());
        // The control: the lookup above is looking, not agreeing.
        assert!(info.lookup_property("NoSuchProperty").is_none());
    }

    #[test]
    fn the_menu_publishes_the_three_methods_the_hosts_call() {
        let info = interface(MENU_XML, "com.canonical.dbusmenu").expect("the menu interface");

        for method in ["GetLayout", "GetGroupProperties", "Event"] {
            assert!(info.lookup_method(method).is_some(), "{method} is missing");
        }
    }

    #[test]
    fn the_menu_publishes_nothing_it_does_not_implement() {
        // Advertising a method we answer with an error is worse than not
        // advertising it: a host picks it precisely because we said we had it.
        let info = interface(MENU_XML, "com.canonical.dbusmenu").expect("the menu interface");

        for method in [
            "AboutToShow",
            "AboutToShowGroup",
            "GetProperty",
            "EventGroup",
        ] {
            assert!(
                info.lookup_method(method).is_none(),
                "{method} is advertised but not implemented"
            );
        }
    }

    #[test]
    fn the_menu_property_is_an_object_path_and_not_a_string() {
        // A string here reads to the host as a missing menu, and the icon then
        // never appears with nothing said anywhere.
        assert_eq!(object_path(MENU_PATH).type_().as_str(), "o");
    }

    #[test]
    fn the_outcomes_are_distinct() {
        // No tray and a refusing tray must never compare equal: one keeps the
        // application alive and the other must not.
        assert_ne!(Outcome::NoWatcher, Outcome::Registered);
        assert_ne!(Outcome::NoWatcher, Outcome::Refused("anything".to_string()));
    }
}
