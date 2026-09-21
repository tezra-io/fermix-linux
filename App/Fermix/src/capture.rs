//! The capture mode.
//!
//! `fermix-desktop --capture DIR`, in a debug build only, renders each
//! reference state of the running fixture scenario to a PNG through the real
//! widgets at the real window size, and quits. It is how a surface is reviewed
//! against the redlines without anyone having to describe what they saw.
//!
//! The caller runs one capture per scenario and per colour scheme; this module
//! filters the state list down to the states that scenario can actually show,
//! because a capture of a state the fixtures cannot produce would be a picture
//! of something else.

use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;

use crate::app::FermixApplication;
use crate::management::types::SettingsPane;
use crate::metrics;
use crate::models::secret_store::StoreRefusal;
use crate::models::settings_model::Sentence;
use crate::window::FermixWindow;

/// The flag that turns this on.
pub const FLAG: &str = "--capture";

/// How long the main loop is left to settle before a frame is taken. Long
/// enough for the fixture exchange, the poll and the toolkit's own transition.
const SETTLE: Duration = Duration::from_millis(400);

/// The most times the capture waits for a window to exist before giving up.
const WINDOW_TRIES: u32 = 20;

/// One reference state: what to arrange, and which scenario can show it.
struct Reference {
    name: &'static str,
    scenario: &'static str,
    arrange: fn(&FermixWindow),
}

/// Every reference state, in the order they are taken.
const REFERENCES: &[Reference] = &[
    Reference {
        name: "home_running",
        scenario: "default",
        arrange: show_home,
    },
    Reference {
        name: "home_attention",
        scenario: "restart_pending",
        arrange: show_home,
    },
    Reference {
        name: "home_setup_required",
        scenario: "setup_required",
        arrange: show_home,
    },
    Reference {
        name: "home_not_running",
        scenario: "not_running",
        arrange: show_home,
    },
    Reference {
        name: "doctor_running",
        scenario: "default",
        arrange: show_doctor,
    },
    Reference {
        name: "doctor_healthy",
        scenario: "doctor_healthy",
        arrange: show_doctor,
    },
    Reference {
        name: "doctor_failed",
        scenario: "doctor_failed",
        arrange: show_doctor_evidence,
    },
    Reference {
        name: "logs_populated",
        scenario: "default",
        arrange: show_logs,
    },
    Reference {
        name: "logs_empty",
        scenario: "logs_empty",
        arrange: show_logs,
    },
    Reference {
        name: "settings_memory",
        scenario: "default",
        arrange: show_memory,
    },
    Reference {
        name: "settings_sandbox",
        scenario: "default",
        arrange: show_sandbox,
    },
    Reference {
        name: "settings_providers",
        scenario: "default",
        arrange: show_providers,
    },
    Reference {
        name: "settings_providers_detail",
        scenario: "default",
        arrange: show_provider_detail,
    },
    Reference {
        name: "settings_channels",
        scenario: "default",
        arrange: show_channels,
    },
    Reference {
        name: "settings_integrations_installed",
        scenario: "integrations_states",
        arrange: show_integrations,
    },
    Reference {
        name: "settings_integrations_available",
        scenario: "integrations_states",
        arrange: show_integrations_available,
    },
    Reference {
        name: "settings_integrations_detail",
        scenario: "integrations_states",
        arrange: show_integration_detail,
    },
    Reference {
        name: "settings_meetings_signed_in",
        scenario: "default",
        arrange: show_meetings,
    },
    Reference {
        name: "settings_meetings_signed_out",
        scenario: "meetings_signed_out",
        arrange: show_meetings,
    },
    Reference {
        name: "settings_computer_installed",
        scenario: "default",
        arrange: show_computer,
    },
    Reference {
        name: "settings_computer_not_installed",
        scenario: "computer_states",
        arrange: show_computer,
    },
    Reference {
        name: "settings_computer_wayland",
        scenario: "computer_wayland",
        arrange: show_computer,
    },
    Reference {
        name: "settings_permissions",
        scenario: "default",
        arrange: show_permissions,
    },
    Reference {
        name: "settings_voice",
        scenario: "default",
        arrange: show_voice,
    },
    Reference {
        name: "settings_model_picker",
        scenario: "default",
        arrange: show_model_picker,
    },
    Reference {
        name: "secret_dialog",
        scenario: "default",
        arrange: show_secret_dialog,
    },
    // Both stores get a capture of their own, because the row differs in more
    // than one word between them: the file store carries a caption about who
    // can read the file and a button back to the keyring, and the keyring
    // carries neither. The default scenario does show the row in Permissions,
    // but it sits below the fold there, so the keyring case needs the same
    // scrolled arrangement the file case uses to be reviewable at all.
    Reference {
        name: "settings_secret_store_keyring",
        scenario: "default",
        arrange: show_secret_store,
    },
    Reference {
        name: "settings_secret_store_file",
        scenario: "secret_store_file",
        arrange: show_secret_store,
    },
    Reference {
        name: "secret_keyring_locked",
        scenario: "default",
        arrange: show_keyring_locked,
    },
    Reference {
        name: "secret_store_absent",
        scenario: "default",
        arrange: show_store_absent,
    },
    Reference {
        name: "consent_dialog",
        scenario: "integrations_states",
        arrange: show_consent_dialog,
    },
    Reference {
        name: "settings_external_change",
        scenario: "external_change",
        arrange: show_memory_after_refused_write,
    },
    Reference {
        name: "restart_dialog",
        scenario: "restart_pending",
        arrange: show_restart_dialog,
    },
    Reference {
        name: "recovery",
        scenario: "unreadable",
        arrange: show_recovery_after_refused_write,
    },
    // The Setup assistant. Every state is a scenario of its own, because the
    // screen a person meets is decided by what the daemon reports and by what
    // the command line answers rather than by anything this application holds.
    Reference {
        name: "welcome",
        scenario: "onboarding_welcome",
        arrange: show_setup,
    },
    Reference {
        name: "starting_running",
        scenario: "onboarding_starting",
        arrange: show_starting,
    },
    Reference {
        name: "starting_failed_linger",
        scenario: "onboarding_linger_denied",
        arrange: show_starting,
    },
    Reference {
        name: "boot_failed",
        scenario: "onboarding_boot_failed",
        arrange: show_starting,
    },
    Reference {
        name: "connect_ai",
        scenario: "onboarding_connect_ai",
        arrange: show_setup,
    },
    Reference {
        name: "connect_ai_waiting",
        scenario: "onboarding_connect_ai",
        arrange: show_sign_in,
    },
    Reference {
        name: "about_you",
        scenario: "onboarding_about_you",
        arrange: show_setup,
    },
    Reference {
        name: "about_you_refused",
        scenario: "onboarding_refused_personalization",
        arrange: show_refused_personalization,
    },
    Reference {
        name: "applying_no_restart",
        scenario: "onboarding_no_restart",
        arrange: show_applying,
    },
    Reference {
        name: "applying_restart",
        scenario: "onboarding_restart_needed",
        arrange: show_setup,
    },
    Reference {
        name: "ready",
        scenario: "onboarding_ready",
        arrange: show_setup,
    },
    Reference {
        name: "skew_attention",
        scenario: "onboarding_skew",
        arrange: show_home,
    },
];

/// The capture directory named on the command line, where one was.
pub fn directory_from(arguments: &[String]) -> Option<PathBuf> {
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        if argument == FLAG {
            return arguments.next().map(PathBuf::from);
        }
        if let Some(directory) = argument.strip_prefix("--capture=") {
            return Some(PathBuf::from(directory));
        }
    }
    None
}

/// Take every reference state this scenario can show, then quit.
pub fn arm(application: &FermixApplication, directory: PathBuf) {
    application.connect_activate(move |application| {
        let application = application.clone();
        let directory = directory.clone();

        glib::spawn_future_local(async move {
            if let Err(reason) = run(&application, &directory).await {
                glib::g_warning!("fermix-desktop", "the capture did not finish: {reason}");
            }
            application.quit();
        });
    });
}

async fn run(application: &FermixApplication, directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;

    // A still frame cannot show a transition, and a frame taken during one is a
    // picture of two states at once. Asking the platform for no motion is the
    // same switch a person can throw, and every surface already follows it.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }

    let window = wait_for_window(application).await?;
    window.set_default_size(
        metrics::WINDOW_DEFAULT_WIDTH,
        metrics::WINDOW_DEFAULT_HEIGHT,
    );

    let scenario = std::env::var("FIXTURE_DAEMON_SCENARIO").unwrap_or_else(|_| "default".into());
    let scheme = if adw::StyleManager::default().is_dark() {
        "dark"
    } else {
        "light"
    };

    // The first read has to land before anything is worth looking at.
    glib::timeout_future(SETTLE).await;

    for reference in REFERENCES
        .iter()
        .filter(|reference| reference.scenario == scenario)
    {
        (reference.arrange)(&window);
        glib::timeout_future(SETTLE).await;

        let path = directory.join(format!("{}-{scheme}.png", reference.name));
        take(&window, &path)?;
        println!("{}", path.display());

        close_dialogs(&window);
        glib::timeout_future(Duration::from_millis(100)).await;
    }

    Ok(())
}

async fn wait_for_window(application: &FermixApplication) -> Result<FermixWindow, String> {
    for _ in 0..WINDOW_TRIES {
        if let Some(window) = application
            .active_window()
            .and_then(|window| window.downcast::<FermixWindow>().ok())
        {
            return Ok(window);
        }
        glib::timeout_future(Duration::from_millis(100)).await;
    }

    Err("no window was presented".to_string())
}

/// One frame of the real window, through the real renderer.
fn take(window: &FermixWindow, path: &Path) -> Result<(), String> {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let image = paintable.current_image();

    let width = f64::from(image.intrinsic_width().max(1));
    let height = f64::from(image.intrinsic_height().max(1));

    let snapshot = gtk::Snapshot::new();
    image.snapshot(&snapshot, width, height);
    let node = snapshot
        .to_node()
        .ok_or_else(|| "the snapshot carried no node".to_string())?;

    let renderer = window
        .renderer()
        .ok_or_else(|| "the window has no renderer yet".to_string())?;
    let texture = renderer.render_texture(&node, None);

    std::fs::write(path, texture.save_to_png_bytes()).map_err(|error| error.to_string())
}

/// Close whatever a capture opened, so the next one starts on a clean window.
/// Bounded, because a dialog that will not close must not stop the run.
fn close_dialogs(window: &FermixWindow) {
    for _ in 0..4 {
        let Some(dialog) = topmost_dialog(window) else {
            return;
        };
        dialog.force_close();
    }
}

fn topmost_dialog(window: &FermixWindow) -> Option<adw::Dialog> {
    find_dialog(window.upcast_ref::<gtk::Widget>())
}

fn find_dialog(widget: &gtk::Widget) -> Option<adw::Dialog> {
    if let Some(dialog) = widget.downcast_ref::<adw::Dialog>() {
        return Some(dialog.clone());
    }

    let mut child = widget.first_child();
    while let Some(candidate) = child {
        if let Some(found) = find_dialog(&candidate) {
            return Some(found);
        }
        child = candidate.next_sibling();
    }
    None
}

// ---- The arrangements ---------------------------------------------------

fn show_home(window: &FermixWindow) {
    activate(window, "win.home");
}

fn show_doctor(window: &FermixWindow) {
    activate(window, "win.doctor");
}

/// The failed row with its evidence open, which is the state the redlines ask
/// to see rather than the row with the evidence closed.
fn show_doctor_evidence(window: &FermixWindow) {
    activate(window, "win.doctor");
    expand_first_expander(window.upcast_ref::<gtk::Widget>());
}

fn show_logs(window: &FermixWindow) {
    activate(window, "win.logs");
}

fn show_memory(window: &FermixWindow) {
    show_pane(window, SettingsPane::Memory);
}

fn show_sandbox(window: &FermixWindow) {
    show_pane(window, SettingsPane::Sandbox);
}

fn show_providers(window: &FermixWindow) {
    show_pane(window, SettingsPane::Providers);
}

/// One provider's own page, which is where everything about it lives.
fn show_provider_detail(window: &FermixWindow) {
    show_pane(window, SettingsPane::Providers);
    activate_row_titled(window, "Anthropic");
}

fn show_channels(window: &FermixWindow) {
    show_pane(window, SettingsPane::Channels);
}

fn show_integrations(window: &FermixWindow) {
    show_pane(window, SettingsPane::Integrations);
}

/// The Available filter, which is the second of the four.
fn show_integrations_available(window: &FermixWindow) {
    show_pane(window, SettingsPane::Integrations);
    press_filter(window, crate::copy::Key::IntegrationsFilterAvailable);
}

/// One integration's own page, on a row the daemon left waiting for a person.
///
/// The filter is pressed first, because the capture before this one may have
/// left another showing and the row has to be in the list to be opened.
fn show_integration_detail(window: &FermixWindow) {
    show_pane(window, SettingsPane::Integrations);
    press_filter(window, crate::copy::Key::IntegrationsFilterInstalled);
    activate_row_titled(window, "GitHub");
}

/// Meetings, scrolled to the sign-in row: the state this capture is named for
/// is below the fold at the default window size, and a reference state nobody
/// can see in its own picture is not a reference.
fn show_meetings(window: &FermixWindow) {
    show_pane(window, SettingsPane::Meetings);
    scroll_to_end(window);
}

fn show_computer(window: &FermixWindow) {
    show_pane(window, SettingsPane::Computer);
}

fn show_permissions(window: &FermixWindow) {
    show_pane(window, SettingsPane::Permissions);
}

fn show_voice(window: &FermixWindow) {
    show_pane(window, SettingsPane::Voice);
}

/// The listing the wire does not carry the whole of, opened from the row that
/// owns the value.
fn show_model_picker(window: &FermixWindow) {
    show_provider_detail(window);
    click_button_labelled(
        window,
        &crate::copy::text(crate::copy::Key::ProviderChooseModel),
    );
}

/// The one dialog a secret is typed in, opened from the row that owns the slot.
fn show_secret_dialog(window: &FermixWindow) {
    show_pane(window, SettingsPane::Providers);
    click_button_labelled(window, &crate::copy::text(crate::copy::Key::ProviderAddKey));
}

/// Where secrets live, and the way home, which sit below the rights.
///
/// Scrolled to the foot: the row is the last thing in the pane, and a capture
/// that cuts it in half says nothing about the one control it carries.
fn show_secret_store(window: &FermixWindow) {
    show_permissions(window);
    scroll_to_end(window);
}

/// The keyring is here and locked, which is the one refusal with a way through.
///
/// The dialog is built and presented directly rather than driven through a
/// refused write: the words and the buttons are what a capture is for, and the
/// routing that chooses between these two is covered by its own tests.
fn show_keyring_locked(window: &FermixWindow) {
    present_store_refusal(window, StoreRefusal::KeyringLocked);
}

/// No keyring at all, where the private file is the whole of what is offered.
fn show_store_absent(window: &FermixWindow) {
    present_store_refusal(window, StoreRefusal::NoKeyring);
}

/// One store refusal, on this window.
fn present_store_refusal(window: &FermixWindow, refusal: StoreRefusal) {
    let sentence = Sentence {
        code: Some("fixture".to_string()),
        text: String::new(),
        reason: None,
    };
    crate::ui::settings::dialogs::secret::store_dialog(refusal, false, &sentence)
        .present(Some(window.upcast_ref::<gtk::Widget>()));
}

/// The consent a plugin is installed under, which is what a switch-on asks for.
fn show_consent_dialog(window: &FermixWindow) {
    show_pane(window, SettingsPane::Integrations);
    press_filter(window, crate::copy::Key::IntegrationsFilterAvailable);
    press_switch_in_row_titled(window, "Obsidian");
}

/// Scroll the pane being shown to its foot, once it has been laid out.
///
/// After a turn of the loop rather than now: a pane that has just been put on
/// screen has no height yet, and scrolling to the end of nothing is scrolling
/// nowhere. The settle this capture already waits covers the delay.
fn scroll_to_end(window: &FermixWindow) {
    let window = window.clone();

    glib::timeout_add_local_once(LAID_OUT, move || {
        // Inside the detail, because the sidebar has a scroller of its own and
        // it comes first in the tree: scrolling that one moves the pane list
        // and leaves the pane exactly where it was.
        let detail = window.presentation().detail();
        let found = find(&detail, &mut |widget| {
            widget
                .downcast_ref::<gtk::ScrolledWindow>()
                .is_some_and(|scroller| scroller.is_mapped())
        });

        let Some(scroller) = found.and_then(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
        else {
            return;
        };
        let adjustment = scroller.vadjustment();
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
    });
}

/// How long a pane is given to be laid out before it is scrolled.
const LAID_OUT: Duration = Duration::from_millis(150);

/// Activate the row with one title, wherever it is in the tree.
fn activate_row_titled(window: &FermixWindow, title: &str) {
    let found = find(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        widget
            .downcast_ref::<adw::ActionRow>()
            .is_some_and(|row| row.title() == title)
    });

    if let Some(row) = found.and_then(|widget| widget.downcast::<adw::ActionRow>().ok()) {
        row.emit_activate();
    }
}

/// Press the switch inside the row with one title.
fn press_switch_in_row_titled(window: &FermixWindow, title: &str) {
    let found = find(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        widget
            .downcast_ref::<adw::ActionRow>()
            .is_some_and(|row| row.title() == title)
    });

    let Some(row) = found else {
        return;
    };
    let Some(switch) = find(&row, &mut |widget| widget.is::<gtk::Switch>()) else {
        return;
    };
    if let Ok(switch) = switch.downcast::<gtk::Switch>() {
        switch.set_active(true);
    }
}

/// Press one of the counted filters, by the word it carries.
///
/// By its word rather than by its place: the filters are one row of toggles in
/// a window with others in it, and a capture that pressed the wrong control
/// would be a picture of a state nobody asked for.
fn press_filter(window: &FermixWindow, filter: crate::copy::Key) {
    let word = crate::copy::text(filter);
    let found = find(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        widget
            .downcast_ref::<gtk::ToggleButton>()
            .is_some_and(|button| {
                button
                    .label()
                    .is_some_and(|label| label.starts_with(word.as_str()))
            })
    });

    if let Some(button) = found.and_then(|widget| widget.downcast::<gtk::ToggleButton>().ok()) {
        button.set_active(true);
    }
}

/// Click the button carrying one label.
fn click_button_labelled(window: &FermixWindow, label: &str) {
    let found = find(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().is_some_and(|text| text == label))
    });

    if let Some(button) = found.and_then(|widget| widget.downcast::<gtk::Button>().ok()) {
        button.emit_clicked();
    }
}

/// The first descendant that answers a question, in tree order.
fn find(widget: &gtk::Widget, wanted: &mut dyn FnMut(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }

    let mut child = widget.first_child();
    while let Some(candidate) = child {
        if let Some(found) = find(&candidate, wanted) {
            return Some(found);
        }
        child = candidate.next_sibling();
    }
    None
}

/// Recovery, with the parser's own sentence on it.
///
/// The daemon reports the unreadable state the moment it is asked, and the
/// window routes there on its own; the sentence itself arrives with the first
/// refused write, so the capture makes one and then asks for the surface again.
fn show_recovery_after_refused_write(window: &FermixWindow) {
    let settings = window.settings();
    let window = window.clone();

    glib::spawn_future_local(async move {
        settings
            .apply(
                "memory",
                "review_interval_hours",
                crate::management::types::SettingValue::Number(12.0),
            )
            .await;
        window.show_recovery();
    });
}

/// The banner only appears once a write has actually been refused, which is
/// what a person meets: the state is not a property of the pane.
fn show_memory_after_refused_write(window: &FermixWindow) {
    show_pane(window, SettingsPane::Memory);

    let settings = window.settings();
    glib::spawn_future_local(async move {
        settings
            .apply(
                "memory",
                "review_interval_hours",
                crate::management::types::SettingValue::Number(12.0),
            )
            .await;
    });
}

// ---- The Setup assistant -------------------------------------------------

/// The assistant, at the screen the daemon's own readiness says is owed.
fn show_setup(window: &FermixWindow) {
    window.show_setup();
}

/// The ladder, running. What it is running is whatever the command line and the
/// scenario answer: a refusal lands on the state its own code names, and a web
/// door that never answers keeps the ladder on the row that is asking it.
fn show_starting(window: &FermixWindow) {
    window.show_setup();
    window.assistant().model().begin();
}

/// The browser hop, as a person meets it: the dialog is open and the daemon is
/// still waiting.
fn show_sign_in(window: &FermixWindow) {
    window.show_setup();
    click_button_labelled(window, &crate::copy::text(crate::copy::Key::ProviderSignIn));
}

/// About you, after the daemon refused what it wrote.
fn show_refused_personalization(window: &FermixWindow) {
    window.show_setup();
    window.assistant().model().start_applying(true);
}

/// Applying, after the write landed and before whatever is still owed is
/// answered.
fn show_applying(window: &FermixWindow) {
    window.show_setup();
    window.assistant().model().start_applying(true);
}

fn show_restart_dialog(window: &FermixWindow) {
    show_pane(window, SettingsPane::Memory);
    activate(window, "win.restart");
}

fn show_pane(window: &FermixWindow, pane: SettingsPane) {
    window.settings().select_pane(pane);
    activate(window, "win.settings");

    // A pane with pages of its own may have been left on one by the capture
    // before this. Bounded, because a page that will not close must not stop
    // the run.
    let presentation = window.presentation();
    for _ in 0..4 {
        if !presentation.pop() {
            break;
        }
    }
}

fn activate(window: &FermixWindow, action: &str) {
    let _ = WidgetExt::activate_action(window, action, None);
}

fn expand_first_expander(widget: &gtk::Widget) -> bool {
    if let Some(expander) = widget.downcast_ref::<adw::ExpanderRow>() {
        expander.set_expanded(true);
        return true;
    }

    let mut child = widget.first_child();
    while let Some(candidate) = child {
        if expand_first_expander(&candidate) {
            return true;
        }
        child = candidate.next_sibling();
    }
    false
}
