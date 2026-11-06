//! A chat decision is made only by its card's own buttons (M60 cues §6 rule 3, §12). A registered
//! GtkApplication exports every `win.*` and `app.*` action on the session bus (`org.gtk.Actions`
//! at `/io/tezra/Fermix/window/<n>`), so an action that decided could be activated by any process
//! of the session, one Fermix started included, and the app's own connection, placed as the
//! owner, would then send the answer. These read the app's sources, which hold every action it
//! registers.

use std::path::{Path, PathBuf};

const APP_SOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../app/src");
/// How deep the app's source tree goes; a deeper one fails the walk rather than being skipped.
const MAX_DEPTH: usize = 4;
/// The calls that register an action, each followed by its name: the app's registering helpers
/// (`on`, `on_plain` and `on_value` in app.rs, `app_action` in voice_flow.rs) and GIO's own.
const REGISTRATIONS: [&str; 6] = [
    "on(",
    "on_plain(",
    "on_value(",
    "app_action(",
    "SimpleAction::new(",
    "SimpleAction::new_stateful(",
];
/// Words that make an action a decision.
const DECIDING: [&str; 5] = ["approve", "deny", "confirm", "decide", "answer"];
/// The cards whose buttons decide.
const CARDS: [&str; 2] = ["chat/approvals.rs", "chat/proposals.rs"];

/// Every `.rs` file under `dir`, recursively, at most `depth` levels down.
fn sources(dir: &Path, depth: usize) -> Vec<PathBuf> {
    assert!(
        depth <= MAX_DEPTH,
        "the app's sources are deeper than {MAX_DEPTH} levels"
    );
    let mut found = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            found.extend(sources(&path, depth + 1));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    found
}

/// One registration as the scan reads it.
#[derive(Debug, PartialEq, Eq)]
enum Registration {
    /// The action's name, a string literal.
    Named(String),
    /// A registering helper handing on the name its callers give, which the scan reads there.
    Forwarded,
    /// A name the scan cannot read, by where it stands: the test fails on it.
    Unread(String),
}

/// `rest` past a first argument `app`, the way the app's helpers are called.
fn after_app(rest: &str) -> &str {
    let rest = rest.trim_start();
    rest.strip_prefix("app,").map_or(rest, str::trim_start)
}

/// The length of the identifier `text` starts with; zero when it starts with none.
fn identifier_len(text: &str) -> usize {
    text.find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len())
}

/// The name a registration at the start of `rest` gives, when it is a string literal: the first
/// argument, or the second after `app`.
fn literal_name(rest: &str) -> Option<String> {
    let quoted = after_app(rest).strip_prefix('"')?;
    let (name, _) = quoted.split_once('"')?;
    Some(name.to_owned())
}

/// Whether the name a registration at the start of `rest` gives is a bare identifier.
fn identifier_name(rest: &str) -> bool {
    let rest = after_app(rest);
    let end = identifier_len(rest);
    end > 0 && rest[end..].trim_start().starts_with([',', ')'])
}

/// The function or closure the text before `at` ends inside, by name: the nearest `fn <name>` or
/// `let <name> = |` before it.
fn enclosing(source: &str, at: usize) -> Option<&str> {
    let before = &source[..at];
    let function = before.rfind("fn ").map(|start| start + 3);
    let closure = before
        .match_indices("let ")
        .map(|(start, _)| start + 4)
        .filter(|&start| {
            let rest = &before[start..];
            let value = rest[identifier_len(rest)..].trim_start().strip_prefix('=');
            value.is_some_and(|value| value.trim_start().starts_with('|'))
        })
        .last();
    let start = function.max(closure)?;
    let name = &before[start..];
    Some(&name[..identifier_len(name)])
}

/// Whether the match at `at` is a call: a whole word, not a definition (`fn on(`), and not in a
/// string (`"on(`).
fn is_call(source: &str, at: usize) -> bool {
    let before = &source[..at];
    let word_start = before
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '"'));
    word_start && !before.ends_with("fn ")
}

/// Every registration `source` makes; `file` names it in what the test prints.
fn registered(source: &str, file: &str) -> Vec<Registration> {
    let mut found = Vec::new();
    for call in REGISTRATIONS {
        for (at, _) in source
            .match_indices(call)
            .filter(|(at, _)| is_call(source, *at))
        {
            let rest = &source[at + call.len()..];
            let helper = enclosing(source, at).map(|name| format!("{name}("));
            let forwards = helper.is_some_and(|helper| REGISTRATIONS.contains(&helper.as_str()));
            found.push(match literal_name(rest) {
                Some(name) => Registration::Named(name),
                None if forwards && identifier_name(rest) => Registration::Forwarded,
                None => {
                    let line = source[..at].matches('\n').count() + 1;
                    Registration::Unread(format!("{file}:{line}"))
                }
            });
        }
    }
    found
}

fn every_registration() -> Vec<Registration> {
    let files = sources(Path::new(APP_SOURCES), 0);
    assert!(!files.is_empty(), "the app's sources are at {APP_SOURCES}");
    files
        .iter()
        .flat_map(|file| {
            let source = std::fs::read_to_string(file).expect("a source reads");
            registered(&source, &file.display().to_string())
        })
        .collect()
}

fn every_action() -> Vec<String> {
    every_registration()
        .into_iter()
        .filter_map(|registration| match registration {
            Registration::Named(name) => Some(name),
            _ => None,
        })
        .collect()
}

/// A decision of the chat's or of cues: a deciding word in an action of the chat, a proposal
/// or a cue.
fn chat_decision(name: &str) -> bool {
    let chats = name.starts_with("chat-") || name.contains("proposal") || name.contains("cue");
    chats && DECIDING.iter().any(|word| name.contains(word))
}

#[test]
fn the_scan_finds_the_apps_own_registrations() {
    let actions = every_action();
    for known in [
        "send-message",
        "chat-older",
        "chat-connect",
        "about",
        "quit",
        "voice-call",
    ] {
        assert!(
            actions.iter().any(|name| name == known),
            "{known} is not among {actions:?}"
        );
    }
    let multi_line = "on_value(\n        app,\n        \"setting-apply\",";
    let named = Registration::Named("setting-apply".into());
    assert_eq!(registered(multi_line, "x"), [named]);
    assert!(registered("connection(app, \"x\")", "x").is_empty());
    assert!(registered("let text = \"on(app, NAME)\";", "x").is_empty());
    let helper = "fn on(app: &App, name: &str) {\n    gio::SimpleAction::new(name, None);\n}";
    assert_eq!(registered(helper, "x"), [Registration::Forwarded]);
    let closure =
        "fn group() {\n    let on = |name: &str| {\n        SimpleAction::new(name, None)";
    assert_eq!(registered(closure, "x"), [Registration::Forwarded]);
    let other = "fn helper(name: &str) {\n    gio::SimpleAction::new(name, None);\n}";
    assert_eq!(registered(other, "x"), [Registration::Unread("x:2".into())]);
    let computed = "on(app, NAMES[0], handler);";
    assert_eq!(
        registered(computed, "x"),
        [Registration::Unread("x:1".into())]
    );
}

/// A registration whose name the scan cannot read could be a decision it never sees, so it fails
/// here rather than being skipped: the name is a string literal, or a registering helper's own.
#[test]
fn every_registration_names_its_action_where_the_scan_reads_it() {
    let unread: Vec<String> = every_registration()
        .into_iter()
        .filter_map(|registration| match registration {
            Registration::Unread(at) => Some(at),
            _ => None,
        })
        .collect();
    assert!(
        unread.is_empty(),
        "these registrations name their action in a way the scan cannot read: {unread:?}"
    );
}

#[test]
fn no_window_or_application_action_decides_a_chat_card() {
    let deciding: Vec<String> = every_action()
        .into_iter()
        .filter(|name| chat_decision(name))
        .collect();
    assert!(
        deciding.is_empty(),
        "these actions decide a chat card, and the session bus exports them: {deciding:?}"
    );
}

#[test]
fn a_cards_buttons_decide_by_their_own_closures() {
    for card in CARDS {
        let path = Path::new(APP_SOURCES).join(card);
        let source = std::fs::read_to_string(&path).expect("the card's source reads");
        assert!(
            !source.contains("action_name") && !source.contains("activate_action"),
            "{card} reaches an action"
        );
        assert!(
            source.contains("decide_on_click("),
            "{card} binds no button's click to its decision"
        );
    }
    let page = std::fs::read_to_string(Path::new(APP_SOURCES).join("chat.rs"))
        .expect("the chat page's source reads");
    let (_, binding) = page
        .split_once("fn decide_on_click(")
        .expect("the chat page binds a card's buttons");
    let body = binding.split("\n}\n").next().expect("the binding's body");
    assert!(
        body.contains("connect_clicked") && !body.contains("action"),
        "a card's button decides by its own click: {body}"
    );
}
