//! Installs the package at a path through PackageKit on the system
//! bus (proposal, revision 5): one transaction for each InstallFiles, with the
//! flags the install asks for. The install (`update_install`) asks with
//! only-trusted, and once more without it when PackageKit says the package
//! carries no GPG signature, as any local .deb and the unsigned .rpm do. The
//! system's polkit prompt asks for the password; this app ships no rule and no
//! helper. What each ending means is core's (`update::packagekit`). The tests
//! run against a fake PackageKit on a bus of their own, never the host's.

use fermix_client::update::packagekit::{
    self as pk, ending, refusal, Ending, ONLY_TRUSTED, PERCENT_UNKNOWN,
};
use futures_channel::oneshot;
use gtk::gio::{self, prelude::*};
use gtk::glib::{self, variant::ObjectPath, Variant};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// How long one call may take to be answered.
const CALL_TIMEOUT_MS: i32 = 30_000;
/// How long a transaction may run, the password prompt included.
pub const TRANSACTION_CAP: Duration = Duration::from_secs(60 * 60);
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

/// One InstallFiles of `file`, with `flags`, through the host's PackageKit.
pub async fn install(file: &Path, flags: u64, progress: impl Fn(Progress) + 'static) -> Ending {
    match gio::bus_get_future(gio::BusType::System).await {
        Ok(bus) => install_on(&bus, file, flags, TRANSACTION_CAP, progress).await,
        Err(e) => {
            glib::g_warning!("fermix", "no system bus to reach PackageKit: {e}");
            refusal(None, e.message())
        }
    }
}

/// One InstallFiles of `file`, with `flags`, through the PackageKit on `bus`,
/// for at most `cap`.
pub async fn install_on(
    bus: &gio::DBusConnection,
    file: &Path,
    flags: u64,
    cap: Duration,
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
    match glib::future_with_timeout(cap, finished).await {
        Ok(Ok((exit, error))) => {
            let error = error
                .as_ref()
                .map(|(code, details)| (*code, details.as_str()));
            ending(exit, error, flags == ONLY_TRUSTED)
        }
        Ok(Err(_)) => Ending::Failed("PackageKit stopped following the installation.".into()),
        Err(_) => cancel(bus, &path).await,
    }
}

/// Past the cap the transaction is cancelled, so it does not go on once the
/// window has stopped waiting for it.
async fn cancel(bus: &gio::DBusConnection, path: &str) -> Ending {
    glib::g_warning!(
        "fermix",
        "PackageKit's {path} ran past its cap; cancelling it"
    );
    if let Err(e) = call(bus, path, pk::TRANSACTION, "Cancel", &().to_variant()).await {
        glib::g_warning!("fermix", "PackageKit's {path} was not cancelled: {e}");
    }
    Ending::Failed("PackageKit did not finish within an hour.".into())
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
    let transaction = bus.subscribe_to_signal(
        Some(pk::NAME),
        Some(pk::TRANSACTION),
        None,
        Some(path),
        None,
        gio::DBusSignalFlags::NONE,
        ending_heard(sender),
    );
    let properties = bus.subscribe_to_signal(
        Some(pk::NAME),
        Some(PROPERTIES),
        Some("PropertiesChanged"),
        Some(path),
        None,
        gio::DBusSignalFlags::NONE,
        progress_heard(progress),
    );
    (finished, [transaction, properties])
}

/// Keeps the last `ErrorCode`, and on `Finished` sends the exit with it.
fn ending_heard(sender: oneshot::Sender<Finish>) -> impl Fn(gio::DBusSignalRef) + 'static {
    let sender = RefCell::new(Some(sender));
    let error: RefCell<Option<(u32, String)>> = RefCell::default();
    move |signal| {
        if signal.signal_name == "ErrorCode" {
            *error.borrow_mut() = signal.parameters.get::<(u32, String)>();
            return;
        }
        if signal.signal_name != "Finished" {
            return;
        }
        let exit = signal
            .parameters
            .get::<(u32, u32)>()
            .map_or(0, |(exit, _)| exit);
        let Some(sender) = sender.borrow_mut().take() else {
            return;
        };
        // The receiver goes only once the install stopped waiting.
        let _ = sender.send((exit, error.borrow_mut().take()));
    }
}

/// Calls `progress` whenever a PropertiesChanged moves Percentage or Status.
fn progress_heard(progress: impl Fn(Progress) + 'static) -> impl Fn(gio::DBusSignalRef) + 'static {
    let now = Cell::new(Progress::default());
    move |signal| {
        if let Some(next) = changed(now.get(), signal.parameters) {
            now.set(next);
            progress(next);
        }
    }
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
    use crate::packagekit_fake::{fake, Answer, PrivateBus, Seen, TEST_CAP};
    use fermix_client::update::packagekit::{error, status, Fallback, UNTRUSTED};
    use std::rc::Rc;

    const FILE: &str = "/home/ada/.cache/fermix-desktop/updates/fermix-desktop_0.4.0_amd64.deb";

    /// One install with `flags` against a fresh bus whose PackageKit, if any,
    /// gives `answer`: the ending, what the fake saw, and every progress.
    fn install_against(answer: Option<Answer>, flags: u64) -> (Ending, Seen, Vec<Progress>) {
        capped_against(answer, flags, TRANSACTION_CAP)
    }

    /// The same, with the transaction capped at `cap`.
    fn capped_against(
        answer: Option<Answer>,
        flags: u64,
        cap: Duration,
    ) -> (Ending, Seen, Vec<Progress>) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| context.block_on(install_on_bus(answer, flags, cap)))
            .unwrap()
    }

    async fn install_on_bus(
        answer: Option<Answer>,
        flags: u64,
        cap: Duration,
    ) -> (Ending, Seen, Vec<Progress>) {
        let bus = PrivateBus::start();
        let (client, service) = (bus.connect().await, bus.connect().await);
        let seen = answer.map(|a| fake(&service, vec![a]));
        let heard = Rc::new(RefCell::new(Vec::new()));
        let record = heard.clone();
        let install = install_on(&client, Path::new(FILE), flags, cap, move |p| {
            record.borrow_mut().push(p)
        });
        let ended = glib::future_with_timeout(TEST_CAP, install)
            .await
            .expect("the install ends within the test's cap");
        let seen = seen.map(|s| s.take()).unwrap_or_default();
        (ended, seen, heard.take())
    }

    #[test]
    fn an_unsigned_package_ends_the_trusted_pass_asking_for_an_untrusted_one() {
        let unsigned = Answer::Fail(error::MISSING_GPG_SIGNATURE, "package is not signed");
        let (ended, seen, heard) = install_against(Some(unsigned), ONLY_TRUSTED);
        assert_eq!(ended, Ending::RetryUntrusted);
        assert_eq!(seen.installs, [(ONLY_TRUSTED, vec![FILE.to_owned()])]);
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
    fn an_untrusted_pass_installs_with_the_flags_it_is_given() {
        let (ended, seen, _) = install_against(Some(Answer::Succeed), UNTRUSTED);
        assert_eq!(ended, Ending::Installed);
        assert_eq!(seen.installs, [(UNTRUSTED, vec![FILE.to_owned()])]);
    }

    #[test]
    fn a_trusted_install_needs_one_pass() {
        let (ended, seen, _) = install_against(Some(Answer::Succeed), ONLY_TRUSTED);
        assert_eq!(ended, Ending::Installed);
        assert_eq!(seen.installs.len(), 1);
    }

    #[test]
    fn no_authorization_or_a_held_lock_falls_back_on_the_first_pass() {
        let denied = Answer::Fail(error::NOT_AUTHORIZED, "Failed to obtain authentication.");
        let (ended, seen, _) = install_against(Some(denied), ONLY_TRUSTED);
        assert_eq!(ended, Ending::Fallback(Fallback::NotAuthorized));
        assert_eq!(seen.installs.len(), 1);
        let locked = Answer::Fail(error::LOCK_REQUIRED, "unattended-upgrades is running");
        let (ended, _, _) = install_against(Some(locked), ONLY_TRUSTED);
        assert_eq!(ended, Ending::Fallback(Fallback::Locked));
    }

    #[test]
    fn a_failure_carries_packagekit_s_words() {
        let conflict = Answer::Fail(35, "file /usr/bin/fermix conflicts with fermix 0.15.0");
        let (ended, _, _) = install_against(Some(conflict), ONLY_TRUSTED);
        assert_eq!(
            ended,
            Ending::Failed("file /usr/bin/fermix conflicts with fermix 0.15.0".into())
        );
    }

    #[test]
    fn a_bus_without_packagekit_falls_back_to_the_command() {
        let (ended, _, _) = install_against(None, ONLY_TRUSTED);
        assert_eq!(ended, Ending::Fallback(Fallback::Unavailable));
    }

    #[test]
    fn a_transaction_past_its_cap_is_cancelled() {
        let cap = Duration::from_millis(300);
        let (ended, seen, _) = capped_against(Some(Answer::Hang), ONLY_TRUSTED, cap);
        assert_eq!(
            ended,
            Ending::Failed("PackageKit did not finish within an hour.".into())
        );
        assert_eq!(seen.cancels, ["/org/freedesktop/PackageKit/t1"]);
    }
}
