//! Installs a downloaded, checked package through PackageKit on the system
//! bus (proposal, revision 5): a transaction, InstallFiles with only-trusted,
//! and once more without it when PackageKit says the package carries no GPG
//! signature, as any local .deb and the unsigned .rpm do. The system's polkit
//! prompt asks for the password; this app ships no rule and no helper. What
//! each ending means is core's (`update::packagekit`). The tests run against a
//! fake PackageKit on a bus of their own, never the host's.

use fermix_client::update::packagekit::{
    self as pk, ending, refusal, Ending, ONLY_TRUSTED, PERCENT_UNKNOWN, UNTRUSTED,
};
use futures_channel::oneshot;
use gtk::gio::{self, prelude::*};
use gtk::glib::{self, variant::ObjectPath, Variant};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

/// How long one call may take to be answered.
const CALL_TIMEOUT_MS: i32 = 30_000;
/// How long a transaction may run, the password prompt included.
const TRANSACTION_CAP: Duration = Duration::from_secs(60 * 60);
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// Where a transaction stands, as its properties last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub percent: Option<u32>,
    /// PkStatusEnum.
    pub status: u32,
}

/// `Finished`'s exit, and the last `ErrorCode` before it.
type Finish = (u32, Option<(u32, String)>);

/// Installs `file` through the host's PackageKit.
pub async fn install(file: &Path, progress: impl Fn(Progress) + Clone + 'static) -> Ending {
    match gio::bus_get_future(gio::BusType::System).await {
        Ok(bus) => install_on(&bus, file, progress).await,
        Err(e) => {
            glib::g_warning!("fermix", "no system bus to reach PackageKit: {e}");
            refusal(None, e.message())
        }
    }
}

/// Installs `file` through the PackageKit on `bus`: only-trusted, then at
/// most once more untrusted.
pub async fn install_on(
    bus: &gio::DBusConnection,
    file: &Path,
    progress: impl Fn(Progress) + Clone + 'static,
) -> Ending {
    let trusted = transaction(bus, file, ONLY_TRUSTED, progress.clone()).await;
    if trusted != Ending::RetryUntrusted {
        return trusted;
    }
    glib::g_info!(
        "fermix",
        "PackageKit wants the unsigned package installed untrusted"
    );
    transaction(bus, file, UNTRUSTED, progress).await
}

async fn transaction(
    bus: &gio::DBusConnection,
    file: &Path,
    flags: u64,
    progress: impl Fn(Progress) + 'static,
) -> Ending {
    let path = match create(bus).await {
        Ok(path) => path,
        Err(e) => return refused(&e),
    };
    let hints = vec!["interactive=true", "background=false"];
    if let Err(e) = call(
        bus,
        &path,
        pk::TRANSACTION,
        "SetHints",
        &(hints,).to_variant(),
    )
    .await
    {
        return refused(&e);
    }
    let (finished, _subscriptions) = follow(bus, &path, progress);
    let files = vec![file.to_string_lossy().into_owned()];
    let args = (flags, files).to_variant();
    if let Err(e) = call(bus, &path, pk::TRANSACTION, "InstallFiles", &args).await {
        return refused(&e);
    }
    match glib::future_with_timeout(TRANSACTION_CAP, finished).await {
        Ok(Ok((exit, error))) => {
            let error = error
                .as_ref()
                .map(|(code, details)| (*code, details.as_str()));
            ending(exit, error, flags == ONLY_TRUSTED)
        }
        Ok(Err(_)) => Ending::Failed("PackageKit stopped following the installation.".into()),
        Err(_) => Ending::Failed("PackageKit did not finish within an hour.".into()),
    }
}

async fn create(bus: &gio::DBusConnection) -> Result<String, glib::Error> {
    let reply = call(
        bus,
        pk::PATH,
        pk::MANAGER,
        "CreateTransaction",
        &().to_variant(),
    )
    .await?;
    reply
        .get::<(ObjectPath,)>()
        .map(|(path,)| path.as_str().to_owned())
        .ok_or_else(|| {
            glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                "CreateTransaction answered no object path",
            )
        })
}

/// Hears the transaction's `ErrorCode`, `Finished` and property changes. The
/// subscriptions end when the returned guards are dropped.
fn follow(
    bus: &gio::DBusConnection,
    path: &str,
    progress: impl Fn(Progress) + 'static,
) -> (oneshot::Receiver<Finish>, [gio::SignalSubscription; 2]) {
    let (sender, finished) = oneshot::channel();
    let sender = RefCell::new(Some(sender));
    let error: RefCell<Option<(u32, String)>> = RefCell::default();
    let transaction = bus.subscribe_to_signal(
        Some(pk::NAME),
        Some(pk::TRANSACTION),
        None,
        Some(path),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| match signal.signal_name {
            "ErrorCode" => *error.borrow_mut() = signal.parameters.get::<(u32, String)>(),
            "Finished" => {
                let exit = signal
                    .parameters
                    .get::<(u32, u32)>()
                    .map_or(0, |(exit, _)| exit);
                if let Some(sender) = sender.borrow_mut().take() {
                    // The receiver goes only once the install stopped waiting.
                    let _ = sender.send((exit, error.borrow_mut().take()));
                }
            }
            _ => {}
        },
    );
    let now = Rc::new(Cell::new(Progress::default()));
    let properties = bus.subscribe_to_signal(
        Some(pk::NAME),
        Some(PROPERTIES),
        Some("PropertiesChanged"),
        Some(path),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            if let Some(next) = changed(now.get(), signal.parameters) {
                now.set(next);
                progress(next);
            }
        },
    );
    (finished, [transaction, properties])
}

/// The progress after a PropertiesChanged, if it touched Percentage or Status.
fn changed(now: Progress, parameters: &Variant) -> Option<Progress> {
    let (_, changed, _) = parameters.get::<(String, HashMap<String, Variant>, Vec<String>)>()?;
    let percent = changed.get("Percentage").and_then(Variant::get::<u32>);
    let status = changed.get("Status").and_then(Variant::get::<u32>);
    if percent.is_none() && status.is_none() {
        return None;
    }
    Some(Progress {
        percent: percent.map_or(now.percent, |p| (p != PERCENT_UNKNOWN).then_some(p)),
        status: status.unwrap_or(now.status),
    })
}

async fn call(
    bus: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    args: &Variant,
) -> Result<Variant, glib::Error> {
    bus.call_future(
        Some(pk::NAME),
        path,
        interface,
        method,
        Some(args),
        None,
        gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
        CALL_TIMEOUT_MS,
    )
    .await
}

fn refused(e: &glib::Error) -> Ending {
    glib::g_warning!("fermix", "PackageKit refused: {e}");
    let name = gio::DBusError::remote_error(e).map(|n| n.to_string());
    let mut stripped = e.clone();
    gio::DBusError::strip_remote_error(&mut stripped);
    refusal(name.as_deref(), stripped.message())
}

#[cfg(test)]
mod tests {
    //! A private dbus-daemon for each test, with a fake PackageKit on it that
    //! answers InstallFiles as a script says and records every call.

    use super::*;
    use fermix_client::update::packagekit::{error, exit, status, Fallback};
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;

    const FILE: &str = "/home/ada/.cache/fermix-desktop/updates/fermix-desktop_0.4.0_amd64.deb";
    static BUSES: AtomicU32 = AtomicU32::new(0);
    /// A transaction that never finishes fails the test instead of hanging it.
    const TEST_CAP: Duration = Duration::from_secs(20);

    /// A bus of the test's own: its daemon is killed and its folder removed when dropped.
    struct PrivateBus {
        daemon: Child,
        dir: PathBuf,
        address: String,
    }

    impl PrivateBus {
        fn start() -> PrivateBus {
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

        async fn connect(&self) -> gio::DBusConnection {
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
    enum Answer {
        Succeed,
        Fail(u32, &'static str),
    }

    #[derive(Default)]
    struct Seen {
        installs: Vec<(u64, Vec<String>)>,
        hints: Vec<Vec<String>>,
    }

    const MANAGER_XML: &str = r#"<node><interface name="org.freedesktop.PackageKit">
      <method name="CreateTransaction"><arg type="o" direction="out"/></method>
    </interface></node>"#;
    const TRANSACTION_XML: &str = r#"<node><interface name="org.freedesktop.PackageKit.Transaction">
      <method name="SetHints"><arg type="as" direction="in"/></method>
      <method name="InstallFiles"><arg type="t" direction="in"/><arg type="as" direction="in"/></method>
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
    fn fake(bus: &gio::DBusConnection, answers: Vec<Answer>) -> Rc<RefCell<Seen>> {
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
            .method_call(move |_, _, _, _, method, params, invocation| {
                if method == "SetHints" {
                    seen.borrow_mut()
                        .hints
                        .push(params.get::<(Vec<String>,)>().unwrap().0);
                    return invocation.return_value(None);
                }
                seen.borrow_mut()
                    .installs
                    .push(params.get::<(u64, Vec<String>)>().unwrap());
                // The reply goes out first, then the transaction's signals, as
                // PackageKit sends them.
                invocation.return_value(None);
                run_script(&conn, &at, answer);
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

    /// Runs `answers` against a fresh bus: the ending, what the fake saw, and every progress.
    fn install_against(answers: Option<Vec<Answer>>) -> (Ending, Seen, Vec<Progress>) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                context.block_on(async {
                    let bus = PrivateBus::start();
                    let (client, service) = (bus.connect().await, bus.connect().await);
                    let seen = answers.map(|a| fake(&service, a));
                    let heard = Rc::new(RefCell::new(Vec::new()));
                    let record = heard.clone();
                    let install = install_on(&client, Path::new(FILE), move |p| {
                        record.borrow_mut().push(p)
                    });
                    let ended = glib::future_with_timeout(TEST_CAP, install)
                        .await
                        .expect("the install ends within the test's cap");
                    let seen = seen.map(|s| s.take()).unwrap_or_default();
                    let heard = heard.take();
                    (ended, seen, heard)
                })
            })
            .unwrap()
    }

    #[test]
    fn an_unsigned_package_is_installed_on_the_untrusted_pass() {
        let unsigned = Answer::Fail(error::MISSING_GPG_SIGNATURE, "package is not signed");
        let (ended, seen, heard) = install_against(Some(vec![unsigned, Answer::Succeed]));
        assert_eq!(ended, Ending::Installed);
        assert_eq!(
            seen.installs,
            [
                (ONLY_TRUSTED, vec![FILE.to_owned()]),
                (UNTRUSTED, vec![FILE.to_owned()])
            ],
            "only-trusted first, then untrusted, nothing more"
        );
        assert!(seen
            .hints
            .iter()
            .all(|h| h.contains(&"interactive=true".to_owned())));
        assert!(heard.contains(&Progress {
            percent: Some(50),
            status: status::WAITING_FOR_LOCK
        }));
    }

    #[test]
    fn a_trusted_install_needs_one_pass() {
        let (ended, seen, _) = install_against(Some(vec![Answer::Succeed]));
        assert_eq!(ended, Ending::Installed);
        assert_eq!(seen.installs.len(), 1);
    }

    #[test]
    fn no_authorization_or_a_held_lock_falls_back_on_the_first_pass() {
        let denied = Answer::Fail(error::NOT_AUTHORIZED, "Failed to obtain authentication.");
        let (ended, seen, _) = install_against(Some(vec![denied]));
        assert_eq!(ended, Ending::Fallback(Fallback::NotAuthorized));
        assert_eq!(seen.installs.len(), 1);
        let locked = Answer::Fail(error::LOCK_REQUIRED, "unattended-upgrades is running");
        let (ended, _, _) = install_against(Some(vec![locked]));
        assert_eq!(ended, Ending::Fallback(Fallback::Locked));
    }

    #[test]
    fn a_failure_carries_packagekit_s_words() {
        let conflict = Answer::Fail(35, "file /usr/bin/fermix conflicts with fermix 0.15.0");
        let (ended, _, _) = install_against(Some(vec![conflict]));
        assert_eq!(
            ended,
            Ending::Failed("file /usr/bin/fermix conflicts with fermix 0.15.0".into())
        );
    }

    #[test]
    fn a_bus_without_packagekit_falls_back_to_the_command() {
        let (ended, _, _) = install_against(None);
        assert_eq!(ended, Ending::Fallback(Fallback::Unavailable));
    }
}
