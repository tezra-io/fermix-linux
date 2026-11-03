//! Meetings and Computer: the notetaker's sign-in state and the helper's probe,
//! read as the daemon reports them and never inferred (M38 §5.7, §8.6).

use fermix_client::capabilities::{
    checked_words, linux_probe, Ability, LinuxProbe, ProbeNote, ProbeRow, RowStatus,
};
use fermix_client::capabilities::{meetbot_state, verdict, ComputerPermissions, MeetbotState};
use fermix_client::doctor::Tone;
use fermix_client::management::decode_response;
use fermix_client::model::{DetectResult, DetectRow};
use fermix_client::update::view::{note_spans, Span};
use serde::de::DeserializeOwned;
use serde_json::Value;

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
/// The engine's Linux golden, ahead of the vendored contract, and hand answers in its shape.
const LINUX: &str = include_str!("fixtures/computer-use/permissions.jsonl");

fn decode<T: DeserializeOwned>(name: &str) -> T {
    let found = SUCCESS
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["name"] == name)
        .unwrap();
    let response = &found["response"];
    let id = response["request_id"].as_str().unwrap();
    let result = decode_response(&serde_json::to_vec(response).unwrap(), id).unwrap();
    serde_json::from_value(result).unwrap()
}

fn meetbot(present: bool, signed_in: Option<bool>, detail: Option<&str>) -> DetectRow {
    DetectRow {
        target: "meetbot".into(),
        present,
        detail: detail.map(str::to_owned),
        signed_in,
    }
}

#[test]
fn the_notetaker_sign_in_is_read_from_its_detection_row() {
    let detected: DetectResult = decode("setup_detect");
    let row = detected.results.iter().find(|r| r.target == "meetbot");
    assert_eq!(
        meetbot_state(row),
        MeetbotState::SignedIn("Signed in to Google".into())
    );
}

#[test]
fn only_an_explicit_false_is_signed_out_and_nothing_else_is_guessed() {
    let out = meetbot(true, Some(false), Some("Not signed in to Google"));
    assert_eq!(
        meetbot_state(Some(&out)),
        MeetbotState::SignedOut(Some("Not signed in to Google".into()))
    );
    assert_eq!(
        meetbot_state(Some(&meetbot(false, None, None))),
        MeetbotState::Absent
    );
    assert_eq!(
        meetbot_state(Some(&meetbot(true, None, None))),
        MeetbotState::Unanswered
    );
    assert_eq!(meetbot_state(None), MeetbotState::Unanswered);
}

#[test]
fn a_probe_verdict_says_available_not_available_or_not_checked() {
    let probe: ComputerPermissions = decode("computer_use_permissions_get");
    assert!(probe.installed);
    assert_eq!(
        verdict(probe.screen_capture, probe.probed_at.as_deref()),
        "Available"
    );
    assert_eq!(
        verdict(probe.input_control, probe.probed_at.as_deref()),
        "Not available"
    );
    assert_eq!(verdict(true, None), "Not checked yet");
}

#[test]
fn a_running_job_says_what_it_is_doing_in_words() {
    use fermix_client::capabilities::phase_words;
    assert_eq!(
        phase_words(Some("sidecar_downloading")),
        "Downloading the helper…"
    );
    assert_eq!(
        phase_words(Some("downloading")),
        "Downloading the notetaker's browser…"
    );
    assert_eq!(
        phase_words(Some("awaiting_signin")),
        "Finish signing in to Google in the notetaker's browser window."
    );
    assert_eq!(phase_words(Some("brand_new_phase")), "Working…");
    assert_eq!(phase_words(None), "Working…");
}

#[test]
fn the_sign_in_says_it_opens_the_notetakers_own_browser() {
    use fermix_client::capabilities::MEETBOT_SIGN_IN_NOTE;
    assert!(MEETBOT_SIGN_IN_NOTE.contains("the notetaker's own browser, separate from yours"));
    assert!(MEETBOT_SIGN_IN_NOTE.contains("under that account's name"));
}

const GOLDEN: &str = "computer_use_permissions_get_linux";
const PIXELS: &str = "Screenshots, pointer and keys";
const WINDOW: &str = "Work inside one window";

fn answer(name: &str) -> ComputerPermissions {
    let found = LINUX
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no answer named {name}"));
    serde_json::from_value(found["result"].clone()).unwrap()
}

/// Every answer in the fixtures, the engine's golden first.
fn every_answer() -> Vec<(String, ComputerPermissions)> {
    LINUX
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .map(|v| {
            let name = v["name"].as_str().unwrap().to_owned();
            (name, serde_json::from_value(v["result"].clone()).unwrap())
        })
        .collect()
}

fn drawn(name: &str) -> LinuxProbe {
    linux_probe(&answer(name)).unwrap_or_else(|| panic!("{name} is drawn as Linux"))
}

fn with_halves(name: &str, screen_capture: bool, input_control: bool) -> LinuxProbe {
    let mut permissions = answer(name);
    permissions.screen_capture = screen_capture;
    permissions.input_control = input_control;
    linux_probe(&permissions).unwrap()
}

fn detail(permissions: &ComputerPermissions, row: &str) -> String {
    let rows: &[ProbeRow] = permissions.rows.as_deref().unwrap();
    rows.iter().find(|r| r.name == row).unwrap().detail.clone()
}

fn states(probe: &LinuxProbe) -> Vec<(&'static str, &'static str, Tone)> {
    probe
        .abilities
        .iter()
        .map(|a| (a.title, a.state, a.tone))
        .collect()
}

fn note_titles(probe: &LinuxProbe) -> Vec<&str> {
    probe.notes.iter().map(|n| n.title.as_str()).collect()
}

fn plain(text: &str) -> Vec<Span> {
    vec![Span::Text(text.to_owned())]
}

fn no_backtick_shown(spans: &[Span]) -> bool {
    !spans
        .iter()
        .any(|s| matches!(s, Span::Text(t) if t.contains('`')))
}

#[test]
fn x11_names_its_pixel_route_and_offers_one_window_only_behind_its_switch() {
    let permissions = answer("linux_x11");
    let probe = drawn("linux_x11");
    let bus = detail(&permissions, "accessibility bus");
    assert_eq!(
        probe.abilities,
        vec![
            Ability {
                title: "Controls",
                subtitle: plain(&format!("Accessibility bus: {bus}")),
                state: "Available",
                tone: Tone::Good,
            },
            Ability {
                title: PIXELS,
                subtitle: plain("Pixel route: X11, through this session's X server"),
                state: "Available",
                tone: Tone::Good,
            },
            // The answer carries neither the switch nor the helper's own support, so X11 says
            // where it can work, never that it does.
            Ability {
                title: WINDOW,
                subtitle: plain("Its switch is above. A badge marks the window Fermix works in."),
                state: "On X11",
                tone: Tone::Quiet,
            },
        ]
    );
    assert_eq!(
        note_titles(&probe),
        vec![
            "ATK bridge",
            "Qt applications",
            "Chromium-based applications"
        ]
    );
    assert_eq!(probe.checked.as_deref(), Some("2026-11-01 12:00:00"));
}

#[test]
fn gnome_on_wayland_with_both_halves_names_the_portal_in_the_daemons_words() {
    let probe = drawn("linux_gnome_wayland");
    assert_eq!(
        states(&probe),
        vec![
            ("Controls", "Available", Tone::Good),
            (PIXELS, "Available", Tone::Good),
            (WINDOW, "Not available", Tone::Quiet),
        ]
    );
    // The portal's sentence is read from the engine's golden, never typed here, so this hand
    // answer must carry the golden's own route row.
    let portal = detail(&answer(GOLDEN), "pixel route");
    assert!(portal.starts_with("GNOME's desktop portal"), "{portal}");
    assert_eq!(
        probe.abilities[1].subtitle,
        plain(&format!("Pixel route: {portal}"))
    );
    assert_eq!(probe.abilities[2].subtitle, plain("Only on X11."));
}

#[test]
fn the_engines_stored_no_leaves_keys_and_typing_and_offers_its_reset_to_copy() {
    let permissions = answer(GOLDEN);
    assert!(!permissions.screen_capture && permissions.input_control);
    let probe = drawn(GOLDEN);
    assert_eq!(
        states(&probe),
        vec![
            ("Controls", "Available", Tone::Good),
            (PIXELS, "Partly", Tone::Warn),
            (WINDOW, "Not available", Tone::Quiet),
        ]
    );
    // No route line: the portal is the route, but the stored no keeps it from any screenshot.
    assert_eq!(
        probe.abilities[1].subtitle,
        plain("Keys and typing work, but screenshots do not, so Fermix cannot see where to click.")
    );
    assert_eq!(
        note_titles(&probe),
        vec![
            "ATK bridge",
            "Qt applications",
            "Chromium-based applications",
            "Stored screenshot answer"
        ]
    );
    let (text, command) = detail(&permissions, "portal screenshot")
        .split_once('`')
        .map(|(t, rest)| (t.to_owned(), rest.trim_end_matches('`').to_owned()))
        .unwrap();
    assert_eq!(
        probe.notes[3],
        ProbeNote {
            title: "Stored screenshot answer".into(),
            detail: vec![Span::Text(text), Span::Code(command)],
            selectable: true,
            attention: true,
        }
    );
    let quiet = probe.notes[..3]
        .iter()
        .all(|n| !n.selectable && !n.attention);
    assert!(quiet);
    assert_eq!(probe.checked.as_deref(), Some("2026-08-19 12:00:00"));
}

#[test]
fn a_desktop_without_a_pixel_route_works_by_control_and_says_why_quietly() {
    let probe = drawn("linux_kde_controls_only");
    assert_eq!(
        states(&probe),
        vec![
            ("Controls", "Available", Tone::Good),
            (PIXELS, "Not available", Tone::Quiet),
            (WINDOW, "Not available", Tone::Quiet),
        ]
    );
    assert_eq!(
        probe.abilities[1].subtitle,
        plain(
            "Pixel route: none: on Wayland only GNOME's desktop portal gives Fermix pixels, and \
             this desktop is KDE; Fermix works by controls alone, wherever the accessibility bus \
             answers"
        )
    );
}

#[test]
fn on_wayland_the_stored_screenshot_answer_is_a_note_only_where_the_portal_may_be_the_route() {
    // GNOME: the portal is the route, so whether and when the desktop asks is said.
    let gnome = answer("linux_gnome_wayland");
    assert_eq!(
        drawn("linux_gnome_wayland").notes.last(),
        Some(&ProbeNote {
            title: "Stored screenshot answer".into(),
            detail: plain(&detail(&gnome, "portal screenshot")),
            selectable: false,
            attention: false,
        })
    );
    // An engine from before the route row names none, so the portal may still be it.
    let mut older = answer("linux_older_engine");
    older.display_server = Some("wayland".into());
    let older = linux_probe(&older).unwrap();
    assert!(note_titles(&older).contains(&"Stored screenshot answer"));
    // KDE has no route: "the desktop asks on the first screenshot" would sit under its "none".
    assert_eq!(
        note_titles(&drawn("linux_kde_controls_only")),
        vec![
            "ATK bridge",
            "Qt applications",
            "Chromium-based applications"
        ]
    );
    // There, as off Wayland, it shows when it carries its reset or asks for a look.
    let mut refused = answer("linux_kde_controls_only");
    let golden = answer(GOLDEN);
    let stored_no = golden.rows.as_deref().unwrap().last().unwrap().clone();
    assert_eq!(stored_no.name, "portal screenshot");
    *refused.rows.as_mut().unwrap().last_mut().unwrap() = stored_no;
    let refused = linux_probe(&refused).unwrap();
    let last = refused.notes.last().unwrap();
    assert_eq!(
        (last.title.as_str(), last.selectable, last.attention),
        ("Stored screenshot answer", true, true)
    );
    // On X11 an ok stored answer without a command stays out: the X11 route never asks.
    let x11 = drawn("linux_x11");
    assert!(!note_titles(&x11).contains(&"Stored screenshot answer"));
}

#[test]
fn off_wayland_a_stored_no_still_offers_its_reset_to_copy_without_alarm() {
    let permissions = answer("linux_x11_stored_no");
    let probe = drawn("linux_x11_stored_no");
    // The X11 route never asks the portal, so the engine says the stored no as information.
    let rows = permissions.rows.as_deref().unwrap();
    let stored = rows.iter().find(|r| r.name == "portal screenshot").unwrap();
    assert_eq!(stored.status, RowStatus::Ok);
    assert_eq!(probe.abilities[1].state, "Available");
    assert_eq!(
        probe.notes.last(),
        Some(&ProbeNote {
            title: "Stored screenshot answer".into(),
            detail: note_spans(&stored.detail),
            selectable: true,
            attention: false,
        })
    );
    let reset = &probe.notes.last().unwrap().detail;
    assert!(reset
        .iter()
        .any(|s| matches!(s, Span::Code(c) if c.starts_with("gdbus call"))));
    assert!(no_backtick_shown(reset));
}

#[test]
fn without_controls_or_a_pixel_route_both_warn_and_their_literals_are_code() {
    let permissions = answer("linux_no_bus_portal_without_screencast");
    let probe = drawn("linux_no_bus_portal_without_screencast");
    assert_eq!(
        states(&probe),
        vec![
            ("Controls", "Not available", Tone::Warn),
            (PIXELS, "Not available", Tone::Warn),
            (WINDOW, "Not available", Tone::Quiet),
        ]
    );
    let bus = detail(&permissions, "accessibility bus");
    let controls = &probe.abilities[0].subtitle;
    assert_eq!(controls, &note_spans(&format!("Accessibility bus: {bus}")));
    assert!(controls.contains(&Span::Code("not_installed".into())));
    // The interface the portal lacks is the helper's literal, so it is code.
    assert_eq!(
        probe.abilities[1].subtitle,
        vec![
            Span::Text("Pixel route: none: the desktop portal on this session has no ".into()),
            Span::Code("ScreenCast".into()),
            Span::Text(
                " interface; Fermix works by controls alone, wherever the accessibility bus \
                 answers"
                    .into()
            ),
        ]
    );
    assert!(no_backtick_shown(controls));
    assert!(no_backtick_shown(&probe.abilities[1].subtitle));
    assert_eq!(note_titles(&probe), vec!["ATK bridge"]);
}

#[test]
fn a_pixel_route_this_app_does_not_know_is_named_as_the_daemon_says() {
    let permissions = answer("linux_unknown_route");
    let probe = drawn("linux_unknown_route");
    let unknown =
        plain("Pixel route: unknown: the helper named a pixel route this build does not know");
    assert_eq!(
        probe.abilities[1],
        Ability {
            title: PIXELS,
            subtitle: unknown.clone(),
            state: "Available",
            tone: Tone::Good,
        }
    );
    // Serving neither half, the route row is still what says why.
    let neither = with_halves("linux_unknown_route", false, false);
    assert_eq!(
        neither.abilities[1],
        Ability {
            title: PIXELS,
            subtitle: unknown,
            state: "Not available",
            tone: Tone::Quiet,
        }
    );
    assert_eq!(probe.abilities[2].subtitle, plain("Only on X11."));
    assert_eq!(
        note_titles(&probe),
        vec![
            "Display server",
            "ATK bridge",
            "Chromium-based applications"
        ]
    );
    assert_eq!(
        probe.notes[0],
        ProbeNote {
            title: "Display server".into(),
            detail: plain(&detail(&permissions, "display server")),
            selectable: false,
            attention: true,
        }
    );
}

#[test]
fn x11_without_xtest_takes_screenshots_but_moves_no_pointer_and_binds_no_window() {
    let permissions = answer("linux_x11_without_xtest");
    let probe = drawn("linux_x11_without_xtest");
    assert_eq!(
        states(&probe),
        vec![
            ("Controls", "Available", Tone::Good),
            (PIXELS, "Partly", Tone::Warn),
            (WINDOW, "Not available", Tone::Quiet),
        ]
    );
    assert_eq!(
        probe.abilities[1].subtitle,
        plain("Screenshots work, but the pointer and keys do not.")
    );
    assert_eq!(
        probe.abilities[2].subtitle,
        plain("It needs the screenshots, pointer and keys above.")
    );
    assert_eq!(
        note_titles(&probe),
        vec!["ATK bridge", "Chromium-based applications", "X11 pixels"]
    );
    assert_eq!(
        probe.notes[2],
        ProbeNote {
            title: "X11 pixels".into(),
            detail: plain(&detail(&permissions, "x11 pixels")),
            selectable: false,
            attention: true,
        }
    );
}

#[test]
fn one_window_on_x11_needs_both_halves() {
    let needs = Ability {
        title: WINDOW,
        subtitle: plain("It needs the screenshots, pointer and keys above."),
        state: "Not available",
        tone: Tone::Quiet,
    };
    for (screen, input) in [(true, false), (false, true), (false, false)] {
        let probe = with_halves("linux_x11", screen, input);
        assert_eq!(probe.abilities[2], needs, "screen {screen}, input {input}");
    }
    assert_eq!(drawn("linux_x11").abilities[2].state, "On X11");
    // Off X11 the reason is the display, whatever the halves say.
    let wayland = with_halves("linux_kde_controls_only", true, true);
    assert_eq!(wayland.abilities[2].subtitle, plain("Only on X11."));
}

/// Pixels with neither half and, when `controls` is false, a bus that does not answer.
fn neither_half(mut permissions: ComputerPermissions, controls: bool) -> Ability {
    permissions.screen_capture = false;
    permissions.input_control = false;
    if !controls {
        let rows = permissions.rows.as_mut().unwrap();
        let bus = rows
            .iter_mut()
            .find(|r| r.name == "accessibility bus")
            .unwrap();
        bus.status = RowStatus::Warn;
        bus.detail = "unavailable (`not_installed`): org.a11y.Bus is not provided".into();
    }
    linux_probe(&permissions).unwrap().abilities[1].clone()
}

fn not_available(line: &str, tone: Tone) -> Ability {
    Ability {
        title: PIXELS,
        subtitle: plain(line),
        state: "Not available",
        tone,
    }
}

#[test]
fn without_a_pixel_route_row_no_route_is_named_whatever_the_display_server() {
    let older = answer("linux_older_engine");
    assert!(!older
        .rows
        .as_deref()
        .unwrap()
        .iter()
        .any(|r| r.name == "pixel route"));
    let alone = not_available(
        "Fermix works by controls alone on this desktop.",
        Tone::Quiet,
    );
    let nothing = not_available("They do not work in this session.", Tone::Warn);
    for server in [
        Some("x11"),
        Some("wayland"),
        Some("mir"),
        Some("none"),
        None,
    ] {
        let mut permissions = older.clone();
        permissions.display_server = server.map(str::to_owned);
        let pixels = &linux_probe(&permissions).unwrap().abilities[1];
        assert_eq!(
            (pixels.state, pixels.tone, &pixels.subtitle),
            ("Available", Tone::Good, &Vec::new()),
            "display server {server:?}"
        );
        let with_controls = neither_half(permissions.clone(), true);
        assert_eq!(with_controls, alone, "display server {server:?}");
        assert_eq!(neither_half(permissions, false), nothing, "{server:?}");
    }
    assert_eq!(drawn("linux_older_engine").abilities[2].state, "On X11");
}

#[test]
fn an_x11_route_whose_x_server_did_not_answer_warns_and_its_note_says_why() {
    let permissions = answer("linux_x11_no_x_endpoint");
    let probe = drawn("linux_x11_no_x_endpoint");
    let nothing = not_available("They do not work in this session.", Tone::Warn);
    // Controls work, but a route the helper took and could not use is a fault.
    assert_eq!(probe.abilities[0].state, "Available");
    assert_eq!(probe.abilities[1], nothing);
    assert_eq!(
        probe.abilities[2].subtitle,
        plain("It needs the screenshots, pointer and keys above.")
    );
    assert_eq!(
        note_titles(&probe),
        vec!["ATK bridge", "Chromium-based applications", "X11 pixels"]
    );
    assert_eq!(
        probe.notes[2],
        ProbeNote {
            title: "X11 pixels".into(),
            detail: plain(&detail(&permissions, "x11 pixels")),
            selectable: false,
            attention: true,
        }
    );
    assert_eq!(neither_half(permissions, false), nothing);
    // No bus row is no evidence of controls; the route row still says X11.
    let unheard = with_halves("linux_unknown_rows", false, false);
    assert_eq!(unheard.abilities[1], nothing);
}

#[test]
fn the_pixel_route_row_is_never_a_note() {
    let mut routes = 0;
    for (name, permissions) in every_answer() {
        let Some(probe) = linux_probe(&permissions) else {
            continue;
        };
        let Some(route) = permissions
            .rows
            .as_deref()
            .unwrap()
            .iter()
            .find(|r| r.name == "pixel route")
        else {
            continue;
        };
        routes += 1;
        let spans = note_spans(&route.detail);
        let noted = probe
            .notes
            .iter()
            .any(|n| n.title == "pixel route" || n.title == "Pixel route" || n.detail == spans);
        assert!(!noted, "{name} shows its pixel route among the notes");
    }
    assert_eq!(routes, 10);
}

#[test]
fn unknown_rows_and_status_words_are_shown_as_given_and_asked_about() {
    let permissions = answer("linux_unknown_rows");
    let probe = drawn("linux_unknown_rows");
    assert_eq!(
        states(&probe),
        vec![
            ("Controls", "Unknown", Tone::Quiet),
            (PIXELS, "Partly", Tone::Warn),
            (WINDOW, "Not available", Tone::Quiet),
        ]
    );
    // A row that does not apply is not a note; a word never seen asks for a look.
    assert_eq!(
        note_titles(&probe),
        vec!["ATK bridge", "toolkit gtk3", "X11 pixels", "input method"]
    );
    let attention: Vec<bool> = probe.notes.iter().map(|n| n.attention).collect();
    assert_eq!(attention, vec![false, false, true, true]);
    assert_eq!(
        probe.notes[1].detail,
        plain(&detail(&permissions, "toolkit gtk3"))
    );
}

#[test]
fn macos_older_daemons_and_rowless_answers_keep_todays_rows() {
    assert_eq!(linux_probe(&answer("macos_platform_only")), None);
    assert_eq!(linux_probe(&answer("linux_without_rows")), None);
    let older: ComputerPermissions = decode("computer_use_permissions_get");
    assert_eq!(linux_probe(&older), None);
}

#[test]
fn the_probe_time_reads_as_a_date_and_a_clock() {
    assert_eq!(checked_words("2026-08-19T12:00:00Z"), "2026-08-19 12:00:00");
}
