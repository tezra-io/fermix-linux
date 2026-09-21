//! The structural gates.
//!
//! These are the rules a reviewer would otherwise have to hold in their head:
//! the application paints nothing, sizes no type, draws no container, keeps one
//! split view and one settings model, and never writes down a word the daemon
//! owns.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fermix_desktop::metrics;
use serde_json::Value;

fn manifest_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sources() -> Vec<(PathBuf, String)> {
    rust_files(&manifest_directory().join("src"))
        .into_iter()
        .map(|path| {
            let body = std::fs::read_to_string(&path).expect("a source file reads");
            (path, body)
        })
        .collect()
}

/// The part of a file that ships. A unit test may quote a wire envelope
/// verbatim, because quoting one is how it proves the envelope decodes; the
/// gates below are about what a person can see.
fn shipped(body: &str) -> &str {
    match body.find("#[cfg(test)]") {
        Some(at) => &body[..at],
        None => body,
    }
}

#[test]
fn every_file_keeps_its_tests_in_one_block_at_the_end() {
    // The gates cut a file at its first `#[cfg(test)]`, so a second block
    // would hide shipped code from them.
    for (path, body) in sources() {
        assert!(
            body.matches("#[cfg(test)]").count() <= 1,
            "{} carries more than one test block",
            relative(&path)
        );
    }
}

fn rust_files(directory: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
    found
}

fn relative(path: &Path) -> String {
    path.strip_prefix(manifest_directory())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn stylesheet() -> String {
    std::fs::read_to_string(manifest_directory().join("resources/style.css"))
        .expect("the stylesheet reads")
}

/// The stylesheet without its comments, so a comment explaining why a property
/// is absent does not read as the property being present.
fn declarations(css: &str) -> String {
    let mut kept = String::with_capacity(css.len());
    let mut rest = css;

    while let Some(open) = rest.find("/*") {
        kept.push_str(&rest[..open]);
        match rest[open..].find("*/") {
            Some(close) => rest = &rest[open + close + 2..],
            None => return kept,
        }
    }
    kept.push_str(rest);
    kept
}

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

#[test]
fn no_colour_literal_appears_outside_the_icons_and_the_brand() {
    let mut offenders: Vec<String> = Vec::new();

    for (path, body) in sources().into_iter().chain([(
        manifest_directory().join("resources/style.css"),
        stylesheet(),
    )]) {
        for (number, line) in body.lines().enumerate() {
            if carries_a_colour(line) {
                offenders.push(format!(
                    "{}:{}: {}",
                    relative(&path),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "every colour is a libadwaita variable resolved by the platform: {offenders:#?}"
    );
}

/// A hexadecimal colour, or one of the CSS colour functions. Brand blue lives
/// on the icon and the wordmark and nowhere else, so neither is scanned.
fn carries_a_colour(line: &str) -> bool {
    if line.contains("rgb(") || line.contains("rgba(") || line.contains("hsl(") {
        return true;
    }

    let bytes: Vec<char> = line.chars().collect();
    for (index, character) in bytes.iter().enumerate() {
        if *character != '#' {
            continue;
        }
        let run = bytes[index + 1..]
            .iter()
            .take_while(|c| c.is_ascii_hexdigit())
            .count();
        let terminated = bytes
            .get(index + 1 + run)
            .is_none_or(|c| !c.is_alphanumeric() && *c != '_');
        if (3..=8).contains(&run) && terminated {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Type and containers
// ---------------------------------------------------------------------------

#[test]
fn the_stylesheet_sizes_no_type_and_draws_no_container() {
    let css = declarations(&stylesheet());
    let forbidden = [
        "font-size",
        "font-family",
        "font-weight",
        "background",
        "border",
        "box-shadow",
        "outline",
        "color:",
    ];

    for property in forbidden {
        assert!(
            !css.contains(property),
            "the stylesheet carries {property}; the toolkit owns it"
        );
    }
}

#[test]
fn the_application_draws_no_container_of_its_own() {
    let forbidden = [
        "gtk::Frame",
        "GtkFrame",
        "gtk::Separator",
        "gtk::DrawingArea",
    ];

    for (path, body) in sources() {
        for widget in forbidden {
            assert!(
                !shipped(&body).contains(widget),
                "{} constructs {widget}; libadwaita draws every box, hairline and separator",
                relative(&path)
            );
        }
    }
}

#[test]
fn the_stylesheet_repeats_only_numbers_the_metrics_module_owns() {
    let css = stylesheet();

    assert!(
        css.contains(&format!("min-width: {}px", metrics::ARTWORK_SLOT)),
        "the artwork slot in the stylesheet is not the one metrics declares"
    );
    assert!(
        css.contains(&format!("padding-top: {}px", metrics::ARRANGED_ROW_PADDING)),
        "the arranged-row padding in the stylesheet is not the one metrics declares"
    );
}

// ---------------------------------------------------------------------------
// One of each
// ---------------------------------------------------------------------------

#[test]
fn exactly_one_navigation_split_view_is_constructed() {
    let constructions: usize = sources()
        .iter()
        .map(|(_, body)| {
            body.matches("NavigationSplitView::new").count()
                + body.matches("NavigationSplitView::builder").count()
                + body.matches("adw::NavigationSplitView::default").count()
                + body.matches("pub split: adw::NavigationSplitView").count()
        })
        .sum();

    assert_eq!(
        constructions, 1,
        "there is one split view per window and one window"
    );
}

#[test]
fn exactly_one_settings_model_is_constructed() {
    let constructions: usize = sources()
        .iter()
        .map(|(_, body)| shipped(body).matches("SettingsModel::new").count())
        .sum();

    assert_eq!(
        constructions, 1,
        "the settings model is constructed once, in the application composition"
    );
}

#[test]
fn there_is_one_action_map_and_the_window_adds_every_window_action_in_it() {
    let window = std::fs::read_to_string(manifest_directory().join("src/window.rs"))
        .expect("the window source reads");
    let application = std::fs::read_to_string(manifest_directory().join("src/app.rs"))
        .expect("the application source reads");

    for name in fermix_desktop::actions::window_action_names() {
        assert!(
            window.contains(&format!("\"{name}\"")),
            "the window does not add the {name} action the table declares"
        );
    }
    for name in fermix_desktop::actions::application_action_names() {
        assert!(
            application.contains(&format!("SimpleAction::new(\"{name}\"")),
            "the application does not add the {name} action the table declares"
        );
    }
}

// ---------------------------------------------------------------------------
// No word the daemon owns
// ---------------------------------------------------------------------------

/// The layers that decode the wire and therefore have to name its atoms, plus
/// the two development files that name fixture scenarios after the states those
/// goldens publish: the loader that serves them and the capture mode that walks
/// them, which is compiled only in a debug build. Everywhere else, an atom in a
/// literal means the application has started keeping a copy of a vocabulary the
/// daemon owns.
const DECODING_LAYERS: &[&str] = &[
    "src/management/",
    "src/service/",
    "src/bin/",
    "src/fixtures.rs",
    "src/capture.rs",
];

#[test]
fn no_wire_atom_is_written_down_outside_the_decoding_layers() {
    let atoms = [
        "setup_required",
        "external_change",
        "config_unreadable",
        "not_applicable",
        "needs_secret",
        "reauthorization_required",
        "wrong_region",
        "insufficient_credential_scope",
        "pending_restart",
        "ownership_conflict",
        "cursor_expired",
        "secret_store_failed",
    ];

    for (path, body) in sources() {
        let where_it_is = relative(&path);
        if DECODING_LAYERS
            .iter()
            .any(|layer| where_it_is.starts_with(layer))
        {
            continue;
        }
        for atom in atoms {
            assert!(
                !shipped(&body).contains(atom),
                "{where_it_is} writes down the wire atom {atom}"
            );
        }
    }
}

#[test]
fn no_refusal_sentence_the_daemon_owns_appears_in_the_sources() {
    let sentences = published_error_messages();
    assert!(
        sentences.len() >= 10,
        "the error goldens are unexpectedly few"
    );

    for (path, body) in sources() {
        for sentence in &sentences {
            assert!(
                !shipped(&body).contains(sentence.as_str()),
                "{} writes down the daemon's own refusal: {sentence}",
                relative(&path)
            );
        }
    }
}

#[test]
fn no_remediation_title_the_daemon_owns_appears_in_the_sources() {
    let titles = published_remediation_titles();
    assert!(
        !titles.is_empty(),
        "the goldens carry no remediation to check against"
    );

    for (path, body) in sources() {
        for title in &titles {
            assert!(
                !shipped(&body).contains(title.as_str()),
                "{} writes down the daemon's own remediation title: {title}",
                relative(&path)
            );
        }
    }
}

fn published_error_messages() -> BTreeSet<String> {
    goldens("management/fixtures/errors.jsonl")
        .into_iter()
        .filter_map(|record| {
            record["response"]["error"]["message"]
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

fn published_remediation_titles() -> BTreeSet<String> {
    let mut titles = BTreeSet::new();
    for record in goldens("management/fixtures/success.jsonl") {
        let Some(checks) = record["response"]["result"]["checks"].as_array() else {
            continue;
        };
        for check in checks {
            if let Some(title) = check["remediation"]["title"].as_str() {
                titles.insert(title.to_string());
            }
        }
    }
    titles
}

fn goldens(relative: &str) -> Vec<Value> {
    let path = manifest_directory().join("contracts").join(relative);
    std::fs::read_to_string(&path)
        .expect("a golden file reads")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("a golden parses"))
        .collect()
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

#[test]
fn the_one_secret_entry_lives_in_the_one_dialog_that_owns_it() {
    let mut holders: Vec<String> = Vec::new();

    for (path, body) in sources() {
        if shipped(&body).contains("PasswordEntry") {
            holders.push(relative(&path));
        }
    }

    assert_eq!(
        holders,
        vec!["src/ui/settings/dialogs/secret.rs".to_string()],
        "a secret is typed in one place and nowhere else"
    );
}

#[test]
fn no_surface_reads_a_secret_back() {
    // The wire carries secrets inbound only, so nothing above the client has a
    // value to read: a surface asking for one would be asking for something no
    // result carries.
    for (path, body) in sources() {
        let where_it_is = relative(&path);
        if !where_it_is.starts_with("src/ui/") {
            continue;
        }
        assert!(
            !shipped(&body).contains("secret.get"),
            "{where_it_is} asks for a secret value"
        );
    }
}

// ---------------------------------------------------------------------------
// The two-door registry
// ---------------------------------------------------------------------------

/// The files that draw a hand-built surface: the seven screens of the Setup
/// assistant, the six panes the design names, the one descriptor pane that
/// carries statements over its controls, and every dialog. A descriptor pane is
/// not hand-built and is not in the registry: it is the daemon's rows through
/// the one form.
const HAND_BUILT: &[&str] = &[
    "src/ui/onboarding/welcome.rs",
    "src/ui/onboarding/starting.rs",
    "src/ui/onboarding/connect_ai.rs",
    "src/ui/onboarding/about_you.rs",
    "src/ui/onboarding/applying.rs",
    "src/ui/onboarding/ready.rs",
    "src/ui/onboarding/boot_failed.rs",
    "src/ui/settings/providers.rs",
    "src/ui/settings/channels.rs",
    "src/ui/settings/integrations.rs",
    "src/ui/settings/meetings.rs",
    "src/ui/settings/computer.rs",
    "src/ui/settings/permissions.rs",
    "src/ui/settings/voice.rs",
    "src/ui/settings/dialogs/consent.rs",
    "src/ui/settings/dialogs/model_picker.rs",
    "src/ui/settings/dialogs/oauth_client.rs",
    "src/ui/settings/dialogs/restart.rs",
    "src/ui/settings/dialogs/secret.rs",
    "src/ui/settings/dialogs/sign_in.rs",
    "src/ui/settings/dialogs/workspace.rs",
];

#[test]
fn every_hand_built_surface_has_a_registry_entry() {
    for file in HAND_BUILT {
        let entry = fermix_desktop::registry::surface(file).unwrap_or_else(|| {
            panic!("{file} draws a hand-built surface and has no registry entry")
        });
        assert!(
            entry.counterpart.is_some() || entry.exemption.is_some(),
            "{file} names neither a counterpart nor an exemption"
        );
    }
}

#[test]
fn the_registry_names_no_surface_that_does_not_exist() {
    for surface in fermix_desktop::registry::SURFACES {
        let path = manifest_directory().join(surface.file);
        assert!(
            path.is_file(),
            "the registry names {}, which nothing draws",
            surface.file
        );
        assert!(
            HAND_BUILT.contains(&surface.file),
            "{} is in the registry and is not a hand-built surface",
            surface.file
        );
    }
}

#[test]
fn every_pane_and_dialog_that_draws_is_hand_built_or_a_form() {
    // A file under ui/settings that is neither the shared machinery nor a
    // registered hand-built surface is a surface the registry stopped covering.
    let shared = [
        "src/ui/onboarding/mod.rs",
        "src/ui/settings/mod.rs",
        "src/ui/settings/pane_list.rs",
        "src/ui/settings/descriptor_form.rs",
        "src/ui/settings/descriptor_row.rs",
        "src/ui/settings/dialogs/mod.rs",
    ];

    for (path, _) in sources() {
        let file = relative(&path);
        if !file.starts_with("src/ui/settings/") && !file.starts_with("src/ui/onboarding/") {
            continue;
        }
        assert!(
            shared.contains(&file.as_str()) || HAND_BUILT.contains(&file.as_str()),
            "{file} draws and is in neither the shared machinery nor the registry"
        );
    }
}

// ---------------------------------------------------------------------------
// Every word a person reads comes from the catalogue
// ---------------------------------------------------------------------------

/// The surfaces a person looks at. A user-facing string in one of these is a
/// literal that never reached the catalogue, never reached the casing column
/// and never reached a translator. Every file that draws is listed; a surface
/// added without joining this list is a surface this gate stopped covering.
const SURFACE_FILES: &[&str] = &[
    "src/window.rs",
    "src/app.rs",
    "src/ui/mod.rs",
    "src/ui/home.rs",
    "src/ui/doctor.rs",
    "src/ui/logs.rs",
    "src/ui/recovery.rs",
    "src/ui/widgets/mod.rs",
    "src/ui/widgets/checklist_row.rs",
    "src/ui/widgets/mark.rs",
    "src/ui/widgets/status_pill.rs",
    "src/ui/onboarding/mod.rs",
    "src/ui/onboarding/welcome.rs",
    "src/ui/onboarding/starting.rs",
    "src/ui/onboarding/connect_ai.rs",
    "src/ui/onboarding/about_you.rs",
    "src/ui/onboarding/applying.rs",
    "src/ui/onboarding/ready.rs",
    "src/ui/onboarding/boot_failed.rs",
    "src/ui/settings/mod.rs",
    "src/ui/settings/dialogs/mod.rs",
    "src/ui/settings/pane_list.rs",
    "src/ui/settings/descriptor_form.rs",
    "src/ui/settings/descriptor_row.rs",
    "src/ui/settings/channels.rs",
    "src/ui/settings/computer.rs",
    "src/ui/settings/integrations.rs",
    "src/ui/settings/meetings.rs",
    "src/ui/settings/permissions.rs",
    "src/ui/settings/providers.rs",
    "src/ui/settings/voice.rs",
    "src/ui/settings/dialogs/consent.rs",
    "src/ui/settings/dialogs/model_picker.rs",
    "src/ui/settings/dialogs/oauth_client.rs",
    "src/ui/settings/dialogs/restart.rs",
    "src/ui/settings/dialogs/secret.rs",
    "src/ui/settings/dialogs/sign_in.rs",
    "src/ui/settings/dialogs/workspace.rs",
];

#[test]
fn every_file_that_draws_is_covered_by_the_literal_gate() {
    let drawing: Vec<String> = sources()
        .iter()
        .map(|(path, _)| relative(path))
        .filter(|path| path.starts_with("src/ui/"))
        .collect();

    for path in drawing {
        assert!(
            SURFACE_FILES.contains(&path.as_str()),
            "{path} draws and is not in SURFACE_FILES"
        );
    }
}

#[test]
fn every_word_a_surface_shows_comes_from_the_catalogue() {
    for file in SURFACE_FILES {
        let body =
            std::fs::read_to_string(manifest_directory().join(file)).expect("a surface reads");

        for (number, line) in body.lines().enumerate() {
            if let Some(literal) = user_facing_literal(line) {
                panic!("{file}:{}: {literal} is not a catalogue key", number + 1);
            }
        }
    }
}

/// A string literal handed to a widget property that a person reads. Icon
/// names, action names, CSS classes, stack page names and resource paths are
/// identifiers rather than words, and they are named as such here.
fn user_facing_literal(line: &str) -> Option<String> {
    let setters = [
        ".title(\"",
        ".subtitle(\"",
        ".label(\"",
        ".description(\"",
        ".text(\"",
    ];

    for setter in setters {
        if let Some(at) = line.find(setter) {
            let rest = &line[at + setter.len()..];
            let literal = rest.split('"').next().unwrap_or_default();
            if !literal.is_empty() {
                return Some(format!("\"{literal}\""));
            }
        }
    }
    None
}

/// Every action the tray menu names is one the application registers.
///
/// The tray's rows are a table of action names, and the actions are registered
/// somewhere else, so the two can drift without either file looking wrong. When
/// they drift the failure is quiet in the worst way: the icon appears, the menu
/// opens, the row is there, and clicking it does nothing at all. A person would
/// report that as "the tray is broken" and the cause would be a typed string.
///
/// This is a source-level check on purpose. Activating the real actions needs a
/// running application with a window, which is the one thing the container lane
/// cannot have; the names, though, are readable without any of that.
#[test]
fn every_action_the_tray_offers_is_one_the_application_registers() {
    let directory = manifest_directory();

    let menu_body = std::fs::read_to_string(directory.join("src/tray/menu.rs"))
        .expect("the tray menu table reads");
    let app = std::fs::read_to_string(directory.join("src/app.rs")).expect("the application reads");

    // Only the part that ships. The table's own tests assert on the shape of an
    // action name (`starts_with("app.")`), and reading those as rows finds an
    // action with an empty name that nothing could ever register.
    let menu = shipped(&menu_body);

    // The names the table hands to the bus, as `"app.something"`.
    let named: BTreeSet<String> = menu
        .split("\"app.")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect();

    assert!(
        !named.is_empty(),
        "no application actions were found in the tray menu table, so this test read nothing"
    );

    for action in &named {
        let registered = app.contains(&format!("SimpleAction::new(\"{action}\""));
        assert!(
            registered,
            "the tray offers app.{action}, which the application never registers: \
             the row would open, click, and do nothing"
        );
    }
}

/// The control for the test above: it is looking, not agreeing.
///
/// A name the table does not carry must not be found in it. Without this, a
/// membership test that had stopped reading the table would pass exactly as
/// happily as one that read it.
#[test]
fn an_action_the_tray_does_not_offer_is_not_found_in_its_table() {
    let menu = std::fs::read_to_string(manifest_directory().join("src/tray/menu.rs"))
        .expect("the tray menu table reads");

    assert!(
        !shipped(&menu).contains("\"app.tray-no-such-row\""),
        "the table was read as carrying a row that does not exist in it"
    );
}

/// Every row construction in the surfaces goes through `plain`.
///
/// The walking gate in tests/widgets.rs covers the rows a pane shows, and it
/// reaches around three thousand of them. It cannot reach a row that only
/// exists inside a dialog or in the onboarding flow, because those are not
/// mapped while it walks. This reads the source instead, so a row built
/// anywhere under src/ui is held to the rule whether or not a test can open
/// the surface it lives on.
///
/// `ButtonRow` and a bare `PreferencesRow` are named explicitly: they are
/// PreferencesRow subclasses carrying daemon text, and the first mechanical
/// pass missed both.
#[test]
fn every_row_the_surfaces_build_is_plain() {
    let missed = rows_built_without_plain();
    assert!(
        missed.is_empty(),
        "these rows are built without plain(), so they parse their words as markup: {missed:#?}"
    );
}

/// The rule above, put to a case that must fail it.
///
/// `every_row_the_surfaces_build_is_plain` can only ever be read green, and a
/// green that would survive the rule being deleted says nothing. This feeds
/// the same matcher a construction it must object to, and one it must not.
#[test]
fn the_plain_rule_objects_to_a_row_built_without_it() {
    let offending = "let row = adw::ButtonRow::builder().title(\"x\").build();";
    assert_eq!(
        rows_without_plain_in(offending).len(),
        1,
        "the rule let an unwrapped row through"
    );

    let obedient = "let row = plain(adw::ButtonRow::builder().title(\"x\").build());";
    assert!(
        rows_without_plain_in(obedient).is_empty(),
        "the rule objected to a row that obeys it"
    );
}

/// Every row construction under src/ui that is not wrapped in `plain`.
fn rows_built_without_plain() -> Vec<String> {
    sources()
        .into_iter()
        .filter(|(path, _)| path.components().any(|part| part.as_os_str() == "ui"))
        .flat_map(|(path, body)| {
            rows_without_plain_in(shipped(&body))
                .into_iter()
                .map(move |found| format!("{}: {found}", path.display()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The matcher itself, over one piece of source text.
fn rows_without_plain_in(body: &str) -> Vec<String> {
    const ROWS: [&str; 9] = [
        "ActionRow",
        "EntryRow",
        "SwitchRow",
        "ComboRow",
        "SpinRow",
        "ExpanderRow",
        "PasswordEntryRow",
        "ButtonRow",
        "PreferencesRow",
    ];

    // Only a construction counts. `adw::PreferencesRow` also appears as a
    // type bound — `plain` is generic over it — and a bound builds nothing.
    const MAKERS: [&str; 3] = ["builder(", "new(", "with_range("];

    let mut missed = Vec::new();
    for row in ROWS {
        let needle = format!("adw::{row}::");
        let mut from = 0;
        while let Some(at) = body[from..].find(&needle) {
            let at = from + at;
            from = at + needle.len();
            let tail = &body[from..body.len().min(from + MAKER_REACH)];
            if !MAKERS.iter().any(|maker| tail.starts_with(maker)) {
                continue;
            }
            // `plain(` may sit immediately before it or on the line above,
            // because rustfmt moves the call onto its own line once the
            // construction is long enough to wrap.
            let before = &body[at.saturating_sub(PLAIN_REACH)..at];
            if !before.contains("plain(") {
                missed.push(body[at..from].to_string());
            }
        }
    }
    missed
}

/// How far back the wrapper may sit, in bytes: enough for `plain(` plus the
/// newline and indentation rustfmt puts between it and the constructor.
const PLAIN_REACH: usize = 40;

/// How much of what follows the path is read to tell a construction from a
/// type bound.
const MAKER_REACH: usize = 12;
