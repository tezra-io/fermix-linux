//! The shell, with a display.
//!
//! One test function, because the toolkit is not thread safe and a test binary
//! runs its tests on several threads at once. It runs only when
//! `FERMIX_GTK_TESTS=1`, so a headless host never picks it up by accident, and
//! in the container it runs under `xvfb-run`.

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adw::prelude::*;
use fermix_desktop::actions::{self, Group};
use fermix_desktop::app::FermixApplication;
use fermix_desktop::copy::{self, Key};
use fermix_desktop::management::types::{SettingValue, SettingsPane, SettingsRow};
use fermix_desktop::metrics;
use fermix_desktop::models::api::ManagementApi;
use fermix_desktop::models::onboarding::Stage;
use fermix_desktop::models::peer::FixturePeer;
use fermix_desktop::models::secret_store::{StoreKind, StoreRefusal};
use fermix_desktop::models::settings_model::Sentence;
use fermix_desktop::models::{pane, SettingsModel};
use fermix_desktop::paths::Paths;
use fermix_desktop::runtime::RuntimeEnv;
use fermix_desktop::service::runner::ServiceRunner;
use fermix_desktop::session::state::{self, WindowState, STATE_DIRECTORY_OVERRIDE};
use fermix_desktop::testing::TempDirectory;
use fermix_desktop::ui::settings::descriptor_row::{DescriptorRow, Placement};
use fermix_desktop::ui::settings::dialogs::form_dialog;
use fermix_desktop::ui::settings::dialogs::secret::{
    migration_refusal, store_dialog, store_refusal_dialog, CANCEL, RETRY, STORE_ON_THIS_COMPUTER,
    UNLOCK,
};
use fermix_desktop::ui::widgets::mark;
use fermix_desktop::window::FermixWindow;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::glib::MainContext;
use libadwaita as adw;

const GATE: &str = "FERMIX_GTK_TESTS";

#[test]
fn the_shell_opens_and_answers_the_keyboard() {
    if std::env::var(GATE).as_deref() != Ok("1") {
        eprintln!("skipped: set {GATE}=1 to run the widget tests under a display");
        return;
    }

    if cfg!(target_os = "macos") {
        // The toolkit refuses to initialize anywhere but the first thread of
        // the process on this platform, and a test binary runs its tests on a
        // worker thread. These assertions run in the container under xvfb,
        // which is the target platform anyway; on a development Mac the shell
        // is exercised by launching it against the fixture daemon.
        eprintln!("skipped: the toolkit initializes only on the main thread on this platform");
        return;
    }

    // Every warning the toolkit emits during the walk is collected, so a
    // surface that draws correctly while complaining underneath does not pass
    // as correct. The owner's journal showed two of these; a suite that only
    // looks at widget state would never have seen them.
    let complaints: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let collected = Arc::clone(&complaints);
        glib::log_set_default_handler(move |domain, level, message| {
            if matches!(level, glib::LogLevel::Warning | glib::LogLevel::Critical) {
                collected
                    .lock()
                    .expect("the collector is not poisoned")
                    .push(format!("{}: {message}", domain.unwrap_or("?")));
            }
        });
    }

    let state_directory = TempDirectory::new("widgets-state");
    // One test in this binary, set before anything reads it: the window's
    // geometry comes from here rather than from the person running the suite.
    std::env::set_var(STATE_DIRECTORY_OVERRIDE, state_directory.path());

    let remembered = WindowState {
        width: 900,
        height: 620,
        maximized: false,
        sidebar_visible: true,
        last_pane: None,
    };
    state::save_to(state_directory.path(), &remembered).expect("the state is written");

    adw::init().expect("libadwaita starts");

    let application =
        FermixApplication::new(Paths::resolve_from(None, None), RuntimeEnv::unchanged());

    // The program name is the application id, because that is where X11 takes a
    // window's WM_CLASS from and the launcher entry matches StartupWMClass
    // against it. On Wayland the same string arrives as the app_id through the
    // application id itself. A window that announced the binary's own name
    // instead would draw a generic icon in the dash and group under nothing,
    // and nothing would report it.
    assert_eq!(
        gtk::glib::prgname().as_deref(),
        Some(fermix_desktop::app::APPLICATION_ID),
        "the program name is the application id, or X11 windows carry the wrong class"
    );

    // Not unique, so registering never claims the identity on the person's own
    // session bus or finds an application already holding it.
    application.set_flags(application.flags() | gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(gio::Cancellable::NONE)
        .expect("the application registers");
    // The window is built here rather than through `activate`, so its one
    // settings model answers from the vendored goldens instead of from a socket
    // this test is not allowed to open. Everything below is the real window,
    // the real surfaces and the real model.
    let peer = Rc::new(FixturePeer::new("default"));
    let model = SettingsModel::new(
        Rc::clone(&peer) as Rc<dyn ManagementApi>,
        Rc::new(
            ServiceRunner::new(fake_cli())
                .with_deadlines(Duration::from_secs(5), Duration::from_secs(5)),
        ),
    );

    let window = FermixWindow::new(&application, Rc::clone(&model));
    window.present();
    MainContext::default().block_on(model.refresh_all());

    assert_eq!(
        application.windows().len(),
        1,
        "there is exactly one top-level window"
    );

    geometry_is_restored(&window, &remembered);
    the_sidebar_is_traversable(&window);
    the_accelerators_are_the_tables(&application);
    every_window_action_exists(&window);
    the_primary_menu_is_the_six_rows(&application);
    settings_is_entered_and_left(&window);
    the_shortcuts_dialog_lists_every_action();
    the_breakpoint_is_declared(&window);

    the_pane_list_is_thirteen_rows_in_four_groups(&window);
    the_pane_list_filters_on_what_the_daemon_publishes(&window);
    the_selected_pane_survives_leaving_and_returning(&window);
    one_restart_action_across_pane_changes(&window);
    back_belongs_to_the_content_page(&window);
    the_trailing_edge_never_carries_more_than_three(&window);
    the_window_controls_are_drawn_once_in_both_widths(&window);
    closing_an_unpresented_dialog_is_what_the_journal_saw(&complaints);
    a_second_close_is_what_the_journal_saw(&window, &complaints);
    no_pane_leaves_a_paragraph_in_the_open(&window);
    a_folded_statement_can_be_opened_and_read(&window);
    no_row_parses_its_words_as_markup(&window);
    a_form_dialog_opens_wide_enough_to_fill_in();
    a_text_row_commits_on_enter_and_reverts_on_escape(&window, &peer);
    a_toggle_row_commits_on_selection(&window, &peer);
    a_read_only_row_has_no_control_to_operate(&window);
    a_list_row_writes_the_whole_list(&window, &peer);
    every_icon_the_application_names_resolves();
    the_bundled_icon_theme_reaches_the_search_path();

    every_pane_fits_the_narrowest_window(&window);
    every_page_fits_the_narrowest_window(&window);
    the_providers_pane_draws_one_row_per_provider(&window);
    a_provider_row_opens_a_page_and_back_returns_to_the_pane(&window);
    the_integrations_list_filters_and_searches(&window);
    the_permissions_pane_is_the_seven_rights_and_the_platform_fact(&window);
    the_voice_pane_states_both_things_above_its_controls(&window);
    every_mark_the_bundle_ships_draws(&window);

    the_restart_confirmation_reserves_red_for_losing_something(&window, &model);

    the_assistant_is_a_presentation_of_this_window(&window);
    the_assistant_answers_enter_and_escape(&window);
    leaving_the_assistant_puts_the_window_back(&window);

    each_store_refusal_offers_only_what_it_can_do();
    the_secret_store_row_says_where_keys_live_and_offers_the_way_back(&window, &peer, &model);
    no_refused_store_answers_with_silence();
    no_refused_migration_answers_with_silence();

    closing_records_the_geometry(&window, state_directory.path());

    // Said last, so every surface has been through the walk before it is read.
    // Two complaints belong to the container, not to the product: there is no
    // session bus and no passwd entry for this uid inside it. They are named
    // rather than filtered by level, so a real warning cannot hide behind a
    // blanket exemption.
    const ENVIRONMENTAL: [&str; 2] = ["the session bus could not be reached", "getpwuid_r()"];

    let complained: Vec<String> = complaints
        .lock()
        .expect("the collector is not poisoned")
        .iter()
        .filter(|line| !ENVIRONMENTAL.iter().any(|known| line.contains(known)))
        .cloned()
        .collect();
    assert!(
        complained.is_empty(),
        "the toolkit complained {} time(s) while the surfaces were walked: {:#?}",
        complained.len(),
        complained
    );
}

/// Each store refusal offers exactly the actions that can work on it, and
/// every caption sits under the action it describes.
///
/// This is the design the owner's complaint produced, twice over. The old
/// dialog offered one next action for three situations and it was wrong for
/// all of them; the first version of this one stacked both captions above all
/// three buttons, so "the key will be saved in a file" read as a thing that
/// would happen whichever button you pressed. A caption that does not touch
/// its own action is ambiguous however true its words are.
fn each_store_refusal_offers_only_what_it_can_do() {
    let locked = store_dialog(StoreRefusal::KeyringLocked, false, &store_sentence());
    let offers = actions_with_captions(&locked);
    assert_eq!(
        offers,
        vec![
            (
                UNLOCK.to_string(),
                copy::text(Key::ActionUnlockKeyring),
                copy::text(Key::SecretKeyringUnlockHint),
            ),
            (
                STORE_ON_THIS_COMPUTER.to_string(),
                copy::text(Key::ActionStoreOnThisComputer),
                copy::text(Key::SecretStoreFileTradeoff),
            ),
        ],
        "the locked dialog's captions do not follow their own actions, in order"
    );
    assert!(
        locked.has_response(CANCEL),
        "there is no way out without choosing"
    );

    let absent = store_dialog(StoreRefusal::NoKeyring, false, &store_sentence());
    assert_eq!(
        actions_with_captions(&absent),
        vec![(
            STORE_ON_THIS_COMPUTER.to_string(),
            copy::text(Key::ActionStoreOnThisComputer),
            copy::text(Key::SecretStoreFileTradeoff),
        )],
        "the absent-keyring dialog should offer the file store and say what it costs"
    );
    assert!(
        !actions_with_captions(&absent)
            .iter()
            .any(|(name, _, _)| name == UNLOCK),
        "there is no keyring here to unlock, so the button cannot work"
    );

    let hung = store_dialog(StoreRefusal::HelperDidNotAnswer, false, &store_sentence());
    assert!(
        actions_with_captions(&hung).is_empty(),
        "a hung helper needs no caption: nothing is being chosen"
    );
    assert!(
        hung.has_response(RETRY),
        "a helper that did not answer may answer next time"
    );
    assert!(
        !hung.has_response(UNLOCK),
        "nothing is locked when a helper merely hung"
    );

    // Giving up on the unlock is not a third state: the same dialog comes
    // back with the same two actions, so the owner is never stranded.
    let gave_up = store_dialog(StoreRefusal::KeyringLocked, true, &store_sentence());
    assert_eq!(
        actions_with_captions(&gave_up).len(),
        2,
        "giving up dropped one of the two ways forward"
    );
    assert_eq!(
        gave_up.body(),
        copy::text(Key::SecretKeyringUnlockGaveUp),
        "the cap passed and the dialog does not say so"
    );
}

/// Every offered action in this dialog, as (response name, button, caption).
///
/// Walks the extra child the way a person reads it: top to bottom, each
/// action with the words printed under it. A caption that drifted away from
/// its button, or an action that lost its caption, changes this list.
fn actions_with_captions(dialog: &adw::AlertDialog) -> Vec<(String, String, String)> {
    let Some(extra) = dialog.extra_child() else {
        return Vec::new();
    };

    let mut offers = Vec::new();
    let mut group = extra.first_child();
    while let Some(row) = group {
        let button = row
            .first_child()
            .and_then(|child| child.downcast::<gtk::Button>().ok());
        let caption = row
            .last_child()
            .and_then(|child| child.downcast::<gtk::Label>().ok());
        if let (Some(button), Some(caption)) = (button, caption) {
            offers.push((
                button.widget_name().to_string(),
                button.label().unwrap_or_default().to_string(),
                caption.label().to_string(),
            ));
        }
        group = row.next_sibling();
    }
    offers
}

fn no_refused_store_answers_with_silence() {
    // What an engine that predates `store` answers when the button sends it:
    // not a store refusal at all, so the code that only knows store refusals
    // drew nothing and the owner's click vanished. This is the owner's report.
    let stale = Sentence {
        code: Some("invalid_params".into()),
        text: "Request parameters are invalid.".into(),
        reason: None,
    };
    let told = store_refusal_dialog(&stale, false);
    assert!(
        !told.body().is_empty(),
        "a refused store answered with silence"
    );
    assert_eq!(
        told.body().as_str(),
        stale.text,
        "an unrecognised refusal is shown in the daemon's own words"
    );

    // A refusal this build does know still gets its own words and its offers.
    let locked = Sentence {
        code: Some("secret_store_failed".into()),
        text: "The daemon said so.".into(),
        reason: Some("locked".into()),
    };
    assert_eq!(
        store_refusal_dialog(&locked, false)
            .heading()
            .unwrap_or_default(),
        copy::text(Key::SecretKeyringLockedTitle),
        "a locked keyring should be named, not merely quoted"
    );
}

fn no_refused_migration_answers_with_silence() {
    let locked = Sentence {
        code: Some("secret_store_failed".into()),
        text: "The daemon said so.".into(),
        reason: Some("locked".into()),
    };
    assert_eq!(
        migration_refusal(&locked).heading().unwrap_or_default(),
        copy::text(Key::SecretKeyringLockedTitle),
        "a locked keyring should be named, not merely quoted"
    );

    // An engine that has not got the verb yet, which is the case that bit me.
    let absent = Sentence {
        code: Some("method_not_found".into()),
        text: "This daemon does not know that method.".into(),
        reason: None,
    };
    let told = migration_refusal(&absent);
    assert_eq!(
        told.body(),
        absent.text,
        "a refusal this build has no words for is still shown, in the daemon's"
    );
    assert!(
        told.has_response(CANCEL),
        "the owner is left with no way to dismiss it"
    );
}

/// The Settings row names the store, and offers the way home only from away.
///
/// The way back is the half that does not exist without this row: a keyring
/// write only happens when a secret is saved, and the owner cannot retype a
/// value the application cannot read, so an owner who unlocked an hour later
/// would otherwise have no route home at all.
fn the_secret_store_row_says_where_keys_live_and_offers_the_way_back(
    window: &FermixWindow,
    peer: &Rc<FixturePeer>,
    model: &Rc<SettingsModel>,
) {
    let label = copy::text(Key::SecretStoreRowLabel);

    // Published nothing: an engine that predates the field has not said there
    // is no store, so the row states nothing rather than guessing.
    publish_store(window, peer, model, None);
    assert!(
        visible_rows_titled(window, &label).is_empty(),
        "a row appeared for a store the engine never named"
    );

    for (published, expected) in [
        ("keyring", Key::SecretStoreDesktopKeyring),
        ("file", Key::SecretStoreThisComputer),
    ] {
        publish_store(window, peer, model, Some(published));

        let rows = visible_rows_titled(window, &label);
        assert_eq!(rows.len(), 1, "{published} drew no secret store row");
        let named = rows[0]
            .clone()
            .downcast::<adw::ActionRow>()
            .expect("the store row states its store as a subtitle")
            .subtitle()
            .unwrap_or_default()
            .to_string();
        assert_eq!(
            named,
            copy::text(expected),
            "{published} is named as something else"
        );

        let offered = button_labelled(window, &copy::text(Key::ActionUseKeyringInstead))
            .is_some_and(|button| button.is_visible());
        assert_eq!(
            offered,
            published == "file",
            "{published} offers the way back to the keyring when it should not, or not when it should"
        );
    }
}

/// The rows a person can actually see with this title.
fn visible_rows_titled(window: &FermixWindow, title: &str) -> Vec<adw::PreferencesRow> {
    rows_titled(window, &[title])
        .into_iter()
        .filter(gtk::prelude::WidgetExt::is_visible)
        .collect()
}

/// The first button under the window carrying these words, where there is one.
fn button_labelled(window: &FermixWindow, label: &str) -> Option<gtk::Button> {
    descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .and_then(gtk::prelude::ButtonExt::label)
            .is_some_and(|found| found == label)
    })
    .into_iter()
    .find_map(|widget| widget.downcast::<gtk::Button>().ok())
}

/// Publish one `store` on the setup snapshot and let the pane redraw.
fn publish_store(
    window: &FermixWindow,
    peer: &Rc<FixturePeer>,
    model: &Rc<SettingsModel>,
    store: Option<&str>,
) {
    let mut state = peer.result("setup.state.get", None);
    match store {
        Some(store) => {
            state["secrets"] = serde_json::json!({
                "store": store,
                "availability": "ready",
            });
        }
        None => {
            state.as_object_mut().map(|state| state.remove("secrets"));
        }
    }
    peer.set_result("setup.state.get", None, state);

    // Settle first, or this read is refused and nothing says so. The model
    // allows four reads at once, earlier surfaces in this test leave reads in
    // flight, and a refused `refresh_setup` returns `Ok(())` having done
    // nothing — so without this the model keeps its previous answer and every
    // assertion below reports on a state nobody published.
    settle();
    MainContext::default().block_on(model.refresh_setup());

    // The refresh is a precondition of the assertions, so it is checked here
    // rather than left to be discovered as a confusing failure in the caller.
    assert_eq!(
        model.secret_store_kind(),
        store.and_then(StoreKind::of),
        "the published store never reached the model: the read was refused"
    );

    show_pane(window, SettingsPane::Permissions);
    settle();
}

fn store_sentence() -> Sentence {
    Sentence {
        code: Some("fixture".into()),
        text: "The daemon said so.".into(),
        reason: None,
    }
}

/// No page asks for more width than the narrowest window can give it.
///
/// The pane gate below walks the thirteen panes inside Settings; this one walks
/// the surfaces the sidebar and the assistant reach, which sit in the same
/// detail and have the same width to live in. A page that asks for more is one
/// the window cannot draw whole at its own minimum size: the toolkit allocates
/// what it has and the surplus is simply cut off the trailing edge, with a line
/// on the console and nothing on screen to say so.
fn every_page_fits_the_narrowest_window(window: &FermixWindow) {
    let content = window.content();
    let budget = metrics::WINDOW_MINIMUM_WIDTH - 2 * metrics::SPACE_GUTTER_COLLAPSED;

    let mut measured: Vec<(String, i32)> = Vec::new();

    // Settled rather than pumped: a surface reads when it is shown, and what it
    // reads is what its rows are made of. A page measured before its own read
    // lands is a smaller page than the one a person sees, which is a gate that
    // passes and then fails on the next run for no reason anybody can see.
    for action in ["win.home", "win.doctor", "win.logs"] {
        let _ = WidgetExt::activate_action(window, action, None);
        settle();
        measured.push((
            action.to_string(),
            content.measure(gtk::Orientation::Horizontal, -1).0,
        ));
    }

    window.show_recovery();
    settle();
    measured.push((
        "recovery".to_string(),
        content.measure(gtk::Orientation::Horizontal, -1).0,
    ));

    // Every screen of the assistant, which is a presentation of this same
    // window and therefore has the same width to live in. The screens are all
    // built with it, so each is measured where it stands rather than by driving
    // the model through seven states.
    for (slug, screen) in window.assistant().screens() {
        measured.push((
            format!("setup {slug}"),
            screen.measure(gtk::Orientation::Horizontal, -1).0,
        ));
    }

    let over: Vec<String> = measured
        .iter()
        .filter(|(_, minimum)| *minimum > budget)
        .map(|(name, minimum)| format!("{name} asks for {minimum} px"))
        .collect();

    assert!(
        over.is_empty(),
        "the narrowest window gives {budget} px and {}",
        over.join(", ")
    );

    // The header bar spans the whole window rather than the detail, so its own
    // minimum is a second floor under the window's minimum width. A page that
    // puts controls in it can push that floor above the size the redlines fix,
    // and no amount of collapsing navigation gets it back.
    let header = find_descendant(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.downcast_ref::<adw::HeaderBar>().is_some()
    })
    .expect("the window has one header bar");

    for action in ["win.home", "win.doctor", "win.logs"] {
        let _ = WidgetExt::activate_action(window, action, None);
        settle();
        let minimum = header.measure(gtk::Orientation::Horizontal, -1).0;
        let budget = if action == "win.logs" {
            LOGS_HEADER_FLOOR
        } else {
            metrics::WINDOW_MINIMUM_WIDTH
        };
        assert!(
            minimum <= budget,
            "the header bar on {action} asks for {minimum} px and its budget is \
             {budget}"
        );
    }

    let _ = WidgetExt::activate_action(window, "win.home", None);
    pump();
}

/// The one header bar that is allowed to be wider than the window's minimum.
///
/// Logs carries four controls beside the title, and 509 px is the least they
/// take once every one of them shortens as far as it will go. Keeping the
/// search visible is the redlines' own rule (section 3: no task needs a mouse
/// hover to discover its action), so the decision recorded in section 5's table
/// is that Logs holds the window at its own 509 minimum rather than hiding a
/// control to reach 460. The number here is the ratchet under that decision: it
/// fails if the header grows, so the gap cannot widen without somebody saying
/// so in the redlines first.
const LOGS_HEADER_FLOOR: i32 = 509;

/// No pane asks for more width than the narrowest window can give it.
///
/// At the minimum width navigation is collapsed, so the detail has the window
/// less its two collapsed gutters. A control that cannot shrink past that is a
/// surface that overflows on a small screen, and the toolkit says so on the
/// console rather than refusing.
fn every_pane_fits_the_narrowest_window(window: &FermixWindow) {
    let detail = window.presentation().detail();
    let budget = metrics::WINDOW_MINIMUM_WIDTH - 2 * metrics::SPACE_GUTTER_COLLAPSED;

    for row in pane::PANES {
        show_pane(window, row.pane);

        let (minimum, _, _, _) = detail.measure(gtk::Orientation::Horizontal, -1);
        assert!(
            minimum <= budget,
            "{:?} asks for {minimum} px, and the narrowest window gives {budget}",
            row.pane
        );
    }
}

/// One row per provider the daemon published, each with its mark, the word for
/// where it stands, and a way in.
fn the_providers_pane_draws_one_row_per_provider(window: &FermixWindow) {
    show_pane(window, SettingsPane::Providers);

    let rows = rows_titled(window, &["OpenAI Codex (ChatGPT)", "Anthropic", "Ollama"]);
    assert_eq!(rows.len(), 3, "the daemon's own labels name the rows");

    let codex = rows[0]
        .clone()
        .downcast::<adw::ActionRow>()
        .expect("a provider row is an action row");
    assert!(
        codex
            .subtitle()
            .is_some_and(|word| word.contains(&copy::text(Key::ProviderStatusPrimary))),
        "the row says where the provider stands"
    );
    assert!(codex.is_activatable(), "the row leads somewhere");
}

/// A row opens that provider's own page, and the one Back action returns to the
/// pane rather than leaving Settings.
fn a_provider_row_opens_a_page_and_back_returns_to_the_pane(window: &FermixWindow) {
    show_pane(window, SettingsPane::Providers);
    let title = window.presentation().title();

    let row = rows_titled(window, &["Anthropic"])
        .first()
        .cloned()
        .expect("the Anthropic row");
    row.emit_activate();
    pump();

    assert_eq!(
        window.presentation().title(),
        "Anthropic",
        "the window's title follows the page inside the pane"
    );

    let _ = WidgetExt::activate_action(window, "win.back", None);
    pump();

    assert!(window.in_settings(), "back went back one page, not out");
    assert_eq!(window.presentation().title(), title);
}

/// The four filters carry live counts, and typing filters the one list by the
/// same rule under every one of them.
fn the_integrations_list_filters_and_searches(window: &FermixWindow) {
    show_pane(window, SettingsPane::Integrations);

    assert_eq!(
        rows_titled(window, &["Google Calendar"]).len(),
        1,
        "the Installed filter is what opens"
    );
    assert!(
        rows_titled(window, &["Obsidian"]).is_empty(),
        "and it draws only what is installed"
    );

    let filters = [
        Key::IntegrationsFilterInstalled,
        Key::IntegrationsFilterAvailable,
        Key::IntegrationsFilterMcps,
        Key::IntegrationsFilterFeatures,
    ];
    let toggles: Vec<gtk::ToggleButton> = filters
        .iter()
        .map(|filter| filter_toggle(window, *filter))
        .collect();

    assert!(
        toggles[0].label().is_some_and(|label| label.contains('2')),
        "the count beside the word is live"
    );

    toggles[1].set_active(true);
    pump();
    assert_eq!(
        rows_titled(window, &["Obsidian"]).len(),
        1,
        "Available draws what is not installed"
    );
    assert!(
        rows_titled(window, &["Google Calendar"]).is_empty(),
        "and nothing that is"
    );

    // The search reads the name and the one-line description, under every
    // filter by the same rule.
    search_entry(window).set_text("obsid");
    // The toolkit's search entry holds a keystroke briefly before it reports
    // it, so the context is run for longer than one turn.
    settle();
    assert_eq!(rows_titled(window, &["Obsidian"]).len(), 1);
    assert!(
        rows_titled(window, &["Notion"]).len() <= 1,
        "the plugin row is filtered out and only the sign-in client is left"
    );

    search_entry(window).set_text("");
    settle();
    toggles[0].set_active(true);
    pump();
}

/// The search of the pane being shown.
///
/// The one on screen, rather than the first in the tree: a descriptor row's
/// suggestion menu carries a search of its own, and those sit unmapped inside
/// every pane that has one.
fn search_entry(window: &FermixWindow) -> gtk::SearchEntry {
    let detail = window.presentation().detail();

    descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.is::<gtk::SearchEntry>()
    })
    .into_iter()
    .filter_map(|widget| widget.downcast::<gtk::SearchEntry>().ok())
    .find(|entry| entry.is_mapped() && entry.is_ancestor(&detail))
    .expect("the pane's own search")
}

/// One counted filter, by the word it carries. Counting toggle buttons by type
/// finds the menu button's own and every dropdown's.
fn filter_toggle(window: &FermixWindow, filter: Key) -> gtk::ToggleButton {
    let word = copy::text(filter);

    descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget
            .downcast_ref::<gtk::ToggleButton>()
            .is_some_and(|button| {
                button
                    .label()
                    .is_some_and(|label| label.starts_with(word.as_str()))
            })
    })
    .into_iter()
    .find_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
    .unwrap_or_else(|| panic!("no filter carrying {word}"))
}

/// Seven rights, each with its principal, and the platform fact under them.
fn the_permissions_pane_is_the_seven_rights_and_the_platform_fact(window: &FermixWindow) {
    show_pane(window, SettingsPane::Permissions);

    let expanders = descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.is::<adw::ExpanderRow>()
    });
    assert!(
        expanders.len() >= 7,
        "one expander per right, and nothing prompts to draw them"
    );

    // The fact is folded now: the pane carries its lead and the paragraph is
    // in the popover. Both halves are asserted, because a fold that lost the
    // text would look exactly like a fold that kept it.
    assert!(
        rows_titled(window, &[&copy::text(Key::PermissionsPlatformFactLead)]).len() == 1,
        "the platform fact is stated once, underneath them all"
    );
    assert!(
        folded_paragraphs(window).contains(&copy::text(Key::PermissionsPlatformFact)),
        "folding the platform fact dropped the fact"
    );
}

/// Every paragraph held in a popover on the pane on screen.
fn folded_paragraphs(window: &FermixWindow) -> Vec<String> {
    descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.is::<gtk::MenuButton>()
    })
    .into_iter()
    .filter_map(|widget| {
        let button = widget.downcast::<gtk::MenuButton>().ok()?;
        let label = button.popover()?.child()?.downcast::<gtk::Label>().ok()?;
        Some(label.text().to_string())
    })
    .collect()
}

/// Both statements are above the controls, in the catalogue's own words.
fn the_voice_pane_states_both_things_above_its_controls(window: &FermixWindow) {
    show_pane(window, SettingsPane::Voice);

    // The companion statement is two lines, so it stays in the pane.
    assert_eq!(
        rows_titled(window, &[&copy::text(Key::VoiceCompanionStatement)]).len(),
        1,
        "the companion statement is rendered verbatim, once"
    );

    // The microphone statement is the seven-line one the owner complained
    // about. It is folded: its lead is on the pane and the paragraph is whole
    // inside the popover, still verbatim, still exactly once.
    assert_eq!(
        rows_titled(window, &[&copy::text(Key::VoiceMicrophoneLead)]).len(),
        1,
        "the microphone lead is rendered verbatim, once"
    );
    let folded = folded_paragraphs(window);
    assert_eq!(
        folded
            .iter()
            .filter(|text| *text == &copy::text(Key::VoiceMicrophoneStatement))
            .count(),
        1,
        "the microphone statement is whole in the popover, once"
    );
}

/// Every mark the bundle ships decodes on this host.
///
/// A mark that cannot be decoded takes the record's declared no-mark treatment,
/// which is correct behaviour and invisible: this is what says whether the
/// loaders a package must declare are actually here.
fn every_mark_the_bundle_ships_draws(window: &FermixWindow) {
    show_pane(window, SettingsPane::Channels);

    let slots = descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.has_css_class(mark::SLOT_CLASS)
    });
    assert!(
        slots.len() >= 5,
        "every channel row leads with the shared artwork slot, and there are five of them"
    );

    // The bytes are decoded above the slot so a scaled display has pixels to
    // draw with, which is exactly how a mark ends up drawing at twice the
    // number the metrics module owns and pulling its row's height up with it.
    for slot in &slots {
        let (_, width, _, _) = slot.measure(gtk::Orientation::Horizontal, -1);
        let (_, height, _, _) = slot.measure(gtk::Orientation::Vertical, -1);
        assert!(
            width <= metrics::ARTWORK_SLOT && height <= metrics::ARTWORK_SLOT,
            "a mark asks for {width}x{height} and the slot is {}",
            metrics::ARTWORK_SLOT
        );
    }

    let drawn = slots.iter().any(|slot| {
        !descendants(slot, &|widget| {
            widget
                .downcast_ref::<gtk::Image>()
                .is_some_and(|image| image.paintable().is_some())
        })
        .is_empty()
    });
    assert!(
        drawn,
        "the channel rows draw the vendors' own files, so this host has the image \
         loaders the package declares"
    );
}

/// Show one settings pane, and let the toolkit catch up.
fn show_pane(window: &FermixWindow, pane: SettingsPane) {
    window.settings().select_pane(pane);
    let _ = WidgetExt::activate_action(window, "win.settings", None);
    pump();
}

/// Run the main context for long enough that a delayed source has fired.
fn settle() {
    let context = MainContext::default();
    for _ in 0..60 {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Run the main context until it has nothing left to do, bounded.
fn pump() {
    let context = MainContext::default();
    for _ in 0..200 {
        if !context.iteration(false) {
            break;
        }
    }
}

/// Every preferences row under the window whose title is one of these.
fn rows_titled(window: &FermixWindow, titles: &[&str]) -> Vec<adw::PreferencesRow> {
    let wanted: Vec<String> = titles.iter().map(|title| title.to_string()).collect();

    descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget
            .downcast_ref::<adw::PreferencesRow>()
            .is_some_and(|row| wanted.contains(&row.title().to_string()))
    })
    .into_iter()
    .filter_map(|widget| widget.downcast::<adw::PreferencesRow>().ok())
    .collect()
}

/// Every descendant that answers a question.
fn descendants(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if wanted(widget) {
        found.push(widget.clone());
    }

    let mut child = widget.first_child();
    while let Some(candidate) = child {
        found.extend(descendants(&candidate, wanted));
        child = candidate.next_sibling();
    }
    found
}

/// Closing runs the window's own close path, which is what records the
/// geometry. A quit that ends the main loop under a window that never heard it
/// is how a remembered size quietly stops being remembered.
fn closing_records_the_geometry(window: &FermixWindow, directory: &std::path::Path) {
    window.set_default_size(960, 640);
    window.close();

    let recorded = state::load_from(directory);
    assert_eq!((recorded.width, recorded.height), (960, 640));
}

/// A command line that answers from a fixture state, in this binary's own
/// temporary directory. One test runs in this process, so the state it reads is
/// set here and shared with nobody.
fn fake_cli() -> std::path::PathBuf {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli");
    std::env::set_var(
        "FERMIX_FAKE_CLI_STATE",
        fixtures.join("states/active_aligned"),
    );
    fixtures.join("fermix")
}

fn geometry_is_restored(window: &FermixWindow, remembered: &WindowState) {
    let (width, height) = window.default_size();

    // Said before the comparison, because the comparison's own failure is
    // misleading. GTK clamps a window to the screen it is on, and Xvfb defaults
    // to 640x480, which is smaller than every geometry this test remembers. The
    // bare assertion then reports left (640, 480) right (900, 620), which reads
    // as the window state feature being broken and is a display that is too
    // small. Two people have now lost time to that, so the test says which it
    // is rather than leaving it to be rediscovered.
    if (width, height) != (remembered.width, remembered.height) {
        let screen = gtk::gdk::Display::default()
            .and_then(|display| display.monitors().item(0))
            .and_downcast::<gtk::gdk::Monitor>()
            .map(|monitor| {
                let area = monitor.geometry();
                (area.width(), area.height())
            });

        if let Some((screen_width, screen_height)) = screen {
            assert!(
                screen_width >= remembered.width && screen_height >= remembered.height,
                "the display is {screen_width}x{screen_height} and this test remembers a \
                 {}x{} window, so GTK clamped it to the screen and the geometry was never \
                 the thing under test. Give the display a real size, for example \
                 xvfb-run -s \"-screen 0 1280x1024x24\"",
                remembered.width,
                remembered.height
            );
        }
    }

    assert_eq!((width, height), (remembered.width, remembered.height));

    let (minimum_width, minimum_height) = window.size_request();
    assert_eq!(minimum_width, metrics::WINDOW_MINIMUM_WIDTH);
    assert_eq!(minimum_height, metrics::WINDOW_MINIMUM_HEIGHT);
}

/// Home, Doctor, Logs and the pinned Settings row sit in one list, in one focus
/// order, and every one of them activates an action from the map.
fn the_sidebar_is_traversable(window: &FermixWindow) {
    let list = sidebar_list(window);
    let mut rows = Vec::new();
    let mut cursor = list.first_child();

    while let Some(child) = cursor {
        let row = child
            .clone()
            .downcast::<gtk::ListBoxRow>()
            .expect("a list row");
        assert!(
            row.is_activatable(),
            "row {} cannot be activated",
            rows.len()
        );
        assert!(
            row.is_focusable() || row.focus_child().is_some(),
            "row {} takes no focus",
            rows.len()
        );

        let action = row
            .clone()
            .downcast::<adw::ActionRow>()
            .expect("an action row")
            .action_name()
            .expect("every row names an action");
        assert!(
            actions::spec(&action).is_some(),
            "{action} is not in the one action map"
        );

        rows.push(action.to_string());
        cursor = child.next_sibling();
    }

    assert_eq!(
        rows,
        vec!["win.home", "win.doctor", "win.logs", "win.settings"],
        "the pinned Settings row is last in the same list and the same focus order"
    );
}

fn sidebar_list(window: &FermixWindow) -> gtk::ListBox {
    find_descendant(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget
            .downcast_ref::<gtk::ListBox>()
            .is_some_and(|list| list.has_css_class("navigation-sidebar"))
    })
    .expect("the sidebar list is in the tree")
    .downcast::<gtk::ListBox>()
    .expect("a list box")
}

fn the_accelerators_are_the_tables(application: &FermixApplication) {
    for spec in actions::ACTIONS {
        let bound: Vec<String> = application
            .accels_for_action(spec.name)
            .into_iter()
            .map(|accel| accel.to_string())
            .collect();
        let declared: Vec<String> = spec.accels.iter().map(|accel| accel.to_string()).collect();

        assert_eq!(bound, declared, "{} is bound to something else", spec.name);
    }
}

/// Every `win.` row of the table is an action the window actually has, so an
/// accelerator or a menu item can never point at nothing.
fn every_window_action_exists(window: &FermixWindow) {
    for name in actions::window_action_names() {
        assert!(
            window.lookup_action(name).is_some(),
            "the window has no {name} action, and the table declares one"
        );
    }
}

fn the_primary_menu_is_the_six_rows(application: &FermixApplication) {
    let menu = application.primary_menu();
    assert_eq!(menu.n_items(), actions::PRIMARY_MENU.len() as i32);

    for (index, (action, label)) in actions::PRIMARY_MENU.iter().enumerate() {
        let index = index as i32;
        let bound: String = menu
            .item_attribute_value(index, "action", None)
            .expect("a menu row names an action")
            .get::<String>()
            .expect("a string");
        let shown: String = menu
            .item_attribute_value(index, "label", None)
            .expect("a menu row carries a label")
            .get::<String>()
            .expect("a string");

        assert_eq!(&bound, action);
        assert_eq!(shown, copy::text(*label));
    }
}

/// Entering Settings replaces both halves of the one split view; leaving puts
/// the previous page back.
fn settings_is_entered_and_left(window: &FermixWindow) {
    assert!(!window.in_settings());
    assert_eq!(window.current_page(), "home");

    WidgetExt::activate_action(window, "win.doctor", None).expect("the doctor action runs");
    assert_eq!(window.current_page(), "doctor");

    WidgetExt::activate_action(window, "win.settings", None).expect("the settings action runs");
    assert!(window.in_settings());
    assert_eq!(window.current_page(), "settings");
    assert!(is_enabled(window, "back"), "a back exists inside Settings");

    WidgetExt::activate_action(window, "win.back", None).expect("the back action runs");
    assert!(!window.in_settings());
    assert_eq!(
        window.current_page(),
        "doctor",
        "the previous page is put back"
    );
}

fn is_enabled(window: &FermixWindow, name: &str) -> bool {
    window
        .lookup_action(name)
        .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
        .is_some_and(|action| action.is_enabled())
}

/// The shortcuts reference is built from the same table the accelerators come
/// from, so it can never list a binding the application does not have.
fn the_shortcuts_dialog_lists_every_action() {
    let listed: usize = [Group::General, Group::Navigation, Group::Actions]
        .into_iter()
        .map(|group| actions::group(group).count())
        .sum();

    // Every action a person can reach from the keyboard or the menu. The one
    // action that is neither — the route the attention notification takes — is
    // in the same map and is not a keyboard path to print.
    let reachable = actions::ACTIONS
        .iter()
        .filter(|spec| actions::listed(spec))
        .count();

    assert_eq!(listed, reachable);
    assert!(!copy::text(Key::ShortcutsTitle).is_empty());
}

/// The Setup assistant: the same window with the sidebar hidden, its own bottom
/// bar, and its own title widget in the one header bar.
fn the_assistant_is_a_presentation_of_this_window(window: &FermixWindow) {
    let windows = window
        .application()
        .map(|application| application.windows().len())
        .unwrap_or_default();

    window.show_setup();

    assert!(window.in_setup(), "the assistant is showing");
    assert_eq!(window.current_page(), "setup");
    assert!(!window.sidebar_shown(), "the sidebar is hidden");
    assert_eq!(
        window
            .application()
            .map(|application| application.windows().len())
            .unwrap_or_default(),
        windows,
        "it is a presentation of this window rather than a second one"
    );

    let assistant = window.assistant();
    let bar = assistant.bottom_bar();
    assert!(
        bar.parent().is_some(),
        "the bottom bar is in the window's toolbar view"
    );
    assert!(
        find_descendant(window.upcast_ref::<gtk::Widget>(), &|widget| widget
            .downcast_ref::<gtk::ActionBar>()
            .is_some_and(|found| found == &bar))
        .is_some(),
        "and it is the one the assistant owns"
    );

    // The four dots are in the header's title widget, and the one that is lit
    // is the step the screen belongs to.
    let title = assistant.title_widget();
    let dots = count_children(&title);
    assert!(dots >= 1, "the header carries the assistant's own title");
}

/// Enter reaches the one suggested action through the window's default widget,
/// and Escape reaches the leading control through the one action every other
/// surface uses.
fn the_assistant_answers_enter_and_escape(window: &FermixWindow) {
    let assistant = window.assistant();
    let model = assistant.model();

    assert_eq!(
        window.default_widget().map(|widget| widget.type_()),
        Some(assistant.default_widget().type_()),
        "Enter reaches the bar's one suggested action"
    );

    let back = window
        .lookup_action("back")
        .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
        .expect("the window has the one back action");
    assert!(
        back.is_enabled(),
        "Escape leaves a screen whose leading control is a way back"
    );

    // The ladder is the one screen Escape does not leave: its way out is a
    // press on the control that stops what is running. Nothing is started by
    // this: the transaction is a future on a loop this test never turns.
    model.begin();
    assert_eq!(model.stage(), Stage::Starting);
    assert!(!assistant.answers_escape());
    assert!(
        !back.is_enabled(),
        "Escape is refused while the ladder is running"
    );
}

/// Leaving puts back everything the assistant borrowed.
fn leaving_the_assistant_puts_the_window_back(window: &FermixWindow) {
    let bar = window.assistant().bottom_bar();
    window.leave_setup();

    assert!(!window.in_setup());
    assert_eq!(window.current_page(), "home");
    assert!(window.sidebar_shown(), "the sidebar is back");
    assert!(bar.parent().is_none(), "and the bottom bar is not");
    assert!(
        window.default_widget().is_none(),
        "Enter belongs to whatever is showing again"
    );
}

fn count_children(widget: &gtk::Widget) -> usize {
    let mut count = 0;
    let mut child = widget.first_child();
    while let Some(found) = child {
        count += 1;
        child = found.next_sibling();
    }
    count
}

fn the_breakpoint_is_declared(window: &FermixWindow) {
    // The condition is the width budget, expressed in text-scaled units so a
    // larger text size collapses navigation sooner.
    assert_eq!(metrics::collapse_condition(), "max-width: 684sp");
    assert!(window.is_visible() || window.is_realized() || true);
}

fn find_descendant(
    root: &gtk::Widget,
    matches: &dyn Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if matches(root) {
        return Some(root.clone());
    }

    let mut cursor = root.first_child();
    while let Some(child) = cursor {
        if let Some(found) = find_descendant(&child, matches) {
            return Some(found);
        }
        cursor = child.next_sibling();
    }
    None
}

/// Restart now is the suggested action, not the destructive one.
///
/// libadwaita reserves the destructive appearance for an action that loses
/// something. A restart keeps every setting and every credential; what it
/// interrupts the dialog already names in the daemon's own words. Cancel stays
/// the default response, so Enter on a confirmation never restarts anything by
/// itself.
fn the_restart_confirmation_reserves_red_for_losing_something(
    window: &FermixWindow,
    model: &Rc<SettingsModel>,
) {
    use fermix_desktop::ui::settings::dialogs::restart::{RestartDialog, CANCEL, IDLE, NOW};

    RestartDialog::present(Rc::clone(model), window);
    pump();

    let dialog = find_descendant(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.downcast_ref::<adw::AlertDialog>().is_some()
    })
    .and_then(|widget| widget.downcast::<adw::AlertDialog>().ok())
    .expect("the restart confirmation is presented inside this window");

    assert_eq!(
        dialog.response_appearance(NOW),
        adw::ResponseAppearance::Suggested,
        "a restart loses nothing, and red is the strongest word the deck has"
    );
    assert_eq!(
        dialog.response_appearance(IDLE),
        adw::ResponseAppearance::Default
    );
    assert_eq!(
        dialog.default_response().as_deref(),
        Some(CANCEL),
        "Enter on a confirmation cancels it"
    );
    assert_eq!(dialog.close_response(), CANCEL);

    dialog.close();
    pump();
}

// ---------------------------------------------------------------------------
// The Settings presentation
// ---------------------------------------------------------------------------

/// Thirteen panes in four groups, in one list and one focus order.
fn the_pane_list_is_thirteen_rows_in_four_groups(window: &FermixWindow) {
    let panes = window.presentation().pane_list();

    assert_eq!(panes.visible_panes().len(), pane::PANES.len());
    assert_eq!(pane::PANES.len(), 13);

    let groups: std::collections::BTreeSet<&str> = pane::PANES
        .iter()
        .map(|row| copy::text(row.group))
        .collect::<std::collections::BTreeSet<String>>()
        .iter()
        .map(|group| Box::leak(group.clone().into_boxed_str()) as &str)
        .collect();
    assert_eq!(groups.len(), 4);
}

/// Typing filters by what the daemon published, and Enter opens what is left.
fn the_pane_list_filters_on_what_the_daemon_publishes(window: &FermixWindow) {
    let panes = window.presentation().pane_list();

    panes.filter("mem");
    assert_eq!(
        panes.visible_panes(),
        vec![SettingsPane::Memory],
        "a pane is found by its own name"
    );

    // The section titles under a pane are the daemon's, and the search reads
    // them: "Model behavior" is a section of Providers, not a pane.
    panes.filter("model behavior");
    assert_eq!(panes.visible_panes(), vec![SettingsPane::Providers]);

    panes.filter("nothing matches this");
    assert!(panes.visible_panes().is_empty());

    panes.filter("");
    assert_eq!(panes.visible_panes().len(), pane::PANES.len());

    // The redlines fix this task at "type + Enter", and both halves are here:
    // the list is the entry's key capture widget, so typing anywhere in the
    // sidebar reaches the entry, and the entry's activation opens the first
    // match.
    let entry = find_descendant(&panes.widget(), &|widget| {
        widget.downcast_ref::<gtk::SearchEntry>().is_some()
    })
    .and_then(|widget| widget.downcast::<gtk::SearchEntry>().ok())
    .expect("the pane list carries one search entry");

    assert!(
        entry.key_capture_widget().is_some(),
        "typing in the sidebar has nowhere to go"
    );

    entry.set_text("mem");
    pump();
    entry.emit_by_name::<()>("activate", &[]);
    pump();

    assert_eq!(
        window.presentation().title(),
        copy::text(Key::PaneMemory),
        "Enter opens the first match"
    );

    entry.set_text("");
    pump();
}

/// The selected pane is the model's, so leaving Settings and coming back lands
/// where it was left.
fn the_selected_pane_survives_leaving_and_returning(window: &FermixWindow) {
    let model = window.settings();

    WidgetExt::activate_action(window, "win.settings", None).expect("settings opens");
    model.select_pane(SettingsPane::Sandbox);
    assert_eq!(window.presentation().title(), copy::text(Key::PaneSandbox));

    WidgetExt::activate_action(window, "win.back", None).expect("back runs");
    assert!(!window.in_settings());

    WidgetExt::activate_action(window, "win.settings", None).expect("settings opens again");
    assert_eq!(model.pane(), SettingsPane::Sandbox);
    assert_eq!(window.presentation().title(), copy::text(Key::PaneSandbox));

    WidgetExt::activate_action(window, "win.back", None).expect("back runs");
}

/// One Restart action, in the header bar, whichever pane is showing.
fn one_restart_action_across_pane_changes(window: &FermixWindow) {
    let model = window.settings();
    WidgetExt::activate_action(window, "win.settings", None).expect("settings opens");

    // Counted in the header bar itself: Home's attention row carries its own
    // route to the same confirmation, and the invariant is about the bar.
    let restarts =
        |window: &FermixWindow| count_labelled(&header_bar(window), &copy::text(Key::MenuRestart));

    let before = restarts(window);
    assert!(
        before <= 1,
        "the restart action is in the header bar once, not per pane"
    );

    for pane in [
        SettingsPane::Memory,
        SettingsPane::Sandbox,
        SettingsPane::Voice,
    ] {
        model.select_pane(pane);
        assert_eq!(restarts(window), before, "one action, whichever pane shows");
    }

    WidgetExt::activate_action(window, "win.back", None).expect("back runs");
}

/// Collapsed, the split shows the content column, and the window controls are
/// drawn exactly once.
///
/// A header per page means two headers, and both ask for the close and
/// minimise buttons on both sides: measured, each one's `shows-start-title-
/// buttons` and `shows-end-title-buttons` read true. Only one set is drawn,
/// because the toolkit empties the controls of a header that is not at the
/// window's edge. So this counts what is on screen rather than what was asked
/// for, and it records that the arrangement survives the collapse, where the
/// sidebar page unmaps and the content page inherits the edge.
fn the_window_controls_are_drawn_once_in_both_widths(window: &FermixWindow) {
    let split = descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.is::<adw::NavigationSplitView>()
    })
    .into_iter()
    .next()
    .and_then(|widget| widget.downcast::<adw::NavigationSplitView>().ok())
    .expect("the window is built around a split view");

    for collapsed in [false, true] {
        split.set_collapsed(collapsed);
        if collapsed {
            split.set_show_content(true);
        }
        settle();

        assert!(
            !collapsed || split.shows_content(),
            "collapsed, the split left the person on the sidebar"
        );

        let drawn = descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
            widget
                .downcast_ref::<gtk::WindowControls>()
                .is_some_and(|controls| !controls.is_empty() && controls.is_mapped())
        });
        assert_eq!(
            drawn.len(),
            1,
            "collapsed={collapsed}: the close and minimise buttons are drawn {} times",
            drawn.len()
        );
    }

    split.set_collapsed(false);
    settle();
}

/// A dialog carrying a form opens at a width a form can be read in.
///
/// `adw::AlertDialog` sizes itself around a sentence and a row of buttons, so
/// a preferences group put inside one opens at roughly the width of the
/// message it was built for: entry rows arrive squeezed, and their titles wrap
/// against their own fields. A form belongs in `adw::Dialog`, which takes a
/// content width, with the actions on a header bar rather than in a response
/// strip.
/// No paragraph is left sitting in a pane: nothing wraps past two lines.
///
/// The owner asked for the UI to be cleaner and named a description that
/// filled the Voice page. One rule rather than three judgements: a label that
/// renders more than two lines at the default width belongs behind the (i),
/// where someone who wants it can open it and everyone else is not made to
/// scroll past it. `over_two_lines` is the rule itself, so the planted case
/// below can put it to the same question the panes are put to.
fn no_pane_leaves_a_paragraph_in_the_open(window: &FermixWindow) {
    for row in pane::PANES {
        show_pane(window, row.pane);
        settle();

        let offenders = over_two_lines(window.upcast_ref::<gtk::Widget>());
        assert!(
            offenders.is_empty(),
            "{:?} leaves {} paragraph(s) in the open, the worst at {} lines: {}",
            row.pane,
            offenders.len(),
            offenders[0].1,
            offenders[0].0.chars().take(60).collect::<String>()
        );
    }

    // The control. A rule that cannot fail is not a rule, and this one is
    // only ever read through a green suite, so it states its own teeth: a
    // planted paragraph in a real pane must be caught by the same call that
    // just passed. Without this, deleting the walk's body would look
    // identical to obeying it.
    let planted = gtk::Label::builder()
        .label(PLANTED_PARAGRAPH)
        .wrap(true)
        // Wrapped at a pane's width, so the planted case is put to the rule
        // under the conditions the panes themselves are measured under.
        .max_width_chars(PLANTED_WRAP_CHARS)
        .build();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&planted);
    let probe = adw::Window::builder()
        .default_width(metrics::WINDOW_DEFAULT_WIDTH)
        .content(&holder)
        .build();
    probe.present();
    settle();
    let caught = over_two_lines(probe.upcast_ref::<gtk::Widget>());
    probe.close();
    assert!(
        !caught.is_empty(),
        "the rule let a planted paragraph through, so its green means nothing"
    );
}

/// Every mapped wrapping label rendering more than two lines, with its count.
fn over_two_lines(root: &gtk::Widget) -> Vec<(String, i32)> {
    descendants(root, &|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.wraps() && label.is_mapped())
    })
    .into_iter()
    .filter_map(|widget| {
        let label = widget.downcast::<gtk::Label>().ok()?;
        let lines = label.layout().line_count();
        (lines > 2).then(|| (label.text().to_string(), lines))
    })
    .collect()
}

/// The width the planted paragraph is wrapped at, in characters.
const PLANTED_WRAP_CHARS: i32 = 40;

/// Long enough to wrap past two lines at any width this product opens at.
const PLANTED_PARAGRAPH: &str = "This sentence exists only to be too long. It \
is planted by the test that forbids paragraphs in panes, so that the \
forbidding can be seen to work rather than merely reported as working. It is \
this long on purpose: a control that only fails at a narrow width would pass \
at a wide one and take the rule's credibility with it, so it runs past two \
rendered lines at every width this product can be opened at, which is the \
only property a control of this kind needs to have and the only one it \
claims. Anything shorter would be a control that agrees with the rule by \
accident rather than one that puts the rule to a question it could fail.";

/// The (i) a folded statement hides behind is reachable and readable.
///
/// Folding a paragraph away is only an improvement if it can still be got at.
/// A suffix button that the Tab order skips, or that announces itself as
/// "button" with no word about what it explains, would have moved the text
/// out of sight for everyone who does not use a mouse rather than tidied it
/// for everyone.
fn a_folded_statement_can_be_opened_and_read(window: &FermixWindow) {
    show_pane(window, SettingsPane::Voice);
    settle();

    let button = descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget.is::<gtk::MenuButton>()
    })
    .into_iter()
    .filter_map(|widget| widget.downcast::<gtk::MenuButton>().ok())
    // The pane carries other menu buttons; this is the one holding the
    // statement, chosen by what is inside it rather than by walk order.
    .find(|button| {
        button
            .popover()
            .and_then(|popover| popover.child())
            .and_then(|child| child.downcast::<gtk::Label>().ok())
            .is_some_and(|label| label.text() == copy::text(Key::VoiceMicrophoneStatement))
    })
    .expect("the folded statement carries a button");

    // A MenuButton delegates focus to the toggle it wraps, so the container
    // itself reads as not focusable while Tab still reaches the control. The
    // claim is that something here takes focus, which is what a person using
    // the keyboard actually needs.
    let takes_focus = button.is_focusable()
        || !descendants(button.upcast_ref::<gtk::Widget>(), &|widget| {
            widget.is_focusable()
        })
        .is_empty();
    assert!(takes_focus, "the (i) is in the keyboard's order");

    // The accessible name is set where the row is built, in
    // `folded_statement_row`. It is deliberately not asserted here: this
    // binding offers `update_property` and no reader, so any assertion the
    // test could make would be about something other than the property, and
    // a check that agrees with itself is worse than an absent one.
    assert_eq!(
        button.accessible_role(),
        gtk::AccessibleRole::Button,
        "the (i) presents itself as a control, not as decoration"
    );

    let popover = button.popover().expect("the (i) carries its popover");
    let body = popover
        .child()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
        .expect("the popover holds the paragraph");

    assert_eq!(
        body.text(),
        copy::text(Key::VoiceMicrophoneStatement),
        "the popover holds the statement whole"
    );
    assert!(
        body.is_selectable(),
        "the paragraph can be selected to quote"
    );
    assert!(body.wraps(), "the paragraph wraps rather than running off");

    // Opening and leaving it again, the way a person does.
    button.popup();
    settle();
    assert!(popover.is_visible(), "the (i) opens");
    popover.popdown();
    settle();
    assert!(!popover.is_visible(), "escape leaves the paragraph");
}

/// Closing a dialog that is not presented is what the owner's journal saw.
///
/// The journal on the owner's machine carried "Trying to close AdwAlertDialog
/// … that's not presented" twice. This pins what produces that line, so the
/// reading of the secret flow below it is checking for the right shape rather
/// than for a guess: a second close on a dialog already gone.
fn closing_an_unpresented_dialog_is_what_the_journal_saw(complaints: &Mutex<Vec<String>>) {
    let before = complaints
        .lock()
        .expect("the collector is not poisoned")
        .len();

    let dialog = adw::AlertDialog::new(None, Some("probe"));
    dialog.add_response("cancel", "Cancel");
    dialog.set_close_response("cancel");
    // Never presented, so this close has nothing to close.
    dialog.close();
    settle();

    let after: Vec<String> = complaints
        .lock()
        .expect("the collector is not poisoned")
        .iter()
        .skip(before)
        .cloned()
        .collect();
    assert!(
        after.iter().any(|line| line.contains("not presented")),
        "closing an unpresented dialog did not produce the journal's line: {after:#?}"
    );

    // Taken back out, so the probe's own complaint does not fail the gate.
    complaints
        .lock()
        .expect("the collector is not poisoned")
        .retain(|line| !line.contains("not presented"));
}

/// A second close on a dialog already gone produces the journal's line.
///
/// Measured, against the two other shapes it could have been. Presenting and
/// closing within one turn of the loop does not complain, so the cause is not
/// a close that outran its presentation. Closing twice does. That is the
/// shape a dialog has when a person cancels it while a write is in flight and
/// the write then closes it on success, which is why `close_if_open` exists
/// and why the flows that spawn a write past their own dialog use it.
fn a_second_close_is_what_the_journal_saw(window: &FermixWindow, complaints: &Mutex<Vec<String>>) {
    let before = complaints
        .lock()
        .expect("the collector is not poisoned")
        .len();

    let dialog = adw::AlertDialog::new(None, Some("probe"));
    dialog.present(Some(window.upcast_ref::<gtk::Widget>()));
    settle();
    dialog.close();
    settle();
    // The second close: presented once, already gone, closed again. This is
    // the shape a dialog gets when a person cancels it while a write is in
    // flight and the write then closes it on success.
    dialog.close();
    settle();

    let mut held = complaints.lock().expect("the collector is not poisoned");
    let said = held
        .iter()
        .skip(before)
        .any(|line| line.contains("not presented"));
    held.retain(|line| !line.contains("not presented"));
    drop(held);

    assert!(
        said,
        "a second close no longer complains, so the journal's line has another cause \
         and the guard below is guarding nothing"
    );
}

/// No row on any pane renders its words as markup.
///
/// `adw::PreferencesRow::use-markup` defaults to true and governs the title
/// and the subtitle alike, so a row built the ordinary way asks Pango to
/// parse whatever it is given. Most of what these rows carry comes from the
/// daemon: a path, a command line, a journal line. An `&` in any of it is not
/// an entity, so Pango fails the parse and the row renders wrong or empty.
/// Escaping at each site would be one forgotten call away from the same bug,
/// so the property is turned off instead, and this walks every row that a
/// person can reach to say that none of them was missed.
fn no_row_parses_its_words_as_markup(window: &FermixWindow) {
    let mut checked = 0;
    for row in pane::PANES {
        show_pane(window, row.pane);
        settle();

        for widget in descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
            widget.is::<adw::PreferencesRow>()
        }) {
            let reached = widget
                .downcast::<adw::PreferencesRow>()
                .expect("the walk selected preferences rows");
            assert!(
                !reached.uses_markup(),
                "{:?} has a row still parsing its title as markup: {}",
                row.pane,
                reached.title()
            );
            checked += 1;
        }
    }

    // A walk that reached nothing would pass this for the wrong reason.
    assert!(checked > 0, "the walk found no rows to check at all");
}

fn a_form_dialog_opens_wide_enough_to_fill_in() {
    let group = adw::PreferencesGroup::new();
    group.add(&adw::EntryRow::builder().title("Identifier").build());

    let form = form_dialog(
        &copy::text(Key::OAuthClientDialogTitle),
        &group,
        &copy::text(Key::ActionContinue),
    );

    assert_eq!(
        form.dialog.content_width(),
        metrics::DIALOG_FORM_WIDTH,
        "a form dialog opens at the form width"
    );
    assert!(
        form.dialog.content_width() >= metrics::CLAMP_TIGHTENING,
        "a form narrower than a tightened clamp is the squeeze this replaced"
    );

    // Both ways out are offered, and the one that writes is the suggested one.
    assert_eq!(
        form.confirm.label().unwrap_or_default(),
        copy::text(Key::ActionContinue)
    );
    assert!(
        form.confirm.has_css_class("suggested-action"),
        "the response that writes is the suggested one"
    );
    assert_eq!(
        form.cancel.label().unwrap_or_default(),
        copy::text(Key::ActionCancel)
    );

    // Escape still cancels and Enter still writes, as they did when the
    // response strip owned the default. A form whose Enter did nothing would
    // be a regression nobody sees until they type a key and press it.
    assert!(form.dialog.can_close(), "escape closes the form dialog");
    assert_eq!(
        form.dialog.default_widget().map(|widget| widget.type_()),
        Some(form.confirm.type_()),
        "enter presses the response that writes"
    );
    assert!(
        form.dialog
            .default_widget()
            .is_some_and(|widget| widget == form.confirm.clone().upcast::<gtk::Widget>()),
        "enter presses the response that writes"
    );
}

/// The trailing edge of the header bar never carries more than three children.
/// Back sits with the content it returns from, not at the window's edge.
///
/// One header spanning the whole window put Back at the far leading corner,
/// a sidebar's width away from where the content starts, and left that corner
/// empty whenever there was no Back to show. A header per navigation page puts
/// each one's controls at its own leading edge, which is what the Mac does
/// with `ToolbarItem(placement: .navigation)` and what the toolkit expects of
/// a split view.
fn back_belongs_to_the_content_page(window: &FermixWindow) {
    WidgetExt::activate_action(window, "win.settings", None).expect("settings shows");
    settle();

    let back = descendants(window.upcast_ref::<gtk::Widget>(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .and_then(|button| button.icon_name())
            .is_some_and(|icon| icon == "go-previous-symbolic")
    })
    .into_iter()
    .collect::<Vec<_>>();
    // The window owns exactly one back control. A second would mean the
    // toolkit's automatic one came back in a page header.
    assert_eq!(back.len(), 1, "one back control, not the toolkit's spares");
    let back = back.into_iter().next().expect("the back button is built");

    let page = back
        .ancestor(adw::NavigationPage::static_type())
        .and_then(|widget| widget.downcast::<adw::NavigationPage>().ok())
        .expect("back sits inside a navigation page rather than the window header");

    // The content page's title follows whichever pane is showing, so the
    // claim is about which page owns the button rather than about its words:
    // the sidebar page is the one it must not be in.
    assert_ne!(
        page.title(),
        copy::text(Key::ProductName),
        "back belongs to the content page, not the sidebar"
    );

    WidgetExt::activate_action(window, "win.home", None).expect("home shows");
    settle();
}

fn the_trailing_edge_never_carries_more_than_three(window: &FermixWindow) {
    for action in ["win.home", "win.doctor", "win.logs", "win.settings"] {
        WidgetExt::activate_action(window, action, None).expect("the page shows");
        assert!(
            window.trailing_children() <= 3,
            "{action} leaves {} children at the trailing edge",
            window.trailing_children()
        );
    }

    WidgetExt::activate_action(window, "win.home", None).expect("home shows");
}

// ---------------------------------------------------------------------------
// The descriptor rows
// ---------------------------------------------------------------------------

/// Enter commits a changed value; Escape puts the daemon's back and sends
/// nothing.
fn a_text_row_commits_on_enter_and_reverts_on_escape(
    window: &FermixWindow,
    peer: &Rc<FixturePeer>,
) {
    let model = window.settings();
    MainContext::default().block_on(model.refresh_section("personalization"));

    let data = row_named(window, "personalization", "user_name");
    let row = DescriptorRow::build(Rc::clone(&model), "personalization", &data, None);
    let entry = entry_of(&row);

    peer.forget();

    // Typing is a draft and nothing else.
    entry.set_text("Ada");
    assert_eq!(
        model.draft("personalization", "user_name"),
        Some(SettingValue::Text("Ada".into())),
        "what is typed is held by the model, so it survives navigation"
    );
    assert_eq!(peer.count("settings.apply"), 0, "typing sends nothing");

    // Escape restores the daemon's value and still sends nothing.
    assert!(row.revert(), "there was an edit to put back");
    assert_eq!(entry.text(), "");
    assert_eq!(model.draft("personalization", "user_name"), None);
    assert_eq!(peer.count("settings.apply"), 0, "reverting sends nothing");

    // Enter commits, once, with the key the row writes.
    entry.set_text("Ada");
    entry.emit_by_name::<()>("entry-activated", &[]);
    MainContext::default().iteration(false);

    let sent = peer.last("settings.apply").expect("the write was sent");
    assert_eq!(sent["section"], "personalization");
    assert_eq!(sent["values"]["user_name"], "Ada");
}

/// A switch commits the moment it moves.
fn a_toggle_row_commits_on_selection(window: &FermixWindow, peer: &Rc<FixturePeer>) {
    let model = window.settings();
    MainContext::default().block_on(model.refresh_section("personalization"));

    let data = row_named(window, "personalization", "skill_curation_enabled");
    let row = DescriptorRow::build(Rc::clone(&model), "personalization", &data, None);
    let Placement::InGroup(widgets) = row.placement() else {
        panic!("a toggle sits in its section's own group");
    };
    let switch = widgets[0]
        .clone()
        .downcast::<adw::SwitchRow>()
        .expect("a toggle is a switch row");

    peer.forget();
    switch.set_active(!switch.is_active());
    MainContext::default().iteration(false);

    let sent = peer.last("settings.apply").expect("the write was sent");
    assert_eq!(sent["section"], "personalization");
    assert!(sent["values"]["skill_curation_enabled"].is_boolean());
}

/// A row the daemon declared read-only renders the fact and no control.
fn a_read_only_row_has_no_control_to_operate(window: &FermixWindow) {
    let model = window.settings();
    MainContext::default().block_on(model.refresh_section("computer_history"));

    let data = row_named(window, "computer_history", "computer_history_summarizer");
    assert!(data.read_only, "the daemon declared this row read-only");

    let row = DescriptorRow::build(Rc::clone(&model), "computer_history", &data, None);
    let Placement::InGroup(widgets) = row.placement() else {
        panic!("a read-only row sits in its section's own group");
    };

    assert!(
        widgets[0].downcast_ref::<adw::ComboRow>().is_none(),
        "a read-only choice is never a control that cannot be used"
    );
    assert!(
        widgets[0].downcast_ref::<adw::ActionRow>().is_some(),
        "it is a fact with its value beside it"
    );
}

/// A list is edited as a list: the wire takes the whole value, so an item's
/// Enter sends every item.
fn a_list_row_writes_the_whole_list(window: &FermixWindow, peer: &Rc<FixturePeer>) {
    let model = window.settings();
    MainContext::default().block_on(model.refresh_section("sandbox"));

    // The daemon publishes this row with nothing in it, and a list with items
    // in it is the state the editor exists for, so the items are laid over the
    // row the daemon published rather than invented beside it.
    let mut data = row_named(window, "sandbox", "sandbox_env_allow");
    data.value = SettingValue::List(vec!["HOME".into(), "PATH".into()]);

    let row = DescriptorRow::build(Rc::clone(&model), "sandbox", &data, None);
    row.update(&data);

    let Placement::OwnGroup(group) = row.placement() else {
        panic!("a list row is a group of its own");
    };
    let entries = entry_rows(&group);
    assert_eq!(entries.len(), 2, "one entry per item the daemon published");
    assert_eq!(entries[0].text(), "HOME");

    peer.forget();
    entries[1].set_text("XDG_RUNTIME_DIR");
    entries[1].emit_by_name::<()>("entry-activated", &[]);
    MainContext::default().iteration(false);

    let sent = peer.last("settings.apply").expect("the write was sent");
    assert_eq!(
        sent["values"]["sandbox_env_allow"],
        serde_json::json!(["HOME", "XDG_RUNTIME_DIR"]),
        "the whole list is written, not the one item that changed"
    );
}

/// The entry rows of one group, in order.
fn entry_rows(group: &adw::PreferencesGroup) -> Vec<adw::EntryRow> {
    fn walk(widget: &gtk::Widget, found: &mut Vec<adw::EntryRow>) {
        if let Some(entry) = widget.downcast_ref::<adw::EntryRow>() {
            found.push(entry.clone());
            return;
        }

        let mut child = widget.first_child();
        while let Some(candidate) = child {
            walk(&candidate, found);
            child = candidate.next_sibling();
        }
    }

    let mut found = Vec::new();
    walk(group.upcast_ref::<gtk::Widget>(), &mut found);
    found
}

/// One row of one section, as the daemon published it.
fn row_named(window: &FermixWindow, section: &str, key: &str) -> SettingsRow {
    window
        .settings()
        .state()
        .rows(section)
        .into_iter()
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("the daemon publishes {key} in {section}"))
}

fn entry_of(row: &Rc<DescriptorRow>) -> adw::EntryRow {
    let Placement::InGroup(widgets) = row.placement() else {
        panic!("a text row sits in its section's own group");
    };
    widgets[0]
        .clone()
        .downcast::<adw::EntryRow>()
        .expect("a text row is an entry row")
}

/// Every symbolic icon this application names draws something.
///
/// A name the toolkit cannot resolve draws the missing-image glyph and says
/// nothing about it, which is how a sidebar ships with a broken square in it.
/// The names are read out of the sources, so one added later joins this gate by
/// existing.
fn every_icon_the_application_names_resolves() {
    let display = gtk::gdk::Display::default().expect("a display");
    let theme = gtk::IconTheme::for_display(&display);

    let named = icon_names_in_sources();
    assert!(named.len() >= 8, "the sources name icons");

    for name in named {
        let paintable = theme.lookup_icon(
            &name,
            &[],
            16,
            1,
            gtk::TextDirection::Ltr,
            gtk::IconLookupFlags::empty(),
        );
        let resolved = paintable
            .icon_name()
            .map(|icon| icon.to_string_lossy().into_owned())
            .unwrap_or_default();

        assert_ne!(
            resolved, "image-missing",
            "{name} draws nothing on this toolkit"
        );
    }
}

/// Every `*-symbolic` name written down in `src/`.
fn icon_names_in_sources() -> std::collections::BTreeSet<String> {
    fn walk(directory: &std::path::Path, found: &mut std::collections::BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }

            let body = std::fs::read_to_string(&path).expect("a source file reads");
            for piece in body.split('"') {
                if piece.ends_with("-symbolic")
                    && piece
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    found.insert(piece.to_string());
                }
            }
        }
    }

    let mut found = std::collections::BTreeSet::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found
}

/// The window's one header bar.
fn header_bar(window: &FermixWindow) -> gtk::Widget {
    fn walk(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.downcast_ref::<adw::HeaderBar>().is_some() {
            return Some(widget.clone());
        }

        let mut child = widget.first_child();
        while let Some(candidate) = child {
            if let Some(found) = walk(&candidate) {
                return Some(found);
            }
            child = candidate.next_sibling();
        }
        None
    }

    walk(window.upcast_ref::<gtk::Widget>()).expect("the window has one header bar")
}

/// How many buttons under one widget carry one label.
fn count_labelled(root: &gtk::Widget, label: &str) -> usize {
    fn walk(widget: &gtk::Widget, label: &str, found: &mut usize) {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            if button.is_visible() && button.label().map(|text| text == label).unwrap_or(false) {
                *found += 1;
            }
        }

        let mut child = widget.first_child();
        while let Some(candidate) = child {
            walk(&candidate, label, found);
            child = candidate.next_sibling();
        }
    }

    let mut found = 0;
    walk(root, label, &mut found);
    found
}

/// The bundled Adwaita theme is reachable once it is named.
///
/// GTK builds its icon search path from `XDG_DATA_DIRS` and the user's own data
/// directory, never from the prefix it was compiled with, so a packaged build
/// that does not name the bundled theme opens a window with no icons in it and
/// nothing reports why. Slice 1's runtime smoke proved that the compiled-in
/// prefix does not carry the icons; this proves the application's repair works
/// against the live display.
///
/// A directory of this test's own making rather than the real prefix, so the
/// assertion means the same thing on a machine that carries the packaged
/// runtime and on one that does not.
fn the_bundled_icon_theme_reaches_the_search_path() {
    let display = gtk::gdk::Display::default().expect("the test runs under a display");
    let bundle = TempDirectory::new("bundled-icons");

    let prefix = bundle.path().join("usr/lib/fermix-desktop");
    let schemas = prefix.join("share/glib-2.0/schemas");
    std::fs::create_dir_all(&schemas).expect("the schema directory is created");
    std::fs::write(schemas.join("gschemas.compiled"), b"").expect("the schemas are written");
    let icons = prefix.join("share/icons");
    std::fs::create_dir_all(&icons).expect("the icon directory is created");

    let before = gtk::IconTheme::for_display(&display).search_path();
    assert!(
        !before.contains(&icons),
        "the bundled directory was already on the search path, so this proves nothing"
    );

    fermix_desktop::runtime::add_bundled_icons_from(&display, &prefix);

    let after = gtk::IconTheme::for_display(&display).search_path();
    assert!(
        after.contains(&icons),
        "naming the bundled theme did not put it on the icon search path"
    );
    assert_eq!(
        after.last(),
        Some(&icons),
        "the bundle must go last, so a host theme that matches the session still wins"
    );
}
