//! "Open at login" through the XDG Background portal (spec §6.6). The portal
//! writes or deletes the autostart entry itself; it has no getter, so the last
//! granted answer is kept in this app's state directory.

use futures_channel::oneshot;
use gtk::gio;
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// How long the desktop's question may stay unanswered before the app gives up.
const ANSWER_WITHIN: Duration = Duration::from_secs(300);
const CALL_TIMEOUT_MS: i32 = 10_000;

/// The portal's `Response`: its code and the `autostart` it granted.
pub type Response = (u32, Option<bool>);
type Waiting = Rc<RefCell<Option<oneshot::Sender<Option<Response>>>>>;

/// Asks the desktop to open this app at login, or to stop. `Err` means the
/// portal could not be asked or never answered.
pub async fn request_background(autostart: bool) -> Result<Response, String> {
    let bus = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(|e| e.to_string())?;
    let token = format!("fermix{}", glib::random_int());
    let request = request_path(&bus, &token)?;
    let (sender, answer) = oneshot::channel();
    let waiting: Waiting = Rc::new(RefCell::new(Some(sender)));
    // Subscribed before the call, so an instant answer is not missed.
    let _subscription = listen(&bus, &request, waiting.clone());
    let give_up = waiting.clone();
    glib::timeout_add_local_once(ANSWER_WITHIN, move || finish(&give_up, None));
    let args = glib::Variant::tuple_from_iter(["".to_variant(), options(&token, autostart)]);
    bus.call_future(
        Some(PORTAL),
        PORTAL_PATH,
        "org.freedesktop.portal.Background",
        "RequestBackground",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        CALL_TIMEOUT_MS,
    )
    .await
    .map_err(|e| e.to_string())?;
    match answer.await {
        Ok(Some(response)) => Ok(response),
        Ok(None) => Err("the desktop did not answer in time".into()),
        Err(e) => Err(format!("the answer was lost: {e}")),
    }
}

/// Where the portal will emit `Response` for a request made with `token`.
fn request_path(bus: &gio::DBusConnection, token: &str) -> Result<String, String> {
    let name = bus.unique_name().ok_or("the session bus gave no name")?;
    let sender = name.trim_start_matches(':').replace('.', "_");
    Ok(format!("{PORTAL_PATH}/request/{sender}/{token}"))
}

fn listen(bus: &gio::DBusConnection, request: &str, waiting: Waiting) -> gio::SignalSubscription {
    bus.subscribe_to_signal(
        Some(PORTAL),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(request),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let parsed = signal
                .parameters
                .get::<(u32, HashMap<String, glib::Variant>)>();
            let Some((code, results)) = parsed else {
                glib::g_warning!(
                    "fermix",
                    "the Background portal answered in an unknown shape"
                );
                return finish(&waiting, Some((2, None)));
            };
            let autostart = results.get("autostart").and_then(|v| v.get::<bool>());
            finish(&waiting, Some((code, autostart)));
        },
    )
}

/// Hands the first outcome to the waiting request; later ones find it gone.
fn finish(waiting: &Waiting, outcome: Option<Response>) {
    let Some(sender) = waiting.borrow_mut().take() else {
        return;
    };
    if sender.send(outcome).is_err() {
        glib::g_debug!(
            "fermix",
            "the Background request stopped waiting before its answer"
        );
    }
}

fn options(token: &str, autostart: bool) -> glib::Variant {
    let options = glib::VariantDict::new(None);
    options.insert("handle_token", token);
    options.insert("reason", "Open Fermix when you log in");
    options.insert("autostart", autostart);
    options.insert("commandline", vec!["fermix-desktop"]);
    options.insert("dbus-activatable", false);
    options.end()
}

fn login_record() -> PathBuf {
    glib::user_state_dir().join("fermix").join("open-at-login")
}

/// Whether the last granted answer was to open at login.
pub fn opens_at_login() -> bool {
    login_record().exists()
}

/// Keeps the granted answer: the record exists exactly while the answer is yes.
pub fn remember_login(on: bool) -> std::io::Result<()> {
    let record = login_record();
    if on {
        let dir = record.parent().expect("the record sits in a directory");
        std::fs::create_dir_all(dir)?;
        return std::fs::write(&record, b"on\n");
    }
    match std::fs::remove_file(&record) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
