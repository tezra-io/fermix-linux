//! The Fermix unit through the systemd user manager, and linger through logind,
//! over D-Bus (spec §6). Every refusal comes back as a sentence.

use fermix_client::service::{ServiceRead, UnitFacts};
use gtk::gio;
use gtk::glib::{self, variant::ObjectPath, variant::ToVariant};
use std::collections::HashMap;

/// systemd's own deadline for one request.
const TIMEOUT_MS: i32 = 10_000;
const UNIT: &str = "fermix.service";
const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Debug, Clone, Copy)]
pub enum UnitVerb {
    Start,
    Restart,
}

/// Asks the systemd user manager to start or restart the Fermix service.
/// The failed state is reset first, so a unit that hit its start limit can run
/// again and a restart never spends one of the unit's few starts on a stale
/// failure (M38 §6.3, §6.4). Returns systemd's own error text when it refuses.
pub async fn systemd(verb: UnitVerb) -> Result<(), String> {
    let (method, command) = match verb {
        UnitVerb::Start => ("StartUnit", "systemctl --user start fermix"),
        UnitVerb::Restart => ("RestartUnit", "systemctl --user restart fermix"),
    };
    let run = async {
        let bus = gio::bus_get_future(gio::BusType::Session).await?;
        reset_failed(&bus).await;
        manager(&bus, method, Some(&(UNIT, "replace").to_variant())).await
    };
    run.await.map_err(|e| systemd_sentence(&e, command))
}

/// "Run in the background" on (spec §6.3): linger first, since a service that
/// stops at logout is not in the background; then register the unit and start it.
/// Only call this when a binding is known to exist.
pub async fn enable_service(linger: Option<bool>) -> Result<(), String> {
    if linger != Some(true) {
        set_linger().await?;
    }
    let run = async {
        let bus = gio::bus_get_future(gio::BusType::Session).await?;
        manager(&bus, "Reload", None).await?;
        reset_failed(&bus).await;
        let units = vec![UNIT];
        manager(
            &bus,
            "EnableUnitFiles",
            Some(&(units, false, false).to_variant()),
        )
        .await?;
        manager(&bus, "Reload", None).await?;
        manager(&bus, "StartUnit", Some(&(UNIT, "replace").to_variant())).await
    };
    run.await.map_err(|e| {
        let why = systemd_sentence(&e, "systemctl --user enable --now fermix");
        format!("Not turned on: {why}")
    })
}

/// "Run in the background" off: unregister and stop. Linger and the binding stay.
pub async fn disable_service() -> Result<(), String> {
    let run = async {
        let bus = gio::bus_get_future(gio::BusType::Session).await?;
        let units = vec![UNIT];
        manager(&bus, "DisableUnitFiles", Some(&(units, false).to_variant())).await?;
        manager(&bus, "Reload", None).await?;
        manager(&bus, "StopUnit", Some(&(UNIT, "replace").to_variant())).await
    };
    run.await.map_err(|e| {
        let why = systemd_sentence(&e, "systemctl --user disable --now fermix");
        format!("Not turned off: {why}")
    })
}

/// What systemd and logind say about the unit now. A manager that does not
/// answer is `Unreachable`, never a stopped service (spec §6.2).
pub async fn read_service() -> ServiceRead {
    let bus = match gio::bus_get_future(gio::BusType::Session).await {
        Ok(bus) => bus,
        Err(e) => {
            glib::g_warning!("fermix", "no session bus: {e}");
            return ServiceRead::Unreachable;
        }
    };
    match unit_facts(&bus).await {
        Ok(unit) => ServiceRead::Unit {
            unit,
            linger: linger().await,
        },
        Err(e) => {
            glib::g_warning!("fermix", "the service manager did not answer: {e}");
            ServiceRead::Unreachable
        }
    }
}

async fn unit_facts(bus: &gio::DBusConnection) -> Result<UnitFacts, glib::Error> {
    let loaded = call(
        bus,
        SYSTEMD,
        SYSTEMD_PATH,
        MANAGER,
        "LoadUnit",
        Some(&(UNIT,).to_variant()),
    )
    .await?;
    let path = object_path(&loaded)?;
    let args = ("org.freedesktop.systemd1.Unit",).to_variant();
    let all = call(bus, SYSTEMD, &path, PROPERTIES, "GetAll", Some(&args)).await?;
    let (properties,) = all
        .get::<(HashMap<String, glib::Variant>,)>()
        .ok_or_else(|| shape_error("GetAll"))?;
    let text = |key: &str| {
        properties
            .get(key)
            .and_then(|v| v.get::<String>())
            .unwrap_or_default()
    };
    Ok(UnitFacts {
        load_state: text("LoadState"),
        unit_file_state: text("UnitFileState"),
        active_state: text("ActiveState"),
        fragment_path: text("FragmentPath"),
    })
}

/// logind's `Linger` for this user, or `None` when it could not be asked (the
/// sandbox may not reach the system bus).
async fn linger() -> Option<bool> {
    let read = async {
        let bus = gio::bus_get_future(gio::BusType::System).await?;
        let user = user_path(&bus).await?;
        let args = ("org.freedesktop.login1.User", "Linger").to_variant();
        let reply = call(&bus, LOGIND, &user, PROPERTIES, "Get", Some(&args)).await?;
        reply
            .child_value(0)
            .as_variant()
            .and_then(|v| v.get::<bool>())
            .ok_or_else(|| shape_error("Linger"))
    };
    read.await
        .inspect_err(|e| glib::g_info!("fermix", "linger could not be read: {e}"))
        .ok()
}

async fn user_path(bus: &gio::DBusConnection) -> Result<String, glib::Error> {
    let uid = gio::Credentials::new().unix_user()?;
    let reply = call(
        bus,
        LOGIND,
        LOGIND_PATH,
        &format!("{LOGIND}.Manager"),
        "GetUser",
        Some(&(uid,).to_variant()),
    )
    .await?;
    object_path(&reply)
}

/// Keeps this user's services running after logout and starts them at boot.
async fn set_linger() -> Result<(), String> {
    let set = async {
        let bus = gio::bus_get_future(gio::BusType::System).await?;
        let uid = gio::Credentials::new().unix_user()?;
        let args = (uid, true, true).to_variant();
        call(
            &bus,
            LOGIND,
            LOGIND_PATH,
            &format!("{LOGIND}.Manager"),
            "SetUserLinger",
            Some(&args),
        )
        .await
    };
    set.await.map(|_| ()).map_err(|e| {
        glib::g_warning!("fermix", "SetUserLinger failed: {e}");
        format!(
            "Fermix could not stay on after you log out ({}). Run {} in a terminal, then try again.",
            plain(&e),
            fermix_client::service::LINGER_COMMAND
        )
    })
}

async fn reset_failed(bus: &gio::DBusConnection) {
    if let Err(e) = manager(bus, "ResetFailedUnit", Some(&(UNIT,).to_variant())).await {
        // Not fatal: a unit that never failed has nothing to reset.
        glib::g_info!("fermix", "reset-failed: {e}");
    }
}

async fn manager(
    bus: &gio::DBusConnection,
    method: &str,
    args: Option<&glib::Variant>,
) -> Result<(), glib::Error> {
    call(bus, SYSTEMD, SYSTEMD_PATH, MANAGER, method, args)
        .await
        .map(|_reply| ())
}

async fn call(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: Option<&glib::Variant>,
) -> Result<glib::Variant, glib::Error> {
    bus.call_future(
        Some(name),
        path,
        interface,
        method,
        args,
        None,
        gio::DBusCallFlags::NONE,
        TIMEOUT_MS,
    )
    .await
}

fn object_path(reply: &glib::Variant) -> Result<String, glib::Error> {
    reply
        .get::<(ObjectPath,)>()
        .map(|(path,)| path.as_str().to_owned())
        .ok_or_else(|| shape_error("an object path"))
}

fn shape_error(what: &str) -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::InvalidData,
        &format!("unexpected reply for {what}"),
    )
}

/// systemd's refusal as a sentence: an unreachable service manager says so
/// and names the terminal `command` that does the same; anything else is
/// systemd's own message without the D-Bus wrapper.
fn systemd_sentence(e: &glib::Error, command: &str) -> String {
    let unreachable = [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
        "org.freedesktop.DBus.Error.AccessDenied",
    ];
    let name = gio::DBusError::remote_error(e).map(|n| n.to_string());
    if name.as_deref().is_some_and(|n| unreachable.contains(&n)) {
        return format!(
            "this app cannot reach your service manager. Run {command} in a terminal instead."
        );
    }
    plain(e)
}

fn plain(e: &glib::Error) -> String {
    let mut stripped = e.clone();
    gio::DBusError::strip_remote_error(&mut stripped);
    stripped.message().to_owned()
}
