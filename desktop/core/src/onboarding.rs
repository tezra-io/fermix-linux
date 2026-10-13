//! The setup assistant's decisions (M38 §5.5): what a launch opens on, what
//! Starting says and when it stops before it starts, where Starting lands,
//! when a stage may be left, what About you writes, and the finish gate. The
//! screens live in the app; every rule they follow is here.

use crate::model::SetupState;
use crate::service::Binding;
use crate::view::DaemonProblem;
use serde_json::{json, Map, Value};
use std::time::Duration;

/// How the window was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// Someone opened it.
    User,
    /// The login entry's `--background`: the window waits in the tray.
    Login,
}

/// What the window opens on once it has read the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// The setup assistant, at Welcome.
    Welcome,
    /// The setup assistant stopped on Starting with why the binding cannot be
    /// used. Linux has no Recovery screen; Starting's failure is where its
    /// sentence and Try again already live.
    Recovery(String),
    Home,
    Chat,
}

/// Starting's words.
pub const STARTING_TITLE: &str = "Starting";
pub const STARTING_NOTE: &str = "Fermix runs as a background service on this computer, so it \
                                 keeps answering when this window is closed. The first start can \
                                 take a minute or two.";
pub const STARTING_SERVICE: &str = "Starting the background service";
pub const STARTING_ANSWER: &str = "Checking it answers";
pub const STOPPED_ANSWERING: &str = "Fermix stopped answering.";

/// The launch rule, after macOS's `AppCoordinator.presentation`. A login launch
/// opens nothing over the window. Otherwise a binding the CLI would refuse
/// stops on its reason before anything else, so no home is ever guessed; a
/// daemon that answers decides by its readiness; and with none answering, a
/// fresh account opens the assistant and a bound one lands on Home.
pub fn opening(launch: Launch, binding: &Binding, daemon: Option<&SetupState>) -> Opening {
    let ready = daemon.map(|state| state.readiness.status == "ready");
    if launch == Launch::Login {
        return if ready == Some(true) {
            Opening::Chat
        } else {
            Opening::Home
        };
    }
    if let Binding::Unreadable(sentence) = binding {
        return Opening::Recovery(sentence.clone());
    }
    match (ready, binding) {
        (Some(true), _) => Opening::Chat,
        (Some(false), _) | (None, Binding::Absent) => Opening::Welcome,
        (None, _) => Opening::Home,
    }
}

/// Why Starting stops before it starts anything: a daemon that answered but
/// cannot serve this app is already running, and starting it again changes
/// nothing. `None` when nothing answered, so Starting starts Fermix.
pub fn already_answered(problem: &DaemonProblem) -> Option<String> {
    match problem {
        DaemonProblem::NotRunning | DaemonProblem::NotResponding => None,
        DaemonProblem::UpdateNeeded(_) | DaemonProblem::Broken(_) => Some(problem.sentence()),
    }
}

/// The service was started, and the window's reads went unanswered for `waited`.
pub fn starting_silent(waited: Duration) -> String {
    let seconds = waited.as_secs();
    format!(
        "The background service was started but did not answer within {seconds} seconds. Read \
         what it said with journalctl --user -u fermix, then try again."
    )
}

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
