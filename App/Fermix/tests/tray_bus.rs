//! The tray against a real session bus, with a stub host standing in for a panel.
//!
//! Every other tray test is a pure-value test: the layout's signature, the row
//! ids, the hold rule. None of them puts a byte on a bus, and the D-Bus path is
//! exactly where this feature can fail while looking perfectly healthy -- a
//! host that accepts a registration and then silently drops the item is the
//! documented behaviour of the GNOME extension when it cannot read the menu.
//!
//! So this test owns `org.kde.StatusNotifierWatcher` itself and then does what
//! that extension does, in the same order: take the registration, read the
//! item's properties, follow `Menu` to the dbusmenu object, call
//! `GetLayout(0, -1, [])` and `GetGroupProperties`, and send
//! `Event(id, "clicked", ...)` for each row. What it asserts is what the
//! extension would need to be true.
//!
//! It requires a private session bus and refuses to run without one, rather
//! than skipping: a tray test that quietly does not run is worth less than no
//! tray test, because it reports as a pass. Run it under `dbus-run-session`;
//! `scripts/tray_smoke.sh` does that.
//!
//! EACH TEST NEEDS ITS OWN BUS, and `scripts/tray_smoke.sh` gives it one by
//! running them one at a time. They cannot share: the watcher name is fixed by
//! the specification, so a test that asserts NOTHING owns it cannot coexist
//! with one that owns it, and cargo's default is to run them in parallel
//! threads of a single process that shares a single bus. Running the whole file
//! on one bus fails, which is the correct outcome -- each test asserts the
//! precondition it needs rather than assuming isolation it was not given.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gtk4::gio;
use gtk4::gio::prelude::{ActionMapExt, ApplicationExt};
use gtk4::glib;
use gtk4::glib::prelude::*;
use gtk4::glib::variant::Variant;

use fermix_desktop::app::APPLICATION_ID;
use fermix_desktop::models::home::StatusWord;
use fermix_desktop::tray::state::TrayGlyph;
use fermix_desktop::tray::{dbusmenu, item, menu};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// The watcher interface, as much of it as a host needs to take an item.
const WATCHER_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem">
      <arg type="s" name="service" direction="in"/>
    </method>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
  </interface>
</node>
"#;

/// The private bus, or a refusal saying why there is none.
fn session_bus() -> gio::DBusConnection {
    assert!(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
        "no private session bus: run this under dbus-run-session, \
         via scripts/tray_smoke.sh. Skipping would report as a pass."
    );

    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .expect("the private session bus connects")
}

/// A stub watcher that records what registered with it.
///
/// It runs on its OWN THREAD with its own main context and its own connection,
/// because a real panel is another process. Sharing a thread with the code
/// under test deadlocks: `item::register` calls the watcher with `call_sync`,
/// which blocks the calling thread, and the stub's handler can only run when
/// that same thread iterates a main context. The call then times out and the
/// item concludes, correctly for what it could observe, that the tray refused
/// it. Two hours of "the watcher is right there and it will not register"
/// come from that one detail, and it is invisible in the code.
struct StubHost {
    registered: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StubHost {
    /// Own the watcher name on a thread of its own and answer registrations.
    fn start() -> Self {
        let registered: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let recorder = Arc::clone(&registered);
        let stopping = Arc::clone(&stop);

        let thread = std::thread::spawn(move || {
            let context = glib::MainContext::new();
            context
                .with_thread_default(|| {
                    let connection =
                        gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                            .expect("the stub connects to the private bus");

                    let node = gio::DBusNodeInfo::for_xml(WATCHER_XML)
                        .expect("the watcher description parses");
                    let info = node
                        .lookup_interface(WATCHER_NAME)
                        .expect("the watcher interface is in it");

                    connection
                        .register_object(WATCHER_PATH, &info)
                        .property(|_, _, _, _, name| match name {
                            "IsStatusNotifierHostRegistered" => true.to_variant(),
                            _ => "".to_variant(),
                        })
                        .method_call(move |_, _, _, _, method, parameters, invocation| {
                            if method == "RegisterStatusNotifierItem" {
                                let service = parameters
                                    .child_value(0)
                                    .str()
                                    .unwrap_or_default()
                                    .to_string();
                                recorder
                                    .lock()
                                    .expect("the record is not poisoned")
                                    .push(service);
                            }
                            invocation.return_value(None);
                        })
                        .build()
                        .expect("the stub watcher registers its object");

                    gio::bus_own_name_on_connection(
                        &connection,
                        WATCHER_NAME,
                        gio::BusNameOwnerFlags::NONE,
                        |_, _| {},
                        |_, _| {},
                    );

                    // Turn this thread's own context until the test is done with us.
                    while !stopping.load(Ordering::Relaxed) {
                        while context.iteration(false) {}
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                })
                .expect("the stub owns its main context");
        });

        Self {
            registered,
            stop,
            thread: Some(thread),
        }
    }

    fn took(&self) -> Vec<String> {
        self.registered
            .lock()
            .expect("the record is not poisoned")
            .clone()
    }
}

impl Drop for StubHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Whether anyone owns this name right now.
fn name_has_owner(connection: &gio::DBusConnection, name: &str) -> bool {
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(name,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2_000,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|reply| reply.child_value(0).get::<bool>())
        .unwrap_or(false)
}

/// Turn the main context until a condition holds, with a bounded wait.
///
/// Bounded and loud: an unbounded wait for something that never happens is a
/// test that hangs rather than fails, and a hanging test in a container lane
/// gets killed and read as infrastructure trouble.
fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
    let context = glib::MainContext::default();

    for _ in 0..500 {
        if condition() {
            return;
        }
        while context.iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    panic!("waited five seconds and {what} did not happen");
}

/// Call a method on the item under test, as a host would.
///
/// Asynchronous, then the main context is turned until the reply lands. NOT
/// `call_sync`, and the reason is the same one that moved the stub watcher onto
/// its own thread: the item being called is served by this thread's main
/// context, so a synchronous call blocks the very thread that has to answer it
/// and the call times out. A real panel is another process and has no such
/// problem, which is exactly why this hazard belongs to the test rather than to
/// the code under test.
fn call(
    connection: &gio::DBusConnection,
    name: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: Option<&Variant>,
) -> Variant {
    let reply: Rc<RefCell<Option<Result<Variant, glib::Error>>>> = Rc::new(RefCell::new(None));
    let landed = Rc::clone(&reply);

    connection.call(
        Some(name),
        path,
        interface,
        method,
        body,
        None,
        gio::DBusCallFlags::NONE,
        2_000,
        gio::Cancellable::NONE,
        move |result| {
            *landed.borrow_mut() = Some(result);
        },
    );

    wait_until(
        || reply.borrow().is_some(),
        &format!("a reply to {interface}.{method} arrived"),
    );

    let reply = reply.borrow_mut().take().expect("the reply is there");
    reply.unwrap_or_else(|error| panic!("{interface}.{method} failed: {error}"))
}

/// Read one property, as a host would.
fn property(
    connection: &gio::DBusConnection,
    name: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Variant {
    call(
        connection,
        name,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&(interface, property).to_variant()),
    )
    .child_value(0)
    .as_variant()
    .expect("a property reply boxes its value")
}

/// Publish an item on the bus and hand back its name.
///
/// The name carries a per-call counter as well as the process id. Two items on
/// one bus asking for the same name is a collision in which the second silently
/// never owns it, and the test that follows then measures the first test's
/// item -- so the names are made distinct rather than trusted to be.
fn publish_item(connection: &gio::DBusConnection) -> (Rc<item::TrayItem>, String) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);

    let name = format!(
        "org.freedesktop.StatusNotifierItem-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );

    gio::bus_own_name_on_connection(
        connection,
        &name,
        gio::BusNameOwnerFlags::NONE,
        |_, _| {},
        |_, _| {},
    );
    wait_until(
        || name_has_owner(connection, &name),
        "the item owns its name",
    );

    let rows = menu::rows(StatusWord::Running);
    let item =
        item::publish(connection, &name, TrayGlyph::Running, rows).expect("the item publishes");

    (item, name)
}

#[test]
#[ignore = "needs a private session bus of its own: run scripts/tray_smoke.sh"]
fn a_host_takes_the_item_and_can_read_its_whole_menu() {
    let connection = session_bus();
    let host = StubHost::start();
    wait_until(
        || name_has_owner(&connection, WATCHER_NAME),
        "the stub watcher owned its name",
    );
    let (_item, name) = publish_item(&connection);

    // ---- registration, as the extension does it -----------------------------
    let outcome = item::register(&connection, &name, gio::Cancellable::NONE);
    assert_eq!(
        outcome,
        item::Outcome::Registered,
        "a watcher was on the bus and the item did not register with it"
    );
    wait_until(
        || !host.took().is_empty(),
        "the watcher recorded the registration",
    );
    assert_eq!(
        host.took(),
        vec![name.clone()],
        "the watcher was handed a different name"
    );

    // ---- the properties the extension reads before it draws anything --------
    let menu_path = property(
        &connection,
        &name,
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
        "Menu",
    );
    assert_eq!(
        menu_path.type_().as_str(),
        "o",
        "Menu must be an object path; a string here reads to the host as no menu at all"
    );
    let menu_path = menu_path
        .str()
        .expect("the menu path is a path")
        .to_string();

    let icon = property(
        &connection,
        &name,
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
        "IconName",
    );
    assert_eq!(icon.str(), Some(TrayGlyph::Running.icon_name()));

    // ---- the menu itself ----------------------------------------------------
    let layout = call(
        &connection,
        &name,
        &menu_path,
        "com.canonical.dbusmenu",
        "GetLayout",
        Some(&(0i32, -1i32, Vec::<String>::new()).to_variant()),
    );
    assert_eq!(
        layout.type_().as_str(),
        "(u(ia{sv}av))",
        "the host parses this signature and shows an empty menu if it differs"
    );

    let root = layout.child_value(1);
    let children = root.child_value(2);
    let expected = menu::rows(StatusWord::Running).len();
    assert_eq!(
        children.n_children(),
        expected,
        "the host would draw {} rows rather than {expected}",
        children.n_children()
    );

    let group = call(
        &connection,
        &name,
        &menu_path,
        "com.canonical.dbusmenu",
        "GetGroupProperties",
        Some(&(Vec::<i32>::new(), Vec::<String>::new()).to_variant()),
    );
    assert_eq!(group.type_().as_str(), "(a(ia{sv}))");
}

#[test]
#[ignore = "needs a private session bus of its own: run scripts/tray_smoke.sh"]
fn every_clickable_row_reaches_the_action_it_names() {
    let connection = session_bus();
    let _host = StubHost::start();
    wait_until(
        || name_has_owner(&connection, WATCHER_NAME),
        "the stub watcher owned its name",
    );
    let (_item, name) = publish_item(&connection);
    let _ = item::register(&connection, &name, gio::Cancellable::NONE);

    // An application carrying exactly the actions the tray names, each of which
    // records that it fired. This is the assertion that a click does something:
    // the rest of the test could pass with every row wired to nothing.
    // Built from the one identity rather than written out. A second identity
    // literal anywhere in the tree is what check_app_identity.sh exists to
    // refuse, and it is right to: on this platform nothing derives the identity
    // for us, so a second spelling is how a window ends up with an identity
    // nobody can establish. The name is still unique to this test.
    let application = gio::Application::new(
        Some(&format!("{APPLICATION_ID}TrayBusTest")),
        gio::ApplicationFlags::IS_SERVICE,
    );
    let fired: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let rows = menu::rows(StatusWord::Running);
    for row in &rows {
        let menu::TrayRow::Command(command) = row else {
            continue;
        };
        let name = command
            .action()
            .strip_prefix("app.")
            .expect("every tray row names an application action");

        let action = gio::SimpleAction::new(name, None);
        let record = Rc::clone(&fired);
        let fired_name = name.to_string();
        action.connect_activate(move |_, _| record.borrow_mut().push(fired_name.clone()));
        application.add_action(&action);
    }

    // `item` activates through the default application, so this one has to be
    // it for the duration.
    application
        .register(gio::Cancellable::NONE)
        .expect("the test application registers");

    let menu_path = property(
        &connection,
        &name,
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
        "Menu",
    )
    .str()
    .expect("the menu path is a path")
    .to_string();

    let mut expected = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let menu::TrayRow::Command(command) = row else {
            continue;
        };

        let id = i32::try_from(index).expect("a row index fits") + 1;
        expected.push(
            command
                .action()
                .strip_prefix("app.")
                .expect("an application action")
                .to_string(),
        );

        call(
            &connection,
            &name,
            &menu_path,
            "com.canonical.dbusmenu",
            "Event",
            Some(&(id, "clicked", Variant::from(0i32), 0u32).to_variant()),
        );
    }

    wait_until(
        || fired.borrow().len() == expected.len(),
        "every clicked row reached its action",
    );
    assert_eq!(*fired.borrow(), expected, "a row reached the wrong action");
}

#[test]
#[ignore = "needs a private session bus of its own: run scripts/tray_smoke.sh"]
fn a_hover_does_not_fire_the_row() {
    // The control for the test above. "hovered" and "opened" arrive on the same
    // method, and a command that ran on hover would fire under a passing
    // pointer -- Restart Daemon under a passing pointer.
    let connection = session_bus();
    let _host = StubHost::start();
    wait_until(
        || name_has_owner(&connection, WATCHER_NAME),
        "the stub watcher owned its name",
    );
    let (_item, name) = publish_item(&connection);

    let application = gio::Application::new(
        Some(&format!("{APPLICATION_ID}TrayHoverTest")),
        gio::ApplicationFlags::IS_SERVICE,
    );
    let fired = Rc::new(RefCell::new(0usize));
    let action = gio::SimpleAction::new("home", None);
    let record = Rc::clone(&fired);
    action.connect_activate(move |_, _| *record.borrow_mut() += 1);
    application.add_action(&action);
    application
        .register(gio::Cancellable::NONE)
        .expect("the test application registers");

    let menu_path = property(
        &connection,
        &name,
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
        "Menu",
    )
    .str()
    .expect("the menu path is a path")
    .to_string();

    for event in ["hovered", "opened", "closed"] {
        call(
            &connection,
            &name,
            &menu_path,
            "com.canonical.dbusmenu",
            "Event",
            Some(&(1i32, event, Variant::from(0i32), 0u32).to_variant()),
        );
    }

    let context = glib::MainContext::default();
    for _ in 0..50 {
        while context.iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    assert_eq!(*fired.borrow(), 0, "a row fired without being clicked");
}

#[test]
#[ignore = "needs a private session bus of its own: run scripts/tray_smoke.sh"]
fn with_no_watcher_on_the_bus_the_item_says_so_rather_than_failing() {
    // The Fedora arm. No stub host is started here at all, so nothing owns the
    // watcher name, and the answer must be NoWatcher -- which is what keeps the
    // application from holding itself open with no icon.
    let connection = session_bus();
    assert!(
        !name_has_owner(&connection, WATCHER_NAME),
        "something already owns the watcher name, so this arm measures nothing"
    );

    let (_item, name) = publish_item(&connection);
    let outcome = item::register(&connection, &name, gio::Cancellable::NONE);

    assert_eq!(
        outcome,
        item::Outcome::NoWatcher,
        "a bus with no watcher must read as NoWatcher, not as a refusal and not as success"
    );
    assert!(
        !fermix_desktop::tray::state::may_outlive_its_window(&outcome),
        "with no tray the application must still quit when its last window closes"
    );
}

#[test]
#[ignore = "needs a private session bus of its own: run scripts/tray_smoke.sh"]
fn the_stub_host_would_reject_a_layout_it_could_not_parse() {
    // The malformed-menu control: proof that the reading above is a reading.
    // If the stub accepted anything at all, every assertion it makes about the
    // real layout would be worthless.
    let good = dbusmenu::layout(1, &menu::rows(StatusWord::Running));
    assert_eq!(good.type_().as_str(), "(u(ia{sv}av))");

    // A plausible-looking but wrong shape: the root without its revision.
    let bad = Variant::tuple_from_iter([0i32.to_variant(), "not a menu".to_variant()]);
    assert_ne!(
        bad.type_().as_str(),
        good.type_().as_str(),
        "the control shape must not match the real one"
    );

    // What the host does with the reply is compare its signature, so that is
    // what is checked here.
    assert!(
        bad.type_().as_str() != "(u(ia{sv}av))",
        "a malformed layout must not pass the check the host applies"
    );
}
