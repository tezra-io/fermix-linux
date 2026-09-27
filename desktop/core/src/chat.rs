//! One conversation as the Chat page draws it: the questions, the replies as
//! they stream, the tools the assistant ran, and whether a reply is coming.

use crate::acp::{Image, StopReason, ToolKind, ToolStatus, Update};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    User(String),
    /// Markdown, growing while it streams.
    Assistant(String),
    /// The assistant's reasoning, growing while it streams; drawn collapsed.
    Thought(String),
    Image(Image),
    /// A file the assistant sent that cannot come through the chat, by name.
    Attachment(String),
    /// Tools that ran one after another, drawn as one group.
    Tools(Vec<ToolRun>),
    /// A quiet line about the conversation itself ("Stopped").
    Notice(String),
    /// Why a reply did not come.
    Failure(String),
}

/// One tool's run. Times are wall-clock seconds, as the transcript's are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    pub id: String,
    /// The engine's name for the tool, as it came: "web_search".
    pub name: String,
    pub kind: ToolKind,
    pub state: RunState,
    pub started: i64,
    pub ended: Option<i64>,
}

/// How a run ended, or that it has not. The wire says only the first three;
/// the last two are what this app knows when the reply ended around a run.
impl ToolRun {
    /// Follows the wire's status; only an ending changes the run.
    fn settle(&mut self, status: ToolStatus, at: i64) {
        let state = match status {
            ToolStatus::Running => return,
            ToolStatus::Completed => RunState::Done,
            ToolStatus::Failed => RunState::Failed,
        };
        self.state = state;
        self.ended = Some(at);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Running,
    Done,
    Failed,
    /// The person stopped the reply while it ran.
    Stopped,
    /// The reply ended, or failed, before the run said how it went.
    Unfinished,
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

/// Times are wall-clock seconds since the epoch, passed in so this stays pure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// One per entry: when a question was sent, or when the reply it ends ended.
    times: Vec<Option<i64>>,
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

    /// The time shown under entry `index`: set on each question, and on the
    /// last entry of each reply once the reply has ended.
    pub fn time(&self, index: usize) -> Option<i64> {
        self.times.get(index).copied().flatten()
    }

    /// Fermix is working on a reply and nothing on screen shows it: no text is
    /// streaming in, no thought is coming in (it carries the orb in its title),
    /// and the last entry is not a group of tools (it carries the live line).
    /// The page shows its own live line.
    pub fn thinking(&self) -> bool {
        if self.phase == Phase::Idle {
            return false;
        }
        !matches!(
            self.entries.last(),
            Some(Entry::Assistant(_)) | Some(Entry::Thought(_)) | Some(Entry::Tools(_))
        )
    }

    /// When the question the running reply answers was asked; `None` while idle.
    pub fn turn_started(&self) -> Option<i64> {
        if self.phase == Phase::Idle {
            return None;
        }
        let (index, _) = self.last_question()?;
        self.time(index)
    }

    fn push(&mut self, entry: Entry, time: Option<i64>) {
        self.entries.push(entry);
        self.times.push(time);
    }

    /// Records a question; false when it is blank or a reply is still coming.
    pub fn send(&mut self, text: &str, at: i64) -> bool {
        let text = text.trim();
        if text.is_empty() || !self.can_send() {
            return false;
        }
        self.push(Entry::User(text.to_owned()), Some(at));
        self.phase = Phase::Waiting;
        true
    }

    /// Adds what the agent sent at `at`. False when it was not shown: a kind of
    /// update this app does not know, or a tool no entry has; the caller logs those.
    pub fn apply(&mut self, update: &Update, at: i64) -> bool {
        match update {
            Update::MessageChunk(text) if text.is_empty() => return true,
            Update::ThoughtChunk(text) if text.is_empty() => return true,
            Update::MessageChunk(text) => self.append_reply(text),
            Update::ThoughtChunk(text) => self.append_thought(text),
            Update::Image(image) => self.push(Entry::Image(image.clone()), None),
            Update::Attachment(name) => self.push(Entry::Attachment(name.clone()), None),
            Update::ToolCall {
                id,
                title,
                kind,
                status,
            } => self
                .add_run(ToolRun {
                    id: id.clone(),
                    name: title.clone(),
                    kind: *kind,
                    state: RunState::Running,
                    started: at,
                    ended: None,
                })
                .settle(*status, at),
            Update::ToolUpdate { id, status } => return self.set_tool_status(id, *status, at),
            Update::Other(_) => return false,
        }
        self.mark_streaming();
        true
    }

    fn append_reply(&mut self, text: &str) {
        match self.entries.last_mut() {
            Some(Entry::Assistant(reply)) => reply.push_str(text),
            _ => self.push(Entry::Assistant(text.to_owned()), None),
        }
    }

    fn append_thought(&mut self, text: &str) {
        match self.entries.last_mut() {
            Some(Entry::Thought(thought)) => thought.push_str(text),
            _ => self.push(Entry::Thought(text.to_owned()), None),
        }
    }

    fn mark_streaming(&mut self) {
        if self.phase == Phase::Waiting {
            self.phase = Phase::Streaming;
        }
    }

    /// Joins the group the reply is in, or starts one after whatever came last.
    fn add_run(&mut self, run: ToolRun) -> &mut ToolRun {
        if !matches!(self.entries.last(), Some(Entry::Tools(_))) {
            self.push(Entry::Tools(Vec::new()), None);
        }
        let Some(Entry::Tools(runs)) = self.entries.last_mut() else {
            unreachable!("the last entry is a group of tools");
        };
        runs.push(run);
        runs.last_mut().expect("a run was just added")
    }

    fn runs_mut(&mut self) -> impl Iterator<Item = &mut ToolRun> {
        self.entries
            .iter_mut()
            .filter_map(|entry| match entry {
                Entry::Tools(runs) => Some(runs),
                _ => None,
            })
            .flatten()
    }

    /// False when no run has `tool_id`.
    fn set_tool_status(&mut self, tool_id: &str, status: ToolStatus, at: i64) -> bool {
        let Some(run) = self.runs_mut().filter(|run| run.id == tool_id).last() else {
            return false;
        };
        run.settle(status, at);
        true
    }

    /// A run still going when its reply ends will never say how it went.
    fn end_runs(&mut self, state: RunState, at: i64) {
        for run in self.runs_mut().filter(|run| run.state == RunState::Running) {
            run.state = state;
            run.ended = Some(at);
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

    pub fn finish(&mut self, reason: &StopReason, at: i64) {
        self.phase = Phase::Idle;
        let left = match reason {
            StopReason::Cancelled => RunState::Stopped,
            StopReason::EndTurn | StopReason::Other(_) => RunState::Unfinished,
        };
        self.end_runs(left, at);
        self.time_reply(at);
        match reason {
            StopReason::EndTurn => {}
            StopReason::Cancelled => self.push(Entry::Notice("Stopped".into()), None),
            StopReason::Other(reason) => self.push(Entry::Notice(cut_short(reason)), None),
        }
    }

    /// Times the reply's last entry when it is a bubble; a reply that brought
    /// nothing, or ended on a caption line (a thought, a tool), has none. Nor
    /// does a reply in the same minute as its question, whose time says it.
    fn time_reply(&mut self, at: i64) {
        let asked = self.last_question().and_then(|(index, _)| self.time(index));
        if asked.is_some_and(|asked| asked.div_euclid(60) == at.div_euclid(60)) {
            return;
        }
        if let (Some(last), Some(time)) = (self.entries.last(), self.times.last_mut()) {
            if matches!(
                last,
                Entry::Assistant(_) | Entry::Image(_) | Entry::Attachment(_)
            ) {
                *time = Some(at);
            }
        }
    }

    /// A quiet line between entries. Skipped on an empty conversation and when
    /// it would repeat the line just above it.
    pub fn note(&mut self, text: &str) {
        let repeat = matches!(self.entries.last(), Some(Entry::Notice(last)) if last == text);
        if self.entries.is_empty() || repeat {
            return;
        }
        self.push(Entry::Notice(text.to_owned()), None);
    }

    pub fn fail(&mut self, sentence: &str, at: i64) {
        self.phase = Phase::Idle;
        self.end_runs(RunState::Unfinished, at);
        self.push(Entry::Failure(sentence.to_owned()), Some(at));
    }

    /// True when the conversation ends in a failed reply that can be asked again.
    pub fn can_retry(&self) -> bool {
        self.phase == Phase::Idle
            && matches!(self.entries.last(), Some(Entry::Failure(_)))
            && self.last_question().is_some()
    }

    /// Asks the failed question again: what the failed reply left is cleared,
    /// the question stays, and its text comes back to send. `None` when
    /// `can_retry` is false.
    pub fn retry(&mut self) -> Option<String> {
        if !self.can_retry() {
            return None;
        }
        let (index, question) = self.last_question()?;
        let question = question.to_owned();
        self.entries.truncate(index + 1);
        self.times.truncate(index + 1);
        self.phase = Phase::Waiting;
        Some(question)
    }

    fn last_question(&self) -> Option<(usize, &str)> {
        self.entries
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, entry)| match entry {
                Entry::User(question) => Some((index, question.as_str())),
                _ => None,
            })
    }
}

/// Why a reply ended before its turn did, from ACP's stop reasons.
fn cut_short(reason: &str) -> String {
    match reason {
        "max_tokens" => "The reply ran out of room.",
        "refusal" => "Fermix declined to answer this.",
        _ => "The reply ended early.",
    }
    .into()
}

/// The width to draw a `width` by `height` picture at so it fits a `bound`
/// pixel square, never larger than itself and never zero wide.
pub fn picture_width(width: i32, height: i32, bound: i32) -> i32 {
    assert!(
        width > 0 && height > 0 && bound > 0,
        "a picture that has no size: {width}x{height} in {bound}"
    );
    let for_height = i64::from(bound) * i64::from(width) / i64::from(height);
    let fitted = i64::from(width)
        .min(i64::from(bound))
        .min(for_height)
        .max(1);
    i32::try_from(fitted).expect("no wider than the picture itself")
}

/// The name a saved picture starts with: its type's usual extension, and none
/// when the type is not a plain word, since the agent chose it.
pub fn picture_file_name(mime: &str) -> String {
    let subtype = mime.strip_prefix("image/").unwrap_or_default();
    let extension = match subtype {
        "jpeg" => "jpg",
        "svg+xml" => "svg",
        other => other,
    };
    let plain = !extension.is_empty() && extension.chars().all(|c| c.is_ascii_alphanumeric());
    if plain {
        format!("Fermix image.{extension}")
    } else {
        "Fermix image".into()
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
    /// The turn failed inside the engine; the reason is in the daemon's log.
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
            ReplyError::Failed => "Fermix could not answer. The reason is in Logs.".into(),
        }
    }
}
