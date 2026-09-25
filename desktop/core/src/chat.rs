//! One conversation as the Chat page draws it: the questions, the replies as
//! they stream, the tools the assistant ran, and whether a reply is coming.

use crate::acp::{StopReason, ToolStatus, Update};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    User(String),
    /// Markdown, growing while it streams.
    Assistant(String),
    Tool {
        id: String,
        title: String,
        status: ToolStatus,
    },
    /// A quiet line about the conversation itself ("Stopped").
    Notice(String),
    /// Why a reply did not come.
    Failure(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// Asked; nothing back yet.
    Waiting,
    Streaming,
    /// Stop was asked for; the reply has not ended yet.
    Stopping,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    entries: Vec<Entry>,
    phase: Phase,
}

impl Transcript {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn can_send(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// Records a question; false when it is blank or a reply is still coming.
    pub fn send(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || !self.can_send() {
            return false;
        }
        self.entries.push(Entry::User(text.to_owned()));
        self.phase = Phase::Waiting;
        true
    }

    pub fn apply(&mut self, update: &Update) {
        match update {
            Update::MessageChunk(text) if text.is_empty() => {}
            Update::MessageChunk(text) => self.append_reply(text),
            Update::ToolCall { id, title, status } => {
                self.entries.push(Entry::Tool {
                    id: id.clone(),
                    title: title.clone(),
                    status: *status,
                });
                self.mark_streaming();
            }
            Update::ToolUpdate { id, status } => self.set_tool_status(id, *status),
            Update::Other(_) => {}
        }
    }

    fn append_reply(&mut self, text: &str) {
        match self.entries.last_mut() {
            Some(Entry::Assistant(reply)) => reply.push_str(text),
            _ => self.entries.push(Entry::Assistant(text.to_owned())),
        }
        self.mark_streaming();
    }

    fn mark_streaming(&mut self) {
        if self.phase == Phase::Waiting {
            self.phase = Phase::Streaming;
        }
    }

    fn set_tool_status(&mut self, tool_id: &str, new: ToolStatus) {
        let tool = self.entries.iter_mut().rev().find_map(|entry| match entry {
            Entry::Tool { id, status, .. } if id == tool_id => Some(status),
            _ => None,
        });
        if let Some(status) = tool {
            *status = new;
        }
    }

    /// Asks the reply to stop; false when there is nothing to stop.
    pub fn stop(&mut self) -> bool {
        if !matches!(self.phase, Phase::Waiting | Phase::Streaming) {
            return false;
        }
        self.phase = Phase::Stopping;
        true
    }

    pub fn finish(&mut self, reason: &StopReason) {
        self.phase = Phase::Idle;
        match reason {
            StopReason::EndTurn => {}
            StopReason::Cancelled => self.entries.push(Entry::Notice("Stopped".into())),
            StopReason::Other(_) => self
                .entries
                .push(Entry::Notice("The reply ended early".into())),
        }
    }

    /// A quiet line between entries. Skipped on an empty conversation and when
    /// it would repeat the line just above it.
    pub fn note(&mut self, text: &str) {
        let repeat = matches!(self.entries.last(), Some(Entry::Notice(last)) if last == text);
        if self.entries.is_empty() || repeat {
            return;
        }
        self.entries.push(Entry::Notice(text.to_owned()));
    }

    pub fn fail(&mut self, sentence: &str) {
        self.phase = Phase::Idle;
        self.entries.push(Entry::Failure(sentence.to_owned()));
    }
}

/// A tool's wire name as words: `web_search` reads "Web search".
pub fn tool_label(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| c == '_' || c == '.' || c == '-' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    let joined = words.join(" ");
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Tool".into(),
    }
}

/// JSON-RPC's code for a request the agent judged malformed.
const INVALID_PARAMS: i64 = -32602;
/// The engine answers a provider credential failure with -32000 and a message
/// starting "Re-authenticate" (engine `Acp.Peer.failure_error/1`).
const AUTH_REQUIRED: i64 = -32000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyError {
    /// The provider rejected Fermix's credentials.
    SignInAgain,
    /// The engine refused the message itself, in its own words.
    Refused(String),
    /// The turn failed inside the engine; the reason is in its log.
    Failed,
}

pub fn reply_error(code: i64, message: &str) -> ReplyError {
    match code {
        AUTH_REQUIRED => ReplyError::SignInAgain,
        INVALID_PARAMS => ReplyError::Refused(message.to_owned()),
        _ => ReplyError::Failed,
    }
}

impl ReplyError {
    pub fn sentence(&self) -> String {
        match self {
            ReplyError::SignInAgain => {
                "Your provider refused Fermix's sign-in. Sign in again under Providers.".into()
            }
            ReplyError::Refused(message) => format!("Fermix refused the message: {message}"),
            ReplyError::Failed => "Fermix could not answer. The reason is in its log.".into(),
        }
    }
}
