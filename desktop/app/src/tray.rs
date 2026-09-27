//! The tray icon on the session bus: a StatusNotifierItem, and the
//! com.canonical.dbusmenu menu it opens, drawn from core's `TrayView`. Nothing
//! here decides what the tray says. A click comes back to the controller as a
//! `TrayEvent`, on the next turn of the main loop so it never runs inside a
//! D-Bus reply. The item registers under the connection's unique name, which a
//! sandbox may use without owning a well-known name.

use fermix_client::tray::{Command, Item, Row, TrayView};
use gtk::gio::{self, prelude::*};
use gtk::glib::{self, Variant, VariantDict, VariantTy};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";
/// The tray host's name, path and interface.
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
/// How long the tray host may take to accept the icon.
const REGISTER_TIMEOUT_MS: i32 = 5_000;
/// The menu protocol version this menu speaks.
const MENU_VERSION: u32 = 3;

const ITEM_XML: &str = r#"<node><interface name="org.kde.StatusNotifierItem">
  <property name="Category" type="s" access="read"/>
  <property name="Id" type="s" access="read"/>
  <property name="Title" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="WindowId" type="i" access="read"/>
  <property name="IconName" type="s" access="read"/>
  <property name="IconThemePath" type="s" access="read"/>
  <property name="IconPixmap" type="a(iiay)" access="read"/>
  <property name="OverlayIconName" type="s" access="read"/>
  <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
  <property name="AttentionIconName" type="s" access="read"/>
  <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
  <property name="AttentionMovieName" type="s" access="read"/>
  <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
  <property name="ItemIsMenu" type="b" access="read"/>
  <property name="Menu" type="o" access="read"/>
  <method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
  <signal name="NewIcon"/>
  <signal name="NewToolTip"/>
</interface></node>"#;

const MENU_XML: &str = r#"<node><interface name="com.canonical.dbusmenu">
  <property name="Version" type="u" access="read"/>
  <property name="TextDirection" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="IconThemePath" type="as" access="read"/>
  <method name="GetLayout">
    <arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/>
    <arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/>
  </method>
  <method name="GetGroupProperties">
    <arg type="ai" direction="in"/><arg type="as" direction="in"/>
    <arg type="a(ia{sv})" direction="out"/>
  </method>
  <method name="GetProperty">
    <arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/>
  </method>
  <method name="Event">
    <arg type="i" direction="in"/><arg type="s" direction="in"/>
    <arg type="v" direction="in"/><arg type="u" direction="in"/>
  </method>
  <method name="EventGroup">
    <arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/>
  </method>
  <method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
  <method name="AboutToShowGroup">
    <arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/>
  </method>
  <signal name="ItemsPropertiesUpdated">
    <arg type="a(ia{sv})"/><arg type="a(ias)"/>
  </signal>
  <signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
  <signal name="ItemActivationRequested"><arg type="i"/><arg type="u"/></signal>
</interface></node>"#;

/// What the tray hands back to the controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayEvent {
    /// The icon itself was activated rather than its menu opened.
    Activate,
    /// A menu row was chosen.
    Command(Command),
    /// The menu is about to open, so its status line should be read again.
    Opening,
    /// Whether a tray host shows the icon now.
    Hosted(bool),
}

/// What the bus callbacks share with the tray: the view they draw, the menu's
/// revision, whether a host took the icon, and where events go.
#[derive(Clone)]
struct Shared {
    view: Rc<RefCell<TrayView>>,
    revision: Rc<Cell<u32>>,
    hosted: Rc<Cell<bool>>,
    events: Rc<dyn Fn(TrayEvent)>,
}

pub struct Tray {
    bus: gio::DBusConnection,
    shared: Shared,
    registrations: Vec<gio::RegistrationId>,
    /// Stops following the tray host. gio names the watch's id only through a
    /// re-export its connection watcher shadows, so the id stays in here.
    unwatch: Option<Box<dyn FnOnce()>>,
}

impl Tray {
    /// Puts the item and its menu on the session bus, then follows the tray
    /// host: registered each time one appears, unhosted when it goes.
    pub async fn connect(
        view: TrayView,
        events: impl Fn(TrayEvent) + 'static,
    ) -> Result<Tray, glib::Error> {
        let bus = gio::bus_get_future(gio::BusType::Session).await?;
        let shared = Shared {
            view: Rc::new(RefCell::new(view)),
            revision: Rc::new(Cell::new(1)),
            hosted: Rc::new(Cell::new(false)),
            events: Rc::new(events),
        };
        let item = register_item(&bus, &shared)?;
        let menu = match register_menu(&bus, &shared) {
            Ok(menu) => menu,
            Err(e) => {
                unregister(&bus, item);
                return Err(e);
            }
        };
        let watcher = follow_host(&bus, &shared);
        Ok(Tray {
            bus,
            shared,
            registrations: vec![item, menu],
            unwatch: Some(watcher),
        })
    }

    pub fn hosted(&self) -> bool {
        self.shared.hosted.get()
    }

    /// Draws a new view: the icon and tooltip when they change, the menu when
    /// its rows do.
    pub fn show(&self, view: TrayView) {
        let old = self.shared.view.replace(view);
        let new = self.shared.view.borrow();
        if old.glyph != new.glyph {
            self.emit(ITEM_PATH, ITEM_IFACE, "NewIcon", None);
        }
        if old.status != new.status {
            self.emit(ITEM_PATH, ITEM_IFACE, "NewToolTip", None);
        }
        if old.rows != new.rows {
            let revision = self.shared.revision.get() + 1;
            self.shared.revision.set(revision);
            let changed = (revision, 0i32).to_variant();
            self.emit(MENU_PATH, MENU_IFACE, "LayoutUpdated", Some(&changed));
        }
    }

    fn emit(&self, path: &str, iface: &str, signal: &str, params: Option<&Variant>) {
        if let Err(e) = self.bus.emit_signal(None, path, iface, signal, params) {
            glib::g_warning!("fermix", "the tray could not send {signal}: {e}");
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if let Some(unwatch) = self.unwatch.take() {
            unwatch();
        }
        for id in self.registrations.drain(..) {
            unregister(&self.bus, id);
        }
    }
}

fn unregister(bus: &gio::DBusConnection, id: gio::RegistrationId) {
    if let Err(e) = bus.unregister_object(id) {
        glib::g_warning!("fermix", "a tray object stayed on the bus: {e}");
    }
}

fn interface(xml: &str, name: &str) -> Result<gio::DBusInterfaceInfo, glib::Error> {
    gio::DBusNodeInfo::for_xml(xml)?
        .lookup_interface(name)
        .ok_or_else(|| glib::Error::new(gio::IOErrorEnum::NotFound, "no such interface"))
}

/// Hands an event to the controller once the current D-Bus call has returned.
fn send(shared: &Shared, event: TrayEvent) {
    let events = shared.events.clone();
    glib::idle_add_local_once(move || events(event));
}

fn follow_host(bus: &gio::DBusConnection, shared: &Shared) -> Box<dyn FnOnce()> {
    let (appeared, vanished) = (shared.clone(), shared.clone());
    let id = gio::bus_watch_name_on_connection(
        bus,
        WATCHER,
        gio::BusNameWatcherFlags::NONE,
        move |bus, _, _| {
            let shared = appeared.clone();
            glib::spawn_future_local(async move { register_with_host(bus, shared).await });
        },
        move |_, _| set_hosted(&vanished, false),
    );
    Box::new(move || gio::bus_unwatch_name(id))
}

async fn register_with_host(bus: gio::DBusConnection, shared: Shared) {
    let answer = bus
        .call_future(
            Some(WATCHER),
            WATCHER_PATH,
            WATCHER,
            "RegisterStatusNotifierItem",
            Some(&(ITEM_PATH,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            REGISTER_TIMEOUT_MS,
        )
        .await;
    if let Err(e) = &answer {
        glib::g_warning!("fermix", "the tray host did not take the icon: {e}");
    }
    set_hosted(&shared, answer.is_ok());
}

fn set_hosted(shared: &Shared, hosted: bool) {
    if shared.hosted.replace(hosted) != hosted {
        send(shared, TrayEvent::Hosted(hosted));
    }
}

// ---- The item -------------------------------------------------------------

fn register_item(
    bus: &gio::DBusConnection,
    shared: &Shared,
) -> Result<gio::RegistrationId, glib::Error> {
    let menu = Variant::parse(Some(VariantTy::OBJECT_PATH), &format!("'{MENU_PATH}'"))?;
    let (calls, reads) = (shared.clone(), shared.view.clone());
    bus.register_object(ITEM_PATH, &interface(ITEM_XML, ITEM_IFACE)?)
        .method_call(move |_, _, _, _, method, _, invocation| {
            if method == "Activate" || method == "SecondaryActivate" {
                send(&calls, TrayEvent::Activate);
            }
            invocation.return_value(None);
        })
        .property(move |_, _, _, _, name| item_property(&reads.borrow(), name, &menu))
        .build()
}

/// The menu opens on any click, as the macOS status item's does; hosts that
/// also send Activate get the window.
fn item_property(view: &TrayView, name: &str, menu: &Variant) -> Variant {
    match name {
        "Category" => "ApplicationStatus".to_variant(),
        "Id" => "io.tezra.Fermix".to_variant(),
        "Title" => "Fermix".to_variant(),
        "Status" => "Active".to_variant(),
        "WindowId" => 0i32.to_variant(),
        "IconName" => view.glyph.icon_name().to_variant(),
        "ToolTip" => tooltip(view),
        "ItemIsMenu" => true.to_variant(),
        "Menu" => menu.clone(),
        "IconPixmap" | "OverlayIconPixmap" | "AttentionIconPixmap" => no_pixmaps(),
        // IconThemePath, OverlayIconName, AttentionIconName, AttentionMovieName.
        _ => "".to_variant(),
    }
}

/// The glyph's own words first, so a screen reader says what the icon shows.
fn tooltip(view: &TrayView) -> Variant {
    Variant::tuple_from_iter([
        "".to_variant(),
        no_pixmaps(),
        "Fermix".to_variant(),
        format!("{}. {}", view.glyph.label(), view.status).to_variant(),
    ])
}

fn no_pixmaps() -> Variant {
    let pixmap = VariantTy::new("(iiay)").expect("(iiay) is a valid type");
    Variant::array_from_iter_with_type(pixmap, std::iter::empty::<Variant>())
}

// ---- The menu -------------------------------------------------------------

fn register_menu(
    bus: &gio::DBusConnection,
    shared: &Shared,
) -> Result<gio::RegistrationId, glib::Error> {
    let calls = shared.clone();
    bus.register_object(MENU_PATH, &interface(MENU_XML, MENU_IFACE)?)
        .method_call(move |_, _, _, _, method, params, invocation| {
            match menu_call(&calls, method, &params) {
                Ok(reply) => invocation.return_value(reply.as_ref()),
                Err(message) => {
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", &message)
                }
            }
        })
        .property(|_, _, _, _, name| menu_property(name))
        .build()
}

fn menu_property(name: &str) -> Variant {
    match name {
        "Version" => MENU_VERSION.to_variant(),
        "TextDirection" => "ltr".to_variant(),
        "Status" => "normal".to_variant(),
        _ => Vec::<String>::new().to_variant(),
    }
}

fn menu_call(shared: &Shared, method: &str, params: &Variant) -> Result<Option<Variant>, String> {
    let view = shared.view.borrow();
    match method {
        "GetLayout" => Ok(Some(Variant::tuple_from_iter([
            shared.revision.get().to_variant(),
            layout(&view),
        ]))),
        "GetGroupProperties" => {
            let (ids, _) = params
                .get::<(Vec<i32>, Vec<String>)>()
                .ok_or("bad arguments")?;
            Ok(Some(Variant::tuple_from_iter([group_properties(
                &view, &ids,
            )])))
        }
        "GetProperty" => {
            let (id, name) = params.get::<(i32, String)>().ok_or("bad arguments")?;
            let value = properties(&view, id)
                .and_then(|props| VariantDict::new(Some(&props)).lookup_value(&name, None))
                .ok_or_else(|| format!("item {id} has no {name}"))?;
            Ok(Some((value,).to_variant()))
        }
        "Event" => {
            let (id, kind, _, _) = params
                .get::<(i32, String, Variant, u32)>()
                .ok_or("bad arguments")?;
            clicked(shared, &view, id, &kind);
            Ok(None)
        }
        "EventGroup" => {
            let events = params
                .get::<(Vec<(i32, String, Variant, u32)>,)>()
                .ok_or("bad arguments")?;
            let missing: Vec<i32> = events
                .0
                .iter()
                .filter(|(id, kind, _, _)| !clicked(shared, &view, *id, kind))
                .map(|e| e.0)
                .collect();
            Ok(Some((missing,).to_variant()))
        }
        "AboutToShow" => {
            send(shared, TrayEvent::Opening);
            Ok(Some((false,).to_variant()))
        }
        // AboutToShowGroup: nothing needs updating, and no id is unknown.
        _ => Ok(Some((Vec::<i32>::new(), Vec::<i32>::new()).to_variant())),
    }
}

/// Acts on a click; `false` when the id names no row.
fn clicked(shared: &Shared, view: &TrayView, id: i32, kind: &str) -> bool {
    let Some(row) = row_at(view, id) else {
        return id == 0;
    };
    if let (Row::Item(item), "clicked") = (row, kind) {
        if item.enabled {
            send(shared, TrayEvent::Command(item.command));
        }
    }
    true
}

/// Row ids are their place in the menu, counted from 1; 0 is the root.
fn row_at(view: &TrayView, id: i32) -> Option<&Row> {
    let index = usize::try_from(id).ok()?.checked_sub(1)?;
    view.rows.get(index)
}

fn layout(view: &TrayView) -> Variant {
    let children = view.rows.iter().enumerate().map(|(index, row)| {
        let id = i32::try_from(index + 1).expect("a menu has fewer rows than i32::MAX");
        Variant::from_variant(&node(id, row_properties(row), Vec::new()))
    });
    node(0, root_properties(), children.collect())
}

fn node(id: i32, properties: Variant, children: Vec<Variant>) -> Variant {
    Variant::tuple_from_iter([
        id.to_variant(),
        properties,
        Variant::array_from_iter_with_type(VariantTy::VARIANT, children),
    ])
}

/// `a(ia{sv})`: every row's properties, or the asked rows'. Names are not
/// filtered, which the protocol allows.
fn group_properties(view: &TrayView, ids: &[i32]) -> Variant {
    let all: Vec<i32> = (0..=view.rows.len())
        .filter_map(|i| i32::try_from(i).ok())
        .collect();
    let wanted = if ids.is_empty() { &all[..] } else { ids };
    let entries = wanted.iter().filter_map(|&id| {
        Some(Variant::tuple_from_iter([
            id.to_variant(),
            properties(view, id)?,
        ]))
    });
    let entry = VariantTy::new("(ia{sv})").expect("(ia{sv}) is a valid type");
    Variant::array_from_iter_with_type(entry, entries)
}

fn properties(view: &TrayView, id: i32) -> Option<Variant> {
    if id == 0 {
        return Some(root_properties());
    }
    row_at(view, id).map(row_properties)
}

fn root_properties() -> Variant {
    let props = VariantDict::new(None);
    props.insert("children-display", "submenu");
    props.end()
}

fn row_properties(row: &Row) -> Variant {
    let props = VariantDict::new(None);
    match row {
        Row::Status(text) => {
            props.insert("label", literal(text));
            props.insert("enabled", false);
        }
        Row::Separator => props.insert("type", "separator"),
        Row::Item(item) => item_properties(&props, item),
    }
    props.end()
}

fn item_properties(props: &VariantDict, item: &Item) {
    props.insert("label", literal(item.label));
    props.insert("enabled", item.enabled);
    if let Some(on) = item.checked {
        props.insert("toggle-type", "checkmark");
        props.insert("toggle-state", i32::from(on));
    }
}

/// A menu label marks its access key with `_`, so a literal one is doubled.
fn literal(text: &str) -> String {
    text.replace('_', "__")
}
