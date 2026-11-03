//! Every sentence the Cues pane adds of its own (M60 cues §8), the Mac's words with this
//! computer's in place of the Mac's. Status lines, consent lines, form labels, notes and refusals
//! are the daemon's and are not here.

use crate::chat::LocalTime;
use crate::overview::duration_words;

pub const PANE_TITLE: &str = "Cues";
/// More words search finds the pane by.
pub const KEYWORDS: &str = "feeds streams webhooks watchers mandates sources proposals door";
pub const SUBTITLE: &str =
    "Outside items that can wake a task: feeds, streams, webhooks and your scripts.";
pub const LEARN_MORE: &str = "Learn more";
/// The product's own page, the same for every home: what the pane keeps short, explained.
pub const GUIDE_URL: &str = "https://fermix.ai/docs/cues/";
pub const GUIDE_FAILED: &str = "The browser could not open the cues guide.";

pub const WAITING_TITLE: &str = "Waiting for you";
pub const SHOW_IN_CHAT: &str = "Show in chat";
pub const SOURCES_TITLE: &str = "Sources";
pub const ADD_SOURCE: &str = "Add source";
pub const SOURCES_EMPTY: &str = "No sources yet.";
pub const WATCHERS_TITLE: &str = "Watchers";
pub const WATCHERS_EMPTY: &str = "No watchers yet.";
pub const WATCHERS_FOOTER: &str = "Watchers are made and changed in chat.";
pub const OPEN_CHAT: &str = "Open chat";
pub const MANDATES_TITLE: &str = "Mandates";
pub const MANDATES_EMPTY: &str = "No mandates. Without one, a task only reports to you.";
pub const ADD_MANDATE: &str = "Add mandate";
pub const READING: &str = "Reading from Fermix…";

pub const MODE_APPROVE: &str = "Ask me first";
pub const MODE_AUTO: &str = "Act without asking";
pub const APPROVE: &str = "Approve";
pub const CANCEL: &str = "Cancel";
pub const DONE: &str = "Done";
pub const EDIT: &str = "Edit";
pub const REMOVE: &str = "Remove";
pub const NONE: &str = "None";
pub const MORE_OPTIONS: &str = "More options";
pub const MORE_ABOUT: &str = "More about this";

// The source form and a source's page.
pub const ADD_SOURCE_TITLE: &str = "Add a source";
pub const KIND: &str = "Kind";
pub const PRESET: &str = "Preset";
pub const SECRET_UNSTORABLE: &str =
    "This version of the app cannot store this secret; update the app.";
pub const ONE_OF_PUBLISHED: &str = "This setting takes one of its published values.";
pub const URL: &str = "URL";
pub const COPY_URL: &str = "Copy URL";
pub const ADDRESS_MADE: &str = "Made";
pub const ADDRESS_NOT_MADE: &str = "Not made yet";
pub const ADDRESS_INFO: &str = "Give the service your tunnel's address followed by this path.";
pub const NEW_TOKEN: &str = "New token…";
pub const MAKE_TOKEN: &str = "Make token";
pub const COPY_TOKEN: &str = "Copy token";
pub const TOKEN_ONCE: &str = "Copy it now. Fermix shows it only once.";
pub const NOT_COPIED: &str = "Nothing was copied.";
pub const STORED: &str = "Stored";
pub const NOT_STORED: &str = "Not stored";
pub const ITEMS_A_DAY: &str = "Items a day";
pub const NO_LIMIT: &str = "No limit";
pub const LAST_DAY: &str = "Last day";
pub const REFUSED: &str = "Refused";
pub const MISSED: &str = "Missed";
pub const LISTENING: &str = "Listening";
pub const REMOVE_SOURCE: &str = "Remove source…";
pub const SOURCE_GONE: &str = "This source is no longer in the list.";

// A watcher's page.
pub const PURPOSE: &str = "Purpose";
pub const LISTENS_FOR: &str = "Listens for";
pub const INSTRUCTIONS: &str = "Instructions";
pub const FILTER: &str = "Filter";
pub const FILTER_MAIN_MODEL: &str = "Your main model";
pub const FILTER_NONE: &str = "No filter";
pub const SOURCES: &str = "Sources";
pub const TASK: &str = "Task";
pub const KEEPS_NOTES: &str = "keeps notes";
pub const KEEPS_NO_NOTES: &str = "keeps no notes";
pub const PAUSED: &str = "paused";
pub const DAILY_LIMIT: &str = "Daily limit";
pub const EXAMPLES: &str = "Examples";
pub const WATCHER_GONE: &str = "This watcher is no longer in the list.";

// A mandate's page and its editor.
pub const MODE: &str = "Mode";
pub const TOOLS: &str = "Tools";
pub const READ_TOOLS: &str = "Read tools";
pub const WAITS: &str = "Waits";
pub const ACTS_ON: &str = "Acts on";
pub const ACTS_ON_ANY: &str = "items of any age";
pub const VALIDATOR: &str = "Validator";
pub const APPROVAL: &str = "Approval";
pub const REMOVE_MANDATE: &str = "Remove mandate…";
pub const ADD_MANDATE_TITLE: &str = "Add a mandate";
pub const NAME: &str = "Name";
pub const NAME_INFO: &str = "Lowercase letters, digits and hyphens.";
pub const CHOOSE_TASK: &str = "Choose a task";
pub const NO_TOOLS: &str = "No installed plugin offers a tool a mandate can name.";
pub const READ_ONLY: &str = "Read only";
pub const CHANGES_THINGS: &str = "Can change things";
pub const MINUTES: &str = "minutes";
pub const WAITS_INFO: &str = "How long a proposal waits for your answer before it expires.";
pub const ACTS_ON_INFO: &str = "The oldest an item can be for a call to act on it. 0 is no limit.";
pub const INTERPRETER: &str = "Interpreter";
pub const ENVIRONMENT_NAMES: &str = "Environment names";
pub const VALIDATOR_INFO: &str =
    "A validator must be owned by root. After saving, the mandate's line says whether it holds.";
pub const CALL_ID_ARGUMENT: &str = "Call ID argument";
pub const CALL_ID_INFO: &str = "The argument Fermix sets to each call's own ID.";
pub const SKILL_PATHS: &str = "Skill paths it writes";
pub const PINNED_ARGUMENTS: &str = "Pinned arguments";

/// Every fixed sentence the Cues pane shows, so a test can hold them to the copy rules.
pub const STRINGS: [&str; 92] = [
    PANE_TITLE,
    SUBTITLE,
    LEARN_MORE,
    GUIDE_FAILED,
    WAITING_TITLE,
    SHOW_IN_CHAT,
    SOURCES_TITLE,
    ADD_SOURCE,
    SOURCES_EMPTY,
    WATCHERS_TITLE,
    WATCHERS_EMPTY,
    WATCHERS_FOOTER,
    OPEN_CHAT,
    MANDATES_TITLE,
    MANDATES_EMPTY,
    ADD_MANDATE,
    READING,
    MODE_APPROVE,
    MODE_AUTO,
    APPROVE,
    CANCEL,
    DONE,
    EDIT,
    REMOVE,
    NONE,
    MORE_OPTIONS,
    MORE_ABOUT,
    ADD_SOURCE_TITLE,
    KIND,
    PRESET,
    SECRET_UNSTORABLE,
    ONE_OF_PUBLISHED,
    URL,
    COPY_URL,
    ADDRESS_MADE,
    ADDRESS_NOT_MADE,
    ADDRESS_INFO,
    NEW_TOKEN,
    MAKE_TOKEN,
    COPY_TOKEN,
    TOKEN_ONCE,
    NOT_COPIED,
    STORED,
    NOT_STORED,
    ITEMS_A_DAY,
    NO_LIMIT,
    LAST_DAY,
    REFUSED,
    MISSED,
    LISTENING,
    REMOVE_SOURCE,
    SOURCE_GONE,
    PURPOSE,
    LISTENS_FOR,
    INSTRUCTIONS,
    FILTER,
    FILTER_MAIN_MODEL,
    FILTER_NONE,
    SOURCES,
    TASK,
    KEEPS_NOTES,
    KEEPS_NO_NOTES,
    PAUSED,
    DAILY_LIMIT,
    EXAMPLES,
    WATCHER_GONE,
    MODE,
    TOOLS,
    READ_TOOLS,
    WAITS,
    ACTS_ON,
    ACTS_ON_ANY,
    VALIDATOR,
    APPROVAL,
    REMOVE_MANDATE,
    ADD_MANDATE_TITLE,
    NAME,
    NAME_INFO,
    CHOOSE_TASK,
    NO_TOOLS,
    READ_ONLY,
    CHANGES_THINGS,
    MINUTES,
    WAITS_INFO,
    ACTS_ON_INFO,
    INTERPRETER,
    ENVIRONMENT_NAMES,
    VALIDATOR_INFO,
    CALL_ID_ARGUMENT,
    CALL_ID_INFO,
    SKILL_PATHS,
    PINNED_ARGUMENTS,
];

/// Two parts of one line, as the Mac joins them.
pub fn middot(parts: &[String]) -> String {
    parts.join(" · ")
}

pub fn waiting_row(mandate: &str, tool: &str) -> String {
    format!("{mandate} wants to run {tool}")
}

pub fn waits_until(time: &str) -> String {
    format!("Waits until {time}")
}

pub fn approve_question(name: &str) -> String {
    format!(
        "Approve {name} as it is now? A later change to its task, skill or validator turns it \
         off until you approve it again."
    )
}

pub fn remove_mandate_question(name: &str) -> String {
    format!("Remove {name}? Its task can no longer act under it.")
}

/// The question Remove source… asks, naming the watchers that listen where any do.
pub fn remove_source_question(name: &str, listening: &[String]) -> String {
    let asked = format!("Remove {name}? Its stored secrets are deleted.");
    if listening.is_empty() {
        return asked;
    }
    format!(
        "{asked} These watchers listen to it: {}.",
        listening.join(", ")
    )
}

pub fn new_token_question(name: &str) -> String {
    format!("Make a new token for {name}? The old one stops working within a minute.")
}

/// What a screen reader hears for a row's switch, which shows no words of its own.
pub fn switch_label(name: &str) -> String {
    format!("Turn {name} on or off")
}

/// What a screen reader hears for a token shown once: never the token.
pub fn token_label(name: &str) -> String {
    format!("Token for {name}, shown once")
}

/// The push example under a token: the door's port, the source, and the token left as a
/// placeholder.
pub fn push_example(port: u16, name: &str) -> String {
    format!(
        "curl -sS http://127.0.0.1:{port}/cues/v1/{name} -H 'Authorization: Bearer <token>' \
         --data @item.json"
    )
}

/// A source's last day: what it received, and how much reached a watcher.
pub fn source_last_day(received: u64, routed: u64) -> String {
    middot(&[
        format!("{received} received"),
        format!("{routed} reached a watcher"),
    ])
}

/// Items a day, where 0 is no limit.
pub fn items_a_day(budget: u64) -> String {
    if budget == 0 {
        return NO_LIMIT.to_owned();
    }
    budget.to_string()
}

/// Names one a line, or None.
pub fn lines(sentences: &[String]) -> String {
    if sentences.is_empty() {
        return NONE.to_owned();
    }
    sentences.join("\n")
}

/// Names in a row, or None.
pub fn list(names: &[String]) -> String {
    if names.is_empty() {
        return NONE.to_owned();
    }
    names.join(", ")
}

pub fn instruction(number: u64, text: &str) -> String {
    format!("{number}. {text}")
}

pub fn added(day: &str) -> String {
    format!("added {day}")
}

pub fn daily_limit(cap: u64) -> String {
    if cap == 0 {
        return NONE.to_owned();
    }
    format!("{cap} items")
}

pub fn watcher_last_day(c: &super::wire::WatcherCounts) -> String {
    middot(&[
        format!("{} offered", c.offered),
        format!("{} waiting", c.waiting),
        format!("{} passed", c.passed),
        format!("{} dropped", c.dropped),
        format!("{} acted", c.acted),
        format!("{} failed", c.failed),
    ])
}

pub fn examples(pass: u64, drop: u64) -> String {
    middot(&[
        format!("{pass} marked useful"),
        format!("{drop} marked not useful"),
    ])
}

pub fn mandate_last_day(c: &super::wire::MandateCounts) -> String {
    middot(&[
        format!("{} allowed", c.allowed),
        format!("{} proposed", c.proposed),
        format!("{} refused", c.refused),
    ])
}

/// How long a proposal waits: "30 minutes, then the proposal expires".
pub fn waits(seconds: u64) -> String {
    format!(
        "{}, then the proposal expires",
        duration_words(seconds * 1000)
    )
}

/// How old an item a mandate acts on may be, where 0 is any age.
pub fn acts_on(seconds: u64) -> String {
    if seconds == 0 {
        return ACTS_ON_ANY.to_owned();
    }
    format!("items at most {} old", duration_words(seconds * 1000))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The day a watcher's instruction was added, as the Mac draws it: "Oct 12", with the year only
/// beyond the reader's. Both are on the reader's clock (a time with its hour goes through
/// `chat::written_time`).
pub fn day(at: LocalTime, now: LocalTime) -> String {
    let month = usize::try_from(at.month - 1)
        .ok()
        .and_then(|index| MONTHS.get(index))
        .unwrap_or_else(|| panic!("a month is 1 to 12, not {}", at.month));
    if at.year == now.year {
        return format!("{month} {}", at.day);
    }
    format!("{month} {}, {}", at.day, at.year)
}
