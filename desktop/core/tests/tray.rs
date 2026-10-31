//! The tray icon: its glyph, the status line at the top of its menu, and the
//! rows under it, as the macOS status item has them (StatusMenu.swift,
//! CommandTable.statusItem). The words are Home's, so the two never disagree.

use fermix_client::model::SetupState;
use fermix_client::service::BackgroundSwitch;
use fermix_client::tray::{tray_view, Command, Daemon, Glyph, Item, Row, TrayFacts};
use fermix_client::view::DaemonProblem;
use serde_json::json;

fn state(status: &str, restart: bool) -> SetupState {
    serde_json::from_value(json!({
        "readiness": {"status": status, "failures": []},
        "restart": {"required": restart, "reasons": []},
        "providers": [],
        "channels": [],
        "features": {"voice": false, "voice_notes": false, "meetings": false, "computer_use": false},
        "coexistence": {"config_state": "clear"}
    }))
    .unwrap()
}

fn background(on: bool, sensitive: bool) -> BackgroundSwitch {
    BackgroundSwitch {
        on,
        sensitive,
        note: String::new(),
    }
}

fn facts<'a>(daemon: Daemon<'a>, switch: &'a BackgroundSwitch) -> TrayFacts<'a> {
    TrayFacts {
        daemon,
        pet_shown: false,
        background: switch,
        update: "Check for Updates…".into(),
        restart_pending: false,
    }
}

fn items(rows: &[Row]) -> Vec<&Item> {
    rows.iter()
        .filter_map(|row| match row {
            Row::Item(item) => Some(item),
            _ => None,
        })
        .collect()
}

fn item(rows: &[Row], command: Command) -> &Item {
    items(rows)
        .into_iter()
        .find(|item| item.command == command)
        .unwrap_or_else(|| panic!("no {command:?} row"))
}

#[test]
fn a_running_fermix_shows_the_mark_and_how_long_it_has_run() {
    let ready = state("ready", false);
    let switch = background(true, true);
    let view = tray_view(&facts(
        Daemon::Up {
            state: &ready,
            uptime_ms: Some(61_897_905),
        },
        &switch,
    ));
    assert_eq!(view.glyph, Glyph::Running);
    assert_eq!(view.status, "Running for 17 hours, 11 minutes");
    let unreported = tray_view(&facts(
        Daemon::Up {
            state: &ready,
            uptime_ms: None,
        },
        &switch,
    ));
    assert_eq!(unreported.status, "Running");
}

#[test]
fn the_menu_reads_as_the_macos_status_menu() {
    let ready = state("ready", false);
    let switch = background(true, true);
    let view = tray_view(&facts(
        Daemon::Up {
            state: &ready,
            uptime_ms: None,
        },
        &switch,
    ));
    let drawn: Vec<String> = view
        .rows
        .iter()
        .map(|row| match row {
            Row::Status(text) => format!("[{text}]"),
            Row::Separator => "---".into(),
            Row::Item(item) => item.label.clone(),
        })
        .collect();
    assert_eq!(
        drawn,
        [
            "[Running]",
            "---",
            "Open Fermix",
            "Settings",
            "Run Doctor",
            "Check for Updates…",
            "---",
            "Restart Fermix…",
            "Show Pet",
            "Run in the Background",
            "---",
            "Quit",
        ]
    );
    assert!(items(&view.rows).iter().all(|item| item.enabled));
}

#[test]
fn starting_is_the_lighter_mark() {
    let switch = background(false, false);
    let reading = tray_view(&facts(Daemon::Reading, &switch));
    assert_eq!(reading.glyph, Glyph::Starting);
    assert_eq!(reading.status, "Reading from Fermix…");
    let waking = tray_view(&facts(Daemon::Waking, &switch));
    assert_eq!(waking.glyph, Glyph::Starting);
    assert_eq!(waking.status, "Starting…");
    for view in [reading, waking] {
        assert!(
            !item(&view.rows, Command::Restart).enabled,
            "nothing to restart yet"
        );
    }
}

#[test]
fn anything_to_look_at_carries_the_badge() {
    let switch = background(true, true);
    let not_running = DaemonProblem::NotRunning;
    let down = tray_view(&facts(Daemon::Down(&not_running), &switch));
    assert_eq!(down.glyph, Glyph::Attention);
    assert_eq!(down.status, "Not running");
    let setup = state("setup_required", false);
    let unfinished = tray_view(&facts(
        Daemon::Up {
            state: &setup,
            uptime_ms: Some(60_000),
        },
        &switch,
    ));
    assert_eq!(unfinished.glyph, Glyph::Attention);
    assert_eq!(unfinished.status, "Setup required");
    let updated = state("ready", true);
    let pending = tray_view(&facts(
        Daemon::Up {
            state: &updated,
            uptime_ms: Some(60_000),
        },
        &switch,
    ));
    assert_eq!(pending.glyph, Glyph::Attention);
    assert_eq!(pending.status, "Restart to finish updating");
}

#[test]
fn the_restart_row_offers_what_home_offers() {
    let switch = background(true, true);
    let not_running = DaemonProblem::NotRunning;
    let start = tray_view(&facts(Daemon::Down(&not_running), &switch));
    let row = &start.rows[7];
    assert_eq!(
        row,
        &Row::Item(Item {
            command: Command::StartService,
            label: "Start Fermix".into(),
            enabled: true,
            checked: None,
        })
    );
    let silent = DaemonProblem::NotResponding;
    let stuck = tray_view(&facts(Daemon::Down(&silent), &switch));
    let restart = item(&stuck.rows, Command::RestartService);
    assert_eq!(
        (restart.label.as_str(), restart.enabled),
        ("Restart Fermix", true)
    );
    let away = DaemonProblem::PrimaryUnreachable("The server fermix-vm is unreachable.".into());
    let unreachable = tray_view(&facts(Daemon::Down(&away), &switch));
    assert_eq!(unreachable.status, "Server unreachable");
    assert!(
        !item(&unreachable.rows, Command::Restart).enabled,
        "Fermix here runs: restarting it reaches no server sooner"
    );
    let too_old = DaemonProblem::UpdateNeeded("Update the fermix package.".into());
    let update = tray_view(&facts(Daemon::Down(&too_old), &switch));
    assert_eq!(update.status, "Update needed");
    assert!(
        !item(&update.rows, Command::Restart).enabled,
        "a restart cannot update anything"
    );
}

#[test]
fn the_pet_and_the_background_service_are_ticked_when_on() {
    let ready = state("ready", false);
    let on = background(true, true);
    let mut shown = facts(
        Daemon::Up {
            state: &ready,
            uptime_ms: None,
        },
        &on,
    );
    shown.pet_shown = true;
    let view = tray_view(&shown);
    assert_eq!(item(&view.rows, Command::TogglePet).checked, Some(true));
    assert_eq!(
        item(&view.rows, Command::ToggleBackground).checked,
        Some(true)
    );
    let unreachable = background(false, false);
    let off = tray_view(&facts(
        Daemon::Up {
            state: &ready,
            uptime_ms: None,
        },
        &unreachable,
    ));
    let service = item(&off.rows, Command::ToggleBackground);
    assert_eq!((service.checked, service.enabled), (Some(false), false));
    assert_eq!(item(&off.rows, Command::TogglePet).checked, Some(false));
}

#[test]
fn a_row_that_may_ask_a_question_brings_the_window_up_first() {
    for command in [
        Command::OpenFermix,
        Command::Settings,
        Command::Doctor,
        Command::Restart,
        Command::ToggleBackground,
        Command::Update,
    ] {
        assert!(command.needs_window(), "{command:?}");
    }
    for command in [
        Command::StartService,
        Command::RestartService,
        Command::TogglePet,
        Command::Quit,
    ] {
        assert!(!command.needs_window(), "{command:?}");
    }
}

#[test]
fn each_glyph_is_an_icon_the_package_exports() {
    assert_eq!(Glyph::Running.icon_name(), "io.tezra.Fermix-tray-symbolic");
    assert_eq!(
        Glyph::Starting.icon_name(),
        "io.tezra.Fermix-tray-starting-symbolic"
    );
    assert_eq!(
        Glyph::Attention.icon_name(),
        "io.tezra.Fermix-tray-attention-symbolic"
    );
    assert_eq!(Glyph::Running.label(), "Fermix is running");
    assert_eq!(Glyph::Starting.label(), "Fermix is starting");
    assert_eq!(Glyph::Attention.label(), "Fermix needs attention");
}

#[test]
fn the_update_row_says_what_the_updater_found() {
    let ready = state("ready", false);
    let switch = background(true, true);
    let mut found = facts(
        Daemon::Up {
            state: &ready,
            uptime_ms: None,
        },
        &switch,
    );
    found.update = "Update to 0.4.0…".into();
    let view = tray_view(&found);
    let update = item(&view.rows, Command::Update);
    assert_eq!(update.label, "Update to 0.4.0…");
    assert!(update.enabled);
    let not_running = DaemonProblem::NotRunning;
    let down = tray_view(&facts(Daemon::Down(&not_running), &switch));
    assert!(
        item(&down.rows, Command::Update).enabled,
        "an update can be looked for with Fermix stopped"
    );
}

#[test]
fn an_installed_update_still_running_old_code_asks_for_the_restart() {
    let ready = state("ready", false);
    let switch = background(true, true);
    let mut pending = facts(
        Daemon::Up {
            state: &ready,
            uptime_ms: Some(60_000),
        },
        &switch,
    );
    pending.restart_pending = true;
    let view = tray_view(&pending);
    assert_eq!(view.glyph, Glyph::Attention);
    assert_eq!(view.status, "Restart to finish updating");
    let not_running = DaemonProblem::NotRunning;
    let mut down = facts(Daemon::Down(&not_running), &switch);
    down.restart_pending = true;
    assert_eq!(
        tray_view(&down).status,
        "Not running",
        "a daemon that is down says so first"
    );
}
