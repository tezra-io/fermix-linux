//! The copy gates.
//!
//! One catalogue, one row per key, one casing class per row. These are the
//! checks that keep the deck honest: every key says something, no key says
//! something forbidden, a header row reads as a header and a sentence row reads
//! as a sentence, every template declares its markers, and no key sits in the
//! catalogue with nothing rendering it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fermix_desktop::copy::{self, Casing, Key, CATALOGUE, TEMPLATED};

fn manifest_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ---------------------------------------------------------------------------
// The catalogue is whole
// ---------------------------------------------------------------------------

#[test]
fn the_catalogue_holds_exactly_one_row_per_key_in_key_order() {
    assert_eq!(
        CATALOGUE.len(),
        Key::Count as usize,
        "a key was declared without a row, or a row without a key"
    );

    for (index, entry) in CATALOGUE.iter().enumerate() {
        assert_eq!(
            entry.key as usize, index,
            "{:?} sits at row {index}, which is not its own position",
            entry.key
        );
    }
}

#[test]
fn every_key_says_something() {
    for entry in CATALOGUE {
        let rendered = copy::text(entry.key);
        assert!(
            !rendered.trim().is_empty(),
            "{:?} renders nothing",
            entry.key
        );
    }
}

// ---------------------------------------------------------------------------
// Casing, per class
// ---------------------------------------------------------------------------

#[test]
fn a_header_row_renders_in_header_capitalization() {
    for entry in CATALOGUE
        .iter()
        .filter(|entry| entry.casing == Casing::Header)
    {
        assert_eq!(
            copy::text(entry.key),
            copy::header_capitalized(entry.source),
            "{:?} does not render as a header",
            entry.key
        );
    }
}

#[test]
fn a_header_row_never_ends_in_a_period() {
    for entry in CATALOGUE
        .iter()
        .filter(|entry| entry.casing == Casing::Header)
    {
        assert!(
            !copy::text(entry.key).ends_with('.'),
            "{:?} is a header and ends in a period",
            entry.key
        );
    }
}

/// The proper nouns a sentence may capitalize mid-sentence. Hand-listed, as the
/// macOS door lists its own: an unlisted capital is either a sentence written
/// in Header Capitalization or a name nobody reviewed, and both are worth a
/// failing test.
const PROPER_NOUNS: &[&str] = &[
    "AI", "Claude", "Code", "Codex", "Discord", "Fermix", "Google", "Home", "Linux", "MCPs",
    "Meet", // A unit symbol, which is not a word and is not lowercased mid-sentence.
    "MB", "PipeWire", "Slack", "Telegram", "Ubuntu", "Wayland", "X11", "Zoom",
];

#[test]
fn a_sentence_row_is_not_header_capitalized() {
    for entry in CATALOGUE
        .iter()
        .filter(|entry| entry.casing == Casing::Sentence)
    {
        let words: Vec<&str> = entry.source.split_whitespace().collect();
        let mut opens_a_sentence = true;

        for word in words {
            let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
            let capitalized = bare.chars().next().is_some_and(|c| c.is_uppercase());

            assert!(
                !capitalized || opens_a_sentence || PROPER_NOUNS.contains(&bare),
                "{:?} capitalizes {bare} mid-sentence: either the row is written in Header \
                 Capitalization, or {bare} is a name the gate has not been told about",
                entry.key
            );

            opens_a_sentence = word.ends_with(['.', ':', '?', ';']) || word.starts_with('{');
        }
    }
}

#[test]
fn a_sentence_row_renders_exactly_as_it_is_written() {
    for entry in CATALOGUE
        .iter()
        .filter(|entry| entry.casing == Casing::Sentence)
    {
        assert_eq!(
            copy::text(entry.key),
            entry.source,
            "{:?} was re-cased",
            entry.key
        );
    }
}

// ---------------------------------------------------------------------------
// Forbidden
// ---------------------------------------------------------------------------

/// Substrings that may not appear anywhere in the catalogue.
const FORBIDDEN_SUBSTRINGS: &[(&str, &str)] = &[
    ("\u{2014}", "an em dash"),
    ("\u{2013}", "an en dash"),
    ("!", "an exclamation mark"),
    ("please wait", "\"please wait\""),
    ("...", "three periods where the ellipsis is U+2026"),
    ("config.toml", "the settings file by name"),
    ("lorem ipsum", "placeholder text"),
    ("coming soon", "placeholder text"),
];

/// Whole words that may not appear anywhere in the catalogue. They are checked
/// as words rather than as substrings because "Fermix" carries "mix" and a
/// substring check on that one silently forbids the product's own name.
const FORBIDDEN_WORDS: &[(&str, &str)] = &[
    ("mix", "a mix task"),
    ("tbd", "placeholder text"),
    ("todo", "placeholder text"),
    ("placeholder", "placeholder text"),
    ("lorem", "placeholder text"),
    ("ipsum", "placeholder text"),
];

#[test]
fn nothing_forbidden_appears_in_the_catalogue() {
    for entry in CATALOGUE {
        let lowered = entry.source.to_lowercase();
        for (needle, what) in FORBIDDEN_SUBSTRINGS {
            assert!(
                !lowered.contains(needle),
                "{:?} carries {what}: {}",
                entry.key,
                entry.source
            );
        }

        let words: Vec<&str> = lowered.split(|c: char| !c.is_alphanumeric()).collect();
        for (word, what) in FORBIDDEN_WORDS {
            assert!(
                !words.contains(word),
                "{:?} carries {what}: {}",
                entry.key,
                entry.source
            );
        }
    }
}

#[test]
fn no_control_label_is_save_apply_or_submit() {
    for entry in CATALOGUE {
        let rendered = copy::text(entry.key);
        for forbidden in ["Save", "Apply", "Submit"] {
            assert_ne!(
                rendered, forbidden,
                "{:?} labels a control {forbidden}",
                entry.key
            );
        }
    }
}

#[test]
fn no_environment_variable_name_reaches_a_person() {
    for entry in CATALOGUE {
        for word in entry
            .source
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        {
            let shaped_like_a_variable = word.len() >= 4
                && word.contains('_')
                && word
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            assert!(
                !shaped_like_a_variable,
                "{:?} names the environment variable {word}",
                entry.key
            );
        }
    }
}

/// The atoms the wire carries. A word from this list in the catalogue means the
/// application has started keeping its own copy of a vocabulary the daemon
/// owns.
const WIRE_ATOMS: &[&str] = &[
    "setup_required",
    "external_change",
    "config_unreadable",
    "restart_pending",
    "not_applicable",
    "needs_secret",
    "needs_auth",
    "needs_workspace",
    "wrong_region",
    "pending_restart",
    "ownership_conflict",
    "local_stdio",
    "remote_mcp",
    "invalid_params",
    "method_not_found",
    "secret_store_failed",
    "cursor_expired",
];

#[test]
fn no_wire_atom_is_used_as_a_word() {
    for entry in CATALOGUE {
        for atom in WIRE_ATOMS {
            assert!(
                !entry.source.contains(atom),
                "{:?} renders the wire atom {atom}",
                entry.key
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

#[test]
fn every_template_declares_its_markers() {
    for entry in CATALOGUE {
        let found = markers(entry.source);
        let declared: Vec<String> = TEMPLATED
            .iter()
            .find(|(key, _)| *key == entry.key)
            .map(|(_, markers)| markers.iter().map(|m| m.to_string()).collect())
            .unwrap_or_default();

        assert_eq!(
            found, declared,
            "{:?} carries markers the template table does not declare",
            entry.key
        );
    }
}

#[test]
fn a_declared_template_names_a_key_that_exists() {
    for (key, markers) in TEMPLATED {
        assert!(!markers.is_empty(), "{key:?} declares no markers");
        assert!(
            (*key as usize) < CATALOGUE.len(),
            "{key:?} is not a catalogue key"
        );
    }
}

#[test]
fn filling_a_template_leaves_no_marker_behind() {
    for (key, markers) in TEMPLATED {
        let values: Vec<(&str, &str)> = markers.iter().map(|marker| (*marker, "x")).collect();
        let filled = copy::fill(*key, &values);
        for marker in *markers {
            assert!(!filled.contains(marker), "{key:?} still carries {marker}");
        }
    }
}

/// Every `{name}` and `<name>` in a source, in the order they appear.
fn markers(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = source;

    while let Some((open, close)) = next_marker(rest) {
        let end = close + 1;
        found.push(rest[open..end].to_string());
        rest = &rest[end..];
    }

    found
}

fn next_marker(source: &str) -> Option<(usize, usize)> {
    for (open, character) in source.char_indices() {
        let closer = match character {
            '{' => '}',
            '<' => '>',
            _ => continue,
        };
        if let Some(offset) = source[open..].find(closer) {
            return Some((open, open + offset));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Nothing sits in the catalogue with nothing rendering it
// ---------------------------------------------------------------------------

#[test]
fn every_key_is_rendered_somewhere_or_declared_as_a_surface_not_yet_built() {
    let used = keys_used_in_sources();
    let declared = pending_keys();

    let unused: BTreeSet<String> = CATALOGUE
        .iter()
        .map(|entry| format!("{:?}", entry.key))
        .filter(|name| !used.contains(name))
        .collect();

    let stale: Vec<&String> = declared.difference(&unused).collect();
    assert!(
        stale.is_empty(),
        "these keys are rendered now and should be removed from {}: {stale:?}",
        PENDING_FILE
    );

    let undeclared: Vec<&String> = unused.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "these keys render nowhere and are not declared in {}: {undeclared:?}",
        PENDING_FILE
    );
}

/// The keys whose surface has not been built yet. It shrinks slice by slice and
/// is empty when the product is whole; a key that leaves it and a key that
/// enters it both fail the gate above until the file is updated deliberately.
const PENDING_FILE: &str = "tests/fixtures/copy/keys_awaiting_a_surface.txt";

fn pending_keys() -> BTreeSet<String> {
    let path = manifest_directory().join(PENDING_FILE);
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));

    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn keys_used_in_sources() -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    let sources = manifest_directory().join("src");

    for path in rust_files(&sources) {
        // The catalogue itself names every key by construction, so it cannot
        // be the thing that proves a key is rendered.
        if path.ends_with("copy.rs") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("a source file reads");
        collect_keys(&body, &mut used);
    }

    used
}

fn collect_keys(body: &str, into: &mut BTreeSet<String>) {
    let mut rest = body;
    while let Some(at) = rest.find("Key::") {
        rest = &rest[at + "Key::".len()..];
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            into.insert(rest[..end].to_string());
        }
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

// ---------------------------------------------------------------------------
// The words the design fixes verbatim
// ---------------------------------------------------------------------------

#[test]
fn the_linux_only_moments_carry_their_fixed_words() {
    let fixed = [
        (
            Key::LingerDeniedTitle,
            "Fermix cannot run in the background yet",
        ),
        (
            Key::LoginManagerAbsentTitle,
            "This host has no login manager",
        ),
        (
            Key::SecretStoreDesktopKeyring,
            "Stored in your desktop keyring",
        ),
        (
            Key::SecretStorePasswordStore,
            "Stored in your password store",
        ),
        (Key::SecretStoreNone, "No store on this host"),
        (
            Key::SecretStoreUnavailableTitle,
            "Fermix has nowhere to store this key",
        ),
        (Key::SkewNewerInstalledTitle, "A newer Fermix is installed"),
        (
            Key::SkewRestartToFinish,
            "Restart Fermix to finish updating",
        ),
        (
            Key::SystemScopeRefusalTitle,
            "Fermix is installed as a system service",
        ),
    ];

    for (key, expected) in fixed {
        assert_eq!(
            copy::entry(key).source,
            expected,
            "{key:?} does not carry the words the design fixes"
        );
    }
}

#[test]
fn the_microphone_statement_says_all_four_things_it_has_to_say() {
    let statement = copy::text(Key::VoiceMicrophoneStatement);

    assert!(statement.contains("Linux has no microphone permission"));
    assert!(statement.contains("nothing to revoke"));
    assert!(statement.contains("any other program you run"));
    assert!(statement.contains("PipeWire"));
    assert!(statement.contains("On macOS the operating system asks first"));
}

#[test]
fn the_voice_and_meetings_statements_are_the_redlines_own_sentences() {
    assert_eq!(
        copy::text(Key::VoiceCompanionStatement),
        "Voice is configured here and used from a companion application. The companion exists \
         for macOS today, and there is no Linux companion yet."
    );
    assert_eq!(
        copy::text(Key::MeetingsSleepStatement),
        "Fermix cannot keep this computer awake during a meeting. If the machine suspends, the \
         recording stops. Adjust your power settings before a long meeting."
    );
}

#[test]
fn the_three_computer_pane_statements_are_the_designs_own_sentences() {
    assert!(copy::text(Key::ComputerArm64)
        .starts_with("Computer use is not available on this architecture."));
    assert!(copy::text(Key::ComputerNoGraphicalSession)
        .starts_with("Computer use needs a graphical session."));
    assert!(copy::text(Key::ComputerWaylandSession)
        .starts_with("Computer use is not available on this Wayland session."));
}

#[test]
fn the_one_ellipsis_is_the_unicode_one() {
    let with_ellipsis: Vec<Key> = CATALOGUE
        .iter()
        .filter(|entry| entry.source.contains('\u{2026}'))
        .map(|entry| entry.key)
        .collect();

    assert!(
        !with_ellipsis.is_empty(),
        "the product has labels that need further input"
    );
    for key in with_ellipsis {
        assert!(
            !copy::text(key).contains("..."),
            "{key:?} mixes the two spellings"
        );
    }
}
