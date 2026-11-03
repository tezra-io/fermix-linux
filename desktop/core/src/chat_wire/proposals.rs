//! A proposal's card as the Chat page draws it (M60 cues §5.3), and the outside text in it
//! (§5.5): every sentence the card shows, and how outside text is prepared, which is plain. The
//! words are the macOS app's (`Localizable.strings` `proposal.*` and `outside.*`).
//!
//! Fermix's lines (the summary, the task, the watcher, the expiry, the daemon's sentences) are
//! Fermix's. What a model or a stranger wrote (the call's arguments, the item that led to it,
//! what the tool answered) is outside text: the page draws it only as such, never as Markdown,
//! never as a link, and never in a tray line, toast, title or notification.

use crate::chat_wire::protocol::{Cue, Decision};
use crate::chat_wire::timeline::{HeldProposal, Standing};
use crate::phone::unix_seconds;

pub const TITLE: &str = "A task wants to act";
pub const DENY: &str = "Deny";
pub const CONFIRM: &str = "Confirm";
pub const RUNNING: &str = "Running…";
pub const EXPIRED: &str = "This proposal expired; nothing ran.";
pub const LAST_MINUTE: &str = "One minute left to decide on the task";
/// The header over the recorded call's arguments.
pub const CALL: &str = "Written from outside text";
/// The header over an item whose record names no source or no time.
pub const ITEM_UNSOURCED: &str = "The item it acted on";
/// The header over what the tool or the validator answered.
pub const RESULT: &str = "What the tool answered";
pub const SHOW_ALL: &str = "Show all";

/// Every fixed sentence above, for the copy test.
pub const STRINGS: [&str; 10] = [
    TITLE,
    DENY,
    CONFIRM,
    RUNNING,
    EXPIRED,
    LAST_MINUTE,
    CALL,
    ITEM_UNSOURCED,
    RESULT,
    SHOW_ALL,
];

/// How many lines of outside text show before Show all.
pub const OUTSIDE_LINES: usize = 12;
/// How many characters one shown line holds, for counting a long line as the lines it wraps to.
pub const WRAP_COLUMNS: usize = 80;

/// What stands under a card's outside text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Footer {
    /// Deny and Confirm, alike, both enabled.
    Ask,
    /// The answer is on its way: both buttons wait, and a confirm says it is running.
    Answered { running: bool },
    /// A card the daemon does not say waits: its own line for where it stands, and no button.
    Status(String),
    /// Past its expiry on this computer's clock, with no answer sent.
    Expired,
    /// The daemon's last word, and what the tool answered, outside text.
    Resolved {
        sentence: String,
        result: Option<String>,
    },
}

/// The item a proposal acted on: where it came from (Fermix's record) and what it said
/// (outside text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub source: Option<String>,
    /// RFC 3339, as the daemon wrote it.
    pub received_at: Option<String>,
    /// The item's own fields, each labelled by Fermix, the values as the item wrote them.
    pub facts: Vec<String>,
    pub text: String,
}

/// One card, as the page draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: String,
    /// Fermix's sentence, with no outside text in it.
    pub summary: String,
    /// The task's name, or its job where it has none.
    pub task: String,
    pub watcher: Option<String>,
    /// RFC 3339, as the daemon wrote it.
    pub expires_at: String,
    /// The recorded call's arguments, outside text.
    pub call: String,
    pub item: Option<Item>,
    pub footer: Footer,
}

/// A held proposal as its card.
pub fn card(held: &HeldProposal) -> Card {
    let proposal = &held.proposal;
    let task = &proposal.task;
    Card {
        id: proposal.proposal_id.clone(),
        summary: proposal.summary.clone(),
        task: task.name.clone().unwrap_or_else(|| task.job_id.clone()),
        watcher: proposal.watcher.clone(),
        expires_at: proposal.expires_at.clone(),
        // Held cards are calls, whose arguments always decode (`Proposal`'s `try_from`).
        call: proposal.outside.args_text.clone().unwrap_or_default(),
        item: proposal.outside.cue.as_ref().map(|cue| Item {
            source: proposal.source_ref.clone(),
            received_at: proposal.received_at.clone(),
            facts: item_facts(cue),
            text: cue.text.clone().unwrap_or_default(),
        }),
        footer: footer(held),
    }
}

fn footer(held: &HeldProposal) -> Footer {
    match &held.standing {
        Standing::Open if held.asks() => Footer::Ask,
        Standing::Open => Footer::Status(held.proposal.status_sentence.clone()),
        Standing::Answered(decision) => Footer::Answered {
            running: *decision == Decision::Confirm,
        },
        Standing::Expired => Footer::Expired,
        Standing::Resolved {
            sentence,
            result_text,
        } => Footer::Resolved {
            sentence: sentence.clone(),
            result: result_text.clone(),
        },
        Standing::Refused { sentence } => Footer::Resolved {
            sentence: sentence.clone(),
            result: None,
        },
    }
}

/// The item's fields that it has, labelled. A missing one is left out, never "none".
fn item_facts(cue: &Cue) -> Vec<String> {
    [
        cue.r#type.as_ref().map(|value| format!("Type: {value}")),
        cue.subject
            .as_ref()
            .map(|value| format!("Subject: {value}")),
        cue.occurred_at
            .as_ref()
            .map(|value| format!("Dated: {value}")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// "Waits until 14:32", the time as `clock` shows it, or as the daemon wrote it where `clock`
/// cannot read it.
pub fn waits_until(card: &Card, clock: impl Fn(&str) -> Option<String>) -> String {
    let at = clock(&card.expires_at).unwrap_or_else(|| card.expires_at.clone());
    format!("Waits until {at}")
}

/// Fermix's line under the summary: the task, the watcher where there is one, and when the card
/// stops waiting.
pub fn facts(card: &Card, clock: impl Fn(&str) -> Option<String>) -> String {
    let mut parts = vec![format!("Task: {}", card.task)];
    parts.extend(card.watcher.as_ref().map(|name| format!("Watcher: {name}")));
    parts.push(waits_until(card, clock));
    parts.join(" · ")
}

/// The item's header: the source Fermix received it from, and when, or, where its record names
/// neither, only that it is the item.
pub fn item_header(item: &Item, clock: impl Fn(&str) -> Option<String>) -> String {
    let (Some(source), Some(received_at)) = (&item.source, &item.received_at) else {
        return ITEM_UNSOURCED.to_owned();
    };
    let received = clock(received_at).unwrap_or_else(|| received_at.clone());
    format!("From {source}, received {received}")
}

/// What a screen reader hears as a card that asks appears.
pub fn arrival(card: &Card, clock: impl Fn(&str) -> Option<String>) -> String {
    format!("{TITLE}, {}", waits_until(card, clock))
}

/// Seconds from `now` until a minute before the card's expiry, when a screen reader hears
/// `LAST_MINUTE`. `None` when that minute has begun or the expiry cannot be read.
pub fn last_minute_in(card: &Card, now: i64) -> Option<u64> {
    let expires = unix_seconds(&card.expires_at)?;
    u64::try_from(expires - 60 - now)
        .ok()
        .filter(|wait| *wait > 0)
}

/// The item's labelled fields on one line, or none.
pub fn joined_facts(item: &Item) -> Option<String> {
    (!item.facts.is_empty()).then(|| item.facts.join(" · "))
}

/// Outside text as it is shown: the characters exactly as written, cut after
/// `OUTSIDE_LINES` lines, a line longer than `WRAP_COLUMNS` counting as the lines it wraps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// What shows before Show all: the text, or its first lines followed by "…".
    pub text: String,
    /// Whether the cap cut anything, so Show all is offered.
    pub cut: bool,
}

/// Prepares outside text: plain, never parsed as Markdown or markup and never made a link,
/// only cut at the line cap.
pub fn outside(text: &str) -> Shown {
    let mut lines = 0;
    for (start, line) in line_starts(text) {
        let room = OUTSIDE_LINES - lines;
        let wraps = line.chars().count().div_ceil(WRAP_COLUMNS).max(1);
        if wraps <= room {
            lines += wraps;
            continue;
        }
        let kept: String = line.chars().take(room * WRAP_COLUMNS).collect();
        let head = format!("{}{kept}", &text[..start]);
        return Shown {
            text: format!("{}…", head.trim_end()),
            cut: true,
        };
    }
    Shown {
        text: text.to_owned(),
        cut: false,
    }
}

/// Each line of `text` with the byte offset it starts at.
fn line_starts(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split('\n').scan(0, |start, line| {
        let at = *start;
        *start += line.len() + 1;
        Some((at, line))
    })
}
