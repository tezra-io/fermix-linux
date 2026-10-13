//! The setup assistant's decisions (M38 §5.5): where Starting lands, when a
//! stage may be left, what About you writes, and the finish gate. The screens
//! live in the app; every rule they follow is here.

use crate::model::SetupState;
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Welcome,
    Starting,
    Connect,
    AboutYou,
    Applying,
    Ready,
}

/// The three styles and the sentence each writes (`assistant.ex:32-36`).
pub const STYLES: [(&str, &str); 3] = [
    ("Concise", "Answer in as few words as the question allows."),
    (
        "Balanced",
        "Answer in a few sentences, and go longer when the question needs it.",
    ),
    ("Detailed", "Explain your reasoning and cover the edges."),
];
pub const DEFAULT_STYLE: usize = 1;
const DEFAULT_ZONE: &str = "UTC";
const DEFAULT_ASSISTANT: &str = "Fermix";
const LINUX_PACKAGE: &str = "linux_package";

fn gates(state: &SetupState, pane: &str) -> bool {
    state
        .readiness
        .failures
        .iter()
        .any(|f| f.gating && f.pane.as_deref() == Some(pane))
}

/// Where the assistant goes once Fermix answers: the first gap with a screen.
pub fn landing(state: &SetupState) -> Stage {
    if gates(state, "providers") {
        return Stage::Connect;
    }
    if gates(state, "personality") {
        return Stage::AboutYou;
    }
    if state.restart.required {
        return Stage::Applying;
    }
    Stage::Ready
}

/// Connect your AI has no skip: it is done when no provider gap gates.
pub fn connect_done(state: &SetupState) -> bool {
    !gates(state, "providers")
}

/// The one write About you makes, all four keys, with nothing typed still valid.
pub fn personalization_answer(
    name: &str,
    zone: &str,
    style: usize,
    assistant: &str,
    account_name: &str,
) -> Map<String, Value> {
    let (_, sentence) = STYLES
        .get(style)
        .unwrap_or_else(|| panic!("style {style} is not one of the three"));
    let or = |typed: &str, fallback: &str| {
        let typed = typed.trim();
        if typed.is_empty() {
            fallback.trim().to_owned()
        } else {
            typed.to_owned()
        }
    };
    let mut answer = Map::new();
    answer.insert("user_name".into(), json!(or(name, account_name)));
    answer.insert("timezone".into(), json!(or(zone, DEFAULT_ZONE)));
    answer.insert("communication_style".into(), json!(sentence));
    answer.insert("bot_name".into(), json!(or(assistant, DEFAULT_ASSISTANT)));
    answer
}

/// The one finish gate, given the engine's distribution identity from `hello`.
/// The protocol window is checked on every read before this runs. The web door
/// check (`/health/live`) needs network access this app does not ask for, so
/// the socket's answer stands in for it.
pub fn finish_gate(distribution: Option<&str>, state: &SetupState) -> Result<(), &'static str> {
    if distribution != Some(LINUX_PACKAGE) {
        return Err(
            "This Fermix did not come from the Linux package, so this app will not manage it.",
        );
    }
    if state.readiness.failures.iter().any(|f| f.gating) {
        return Err("Setup still needs something before Fermix can answer.");
    }
    if state.restart.required {
        return Err("Fermix needs a restart to finish.");
    }
    Ok(())
}
