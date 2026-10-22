//! A fake PackageKit for tests: a private dbus-daemon of the test's own,
//! never the host's buses, with a PackageKit on it that answers InstallFiles
//! as a script says and records every call.

use fermix_client::update::packagekit::{self as pk, exit, status};
use gtk::gio::{self, prelude::*};
use gtk::glib::{variant::ObjectPath, Variant};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

static BUSES: AtomicU32 = AtomicU32::new(0);
/// A transaction that never finishes fails the test instead of hanging it.
pub const TEST_CAP: Duration = Duration::from_secs(20);

/// A bus of the test's own: its daemon is killed and its folder removed when dropped.
pub struct PrivateBus {
    daemon: Child,
    dir: PathBuf,
    address: String,
}

impl PrivateBus {
    pub fn start() -> PrivateBus {
        let n = BUSES.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fermix-pk-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><listen>unix:path={}/bus</listen>\
             <auth>EXTERNAL</auth><policy context=\"default\"><allow send_destination=\"*\"/>\
             <allow receive_sender=\"*\"/><allow own=\"*\"/></policy></busconfig>",
                dir.display()
            ),
        )
        .unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon runs in the SDK");
        let stdout = daemon.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let read = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = sender.send(read);
        });
        let address = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("dbus-daemon prints its address")
            .unwrap();
        PrivateBus {
            daemon,
            dir,
            address: address.trim().to_owned(),
        }
    }

    pub async fn connect(&self) -> gio::DBusConnection {
        let flags = gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
        gio::DBusConnection::for_address_future(&self.address, flags, None)
            .await
            .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// How the fake answers each InstallFiles, pass by pass.
#[derive(Clone, Copy)]
pub enum Answer {
    Succeed,
    Fail(u32, &'static str),
    /// Never finishes, as one whose password prompt is left open.
    Hang,
}

#[derive(Default)]
pub struct Seen {
    pub installs: Vec<(u64, Vec<String>)>,
    pub hints: Vec<Vec<String>>,
    /// The transactions cancelled, by path.
    pub cancels: Vec<String>,
}

const MANAGER_XML: &str = r#"<node><interface name="org.freedesktop.PackageKit">
  <method name="CreateTransaction"><arg type="o" direction="out"/></method>
</interface></node>"#;
const TRANSACTION_XML: &str = r#"<node><interface name="org.freedesktop.PackageKit.Transaction">
  <method name="SetHints"><arg type="as" direction="in"/></method>
  <method name="InstallFiles"><arg type="t" direction="in"/><arg type="as" direction="in"/></method>
  <method name="Cancel"/>
  <signal name="ErrorCode"><arg type="u"/><arg type="s"/></signal>
  <signal name="Finished"><arg type="u"/><arg type="u"/></signal>
</interface></node>"#;

fn interface(xml: &str, name: &str) -> gio::DBusInterfaceInfo {
    gio::DBusNodeInfo::for_xml(xml)
        .unwrap()
        .lookup_interface(name)
        .unwrap()
}

/// Owns PackageKit's name on `bus` and answers as `answers` says, one per pass.
pub fn fake(bus: &gio::DBusConnection, answers: Vec<Answer>) -> Rc<RefCell<Seen>> {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let reply = bus
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(pk::NAME, 4u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5_000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    assert_eq!(reply.get::<(u32,)>(), Some((1,)), "the fake owns the name");
    let (count, answers) = (Rc::new(Cell::new(0usize)), Rc::new(answers));
    let (conn, record) = (bus.clone(), seen.clone());
    // Registered for the bus's life: the test's bus goes with the test.
    let _registration = bus
        .register_object(pk::PATH, &interface(MANAGER_XML, pk::MANAGER))
        .method_call(move |_, _, _, _, _, _, invocation| {
            let n = count.get() + 1;
            count.set(n);
            let path = format!("/org/freedesktop/PackageKit/t{n}");
            serve_transaction(&conn, &path, answers[n - 1], record.clone());
            let object = ObjectPath::try_from(path).unwrap();
            invocation.return_value(Some(&(object,).to_variant()));
        })
        .build()
        .unwrap();
    seen
}

fn serve_transaction(
    bus: &gio::DBusConnection,
    path: &str,
    answer: Answer,
    seen: Rc<RefCell<Seen>>,
) {
    let (conn, at) = (bus.clone(), path.to_owned());
    // Registered for the bus's life: the test's bus goes with the test.
    let _registration = bus
        .register_object(path, &interface(TRANSACTION_XML, pk::TRANSACTION))
        .method_call(move |_, _, _, _, method, params, invocation| match method {
            "SetHints" => {
                let hints = params.get::<(Vec<String>,)>().unwrap().0;
                seen.borrow_mut().hints.push(hints);
                invocation.return_value(None);
            }
            "Cancel" => {
                seen.borrow_mut().cancels.push(at.clone());
                invocation.return_value(None);
            }
            _ => {
                let install = params.get::<(u64, Vec<String>)>().unwrap();
                seen.borrow_mut().installs.push(install);
                // The reply goes out first, then the transaction's signals, as
                // PackageKit sends them.
                invocation.return_value(None);
                run_script(&conn, &at, answer);
            }
        })
        .build()
        .unwrap();
}

fn run_script(bus: &gio::DBusConnection, path: &str, answer: Answer) {
    let changed: HashMap<String, Variant> = HashMap::from([
        ("Percentage".to_owned(), 50u32.to_variant()),
        ("Status".to_owned(), status::WAITING_FOR_LOCK.to_variant()),
    ]);
    let properties = (pk::TRANSACTION, changed, Vec::<String>::new()).to_variant();
    bus.emit_signal(
        None,
        path,
        PROPERTIES,
        "PropertiesChanged",
        Some(&properties),
    )
    .unwrap();
    let exit = match answer {
        Answer::Succeed => exit::SUCCESS,
        Answer::Hang => return,
        Answer::Fail(code, details) => {
            let args = (code, details).to_variant();
            bus.emit_signal(None, path, pk::TRANSACTION, "ErrorCode", Some(&args))
                .unwrap();
            exit::FAILED
        }
    };
    bus.emit_signal(
        None,
        path,
        pk::TRANSACTION,
        "Finished",
        Some(&(exit, 7u32).to_variant()),
    )
    .unwrap();
}
