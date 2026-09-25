//! The voice call as a pure reducer: `Session::apply(Input) -> Vec<Effect>`. Ported from the
//! macOS `AppModel` voice routing and `VoicePresentation`, with the Linux rules of spec §1.4 and
//! §1.6. It holds no socket, no pipeline and no timer: the controller performs the effects, in
//! order, and feeds back what happened as inputs.
//!
//! Inputs:
//! - from the user: `Begin`, `End`, `Mute(bool)`, `Stop { played_ms }` (read `played_ms` from
//!   the playback queue *before* applying: the flush resets its anchor), `CancelTask`;
//! - from the socket: `Connected` or `ConnectFailed` (the answer to `Effect::Connect`), then
//!   every `Incoming` from the `Inbox` as `Wire`;
//! - from the audio pipeline: `Drained` (playback ran dry), `AudioFailed(sentence)`;
//! - from the timer: `CallDeadline(n)`, `CALL_START_DEADLINE` after `Effect::WatchCallStart(n)`.
//!
//! Effects:
//! - `Connect`: `client::connect` off the main thread, answered by `Connected`/`ConnectFailed`;
//! - `Send(event)`: `Outbox::control`;
//! - `StartAudio` / `StopAudio`: build and play the call pipeline, or set it to `Null`;
//! - `Arm(bool)` / `MuteMic(bool)`: the two halves of the microphone gate;
//! - `FlushPlayback` / `ResetAnchor`: `PlaybackQueue::clear` / `reset_anchor`;
//! - `WatchCallStart(n)`: apply `CallDeadline(n)` after `CALL_START_DEADLINE`;
//! - `Disconnect`: drop the `Outbox` and `Inbox`; the connection is already over.
//!
//! Rules that are easy to break:
//! - An effect that answers at once (a failed `StartAudio`) ends its batch; its answer is
//!   applied next. What follows it belongs to a call that did not start: no `call_start`,
//!   which would open a billed provider session for a dead microphone.
//! - Nothing streams before the daemon first says `listening` (the gate starts shut).
//! - Ending a call shuts the gate and stops the pipeline *before* `call_stop` goes out: audio
//!   after `call_stop` is `not_connected` to the daemon, which then hangs up.
//! - The daemon's `muted` and `idle` are authoritative about mute.
//! - The speaking look outlasts the daemon's `speaking` until playback drains.
//! - Any error frame or close tears the audio down; `End` keeps the connection.

use crate::realtime::client::{CloseReason, ConnectError, Incoming};
use crate::realtime::playback::{rms, smooth};
use crate::realtime::protocol::{
    Caption, ClientEvent, Direction, ServerError, ServerEvent, Speaker, Task, TaskStatus,
    ToolStatus, TurnState, Usage,
};
use std::time::Duration;

/// From `call_start` to the first `listening`. The daemon's own limit is 10 s.
pub const CALL_START_DEADLINE: Duration = Duration::from_secs(12);
/// The engine whose calls have tasks that can be cancelled.
const LIVE_ENGINE: &str = "openai_live";

const NOT_OPENED: &str = "Voice is on, but Fermix has not opened its voice connection. \
                          Restarting Fermix usually fixes this.";
const NOT_STARTED: &str = "Fermix did not start the call in time.";
const DISAGREED: &str = "Voice stopped: this app and Fermix could not understand each other.";
const TOOL_FAILED: &str = "The tool did not finish.";
const NOT_ANSWERED: &str = "Fermix did not answer the voice connection.";
const CONNECTION_FAILED: &str = "The voice connection to Fermix failed.";
const CONNECTION_CLOSED: &str = "The voice connection to Fermix closed.";
const NOT_READING: &str = "Fermix stopped taking voice audio.";
/// Reasons that mean this app broke the wire (spec §1.4); tuples arrive as Elixir `inspect`.
const VIOLATIONS: [&str; 14] = [
    "handshake_required",
    "unexpected_client_hello",
    "invalid_json",
    "invalid_event",
    "missing_type",
    "missing_audio",
    "invalid_audio_base64",
    "invalid_audio_end_ms",
    "missing_delegation_id",
    "missing_protocol_version",
    "invalid_protocol_version",
    "line_too_large",
    "not_connected",
    "unsupported_by_engine",
];
const VIOLATION_TUPLES: [&str; 2] = ["{:unknown_event", "{:chunk_too_large"];

/// What voice is doing, as one word (spec §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Offline,
    /// Connecting the socket, or waiting for the daemon's first `listening` after `call_start`.
    Connecting,
    Idle,
    Listening,
    Muted,
    Thinking,
    Speaking,
    ToolUse,
    /// The daemon lost OpenAI mid-call and is retrying.
    Reconnecting,
    /// See `Session::error`.
    Error,
}

/// The palette role a mode draws in. Never the only signal: every mode has a word and an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    Faint,
    Secondary,
    /// A live microphone.
    Accent,
    Warning,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The status word, or in the error mode, the error's sentence.
    pub label: String,
    /// A symbolic icon name from the GNOME 50 Adwaita theme.
    pub icon: &'static str,
    pub palette: Palette,
}

#[derive(Debug)]
pub enum Input {
    Begin,
    End,
    Mute(bool),
    /// `played_ms` is `playback::played_ms` for the current utterance, read before the flush.
    Stop {
        played_ms: u64,
    },
    CancelTask,
    Connected,
    ConnectFailed(ConnectError),
    Wire(Incoming),
    Drained,
    /// The pipeline failed, in the audio module's own sentence.
    AudioFailed(String),
    CallDeadline(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Connect,
    Send(ClientEvent),
    StartAudio,
    StopAudio,
    Arm(bool),
    MuteMic(bool),
    FlushPlayback,
    ResetAnchor,
    WatchCallStart(u64),
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Link {
    #[default]
    Offline,
    Connecting,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    link: Link,
    mode: Mode,
    in_call: bool,
    muted: bool,
    /// Streaming armed: the daemon said `listening` during this call.
    armed: bool,
    /// Reply audio is still playing out.
    tail: bool,
    level: f32,
    error: Option<String>,
    engine: Option<String>,
    call_id: Option<String>,
    caption: Option<Caption>,
    task: Option<Task>,
    usage: Option<Usage>,
    /// Numbers calls, so a deadline left over from an earlier one is ignored.
    call_number: u64,
    awaiting_listen: bool,
    begin_when_connected: bool,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    pub fn apply(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Begin => self.begin(),
            Input::End => self.end(),
            Input::Mute(on) => self.mute(on),
            Input::Stop { played_ms } => self.stop(played_ms),
            Input::CancelTask => self.cancel_task(),
            Input::Connected => self.connected(),
            Input::ConnectFailed(error) => self.connect_failed(&error),
            Input::Wire(Incoming::Event(event)) => self.event(event),
            Input::Wire(Incoming::Audio { rms, .. }) => self.audio(rms),
            Input::Wire(Incoming::Closed(reason)) => self.closed(&reason),
            Input::Drained => self.drained(),
            Input::AudioFailed(sentence) => self.fail_call(sentence),
            Input::CallDeadline(call) => self.call_deadline(call),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The mode the pet shows: speaking while reply audio still plays, whatever the daemon says.
    pub fn visual_mode(&self) -> Mode {
        if self.in_call && self.tail {
            Mode::Speaking
        } else {
            self.mode
        }
    }

    pub fn in_call(&self) -> bool {
        self.in_call
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn speaking_tail(&self) -> bool {
        self.tail
    }

    /// The reply's smoothed output level, 0.0 to 1.0, for the pet's pulse.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Why voice stopped, while the mode is `Error`.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref().filter(|_| self.mode == Mode::Error)
    }

    pub fn engine(&self) -> Option<&str> {
        self.engine.as_deref()
    }

    pub fn call_id(&self) -> Option<&str> {
        self.call_id.as_deref()
    }

    pub fn task(&self) -> Option<&Task> {
        self.task.as_ref()
    }

    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }

    /// Stop is offered while thinking or visibly speaking (macOS `PetFeatureModel`).
    pub fn can_stop(&self) -> bool {
        self.in_call && (self.mode == Mode::Thinking || self.visual_mode() == Mode::Speaking)
    }

    /// Cancel is offered for a running task of a Live call; the Realtime engine refuses it.
    pub fn can_cancel_task(&self) -> bool {
        let running = matches!(&self.task, Some(task) if task.status == TaskStatus::Running);
        self.in_call && running && self.engine.as_deref() == Some(LIVE_ENGINE)
    }

    pub fn status(&self) -> Status {
        let mode = self.visual_mode();
        if mode == Mode::Error {
            let sentence = self
                .error
                .as_deref()
                .expect("an error carries its sentence");
            return Status {
                label: sentence.to_owned(),
                icon: "dialog-warning-symbolic",
                palette: Palette::Error,
            };
        }
        let (label, icon, palette) = look(mode);
        Status {
            label: label.to_owned(),
            icon,
            palette,
        }
    }

    /// The last caption fragment and who said it, verbatim: "You: …" or "Fermix: …".
    pub fn caption_line(&self) -> Option<String> {
        let caption = self.caption.as_ref()?;
        let who = match &caption.speaker {
            Speaker::User => "You",
            Speaker::Assistant => "Fermix",
            Speaker::Other(word) => word,
        };
        Some(format!("{who}: {}", caption.delta))
    }

    /// The task's state, as the value of the Voice page's Task row.
    pub fn task_line(&self) -> Option<String> {
        let line = match &self.task.as_ref()?.status {
            TaskStatus::Pending => "Queued".to_owned(),
            TaskStatus::Running => "Running".to_owned(),
            TaskStatus::Completed => "Finished".to_owned(),
            TaskStatus::Failed => "Did not finish".to_owned(),
            TaskStatus::Cancelled => "Cancelled".to_owned(),
            TaskStatus::Other(word) => word.clone(),
        };
        Some(line)
    }

    /// What the call's voice has cost, where the daemon said, as the value of the "Voice so
    /// far" row. The backend's share is the word `unknown`, never a number, so it is not in here.
    pub fn usage_line(&self) -> Option<String> {
        let cents = self.usage.as_ref()?.voice_cost_cents?;
        Some(format!("${:.2}", cents / 100.0))
    }

    fn input_mode(&self) -> Mode {
        if self.muted {
            Mode::Muted
        } else {
            Mode::Listening
        }
    }

    fn begin(&mut self) -> Vec<Effect> {
        if self.in_call {
            return Vec::new();
        }
        self.begin_when_connected = true;
        match self.link {
            Link::Ready => self.begin_call(),
            Link::Connecting => Vec::new(),
            Link::Offline => {
                self.link = Link::Connecting;
                self.mode = Mode::Connecting;
                self.error = None;
                vec![Effect::Connect]
            }
        }
    }

    /// Warms the pipeline with the gate shut and asks for the call. Everything the daemon said
    /// about the last call belongs to that call.
    fn begin_call(&mut self) -> Vec<Effect> {
        self.begin_when_connected = false;
        self.in_call = true;
        self.muted = false;
        self.armed = false;
        self.tail = false;
        self.level = 0.0;
        self.awaiting_listen = true;
        self.mode = Mode::Connecting;
        self.error = None;
        self.engine = None;
        self.call_id = None;
        self.caption = None;
        self.task = None;
        self.usage = None;
        self.call_number += 1;
        vec![
            Effect::Arm(false),
            Effect::MuteMic(false),
            Effect::FlushPlayback,
            Effect::StartAudio,
            Effect::Send(ClientEvent::CallStart),
            Effect::WatchCallStart(self.call_number),
        ]
    }

    /// Ends the call and keeps the connection for the next one.
    fn end(&mut self) -> Vec<Effect> {
        self.begin_when_connected = false;
        if !self.in_call {
            return Vec::new();
        }
        let mut effects = self.drop_call();
        effects.push(Effect::Send(ClientEvent::CallStop));
        self.mode = Mode::Idle;
        self.error = None;
        effects
    }

    /// Ends the call on this side with a sentence, and tells the daemon.
    fn fail_call(&mut self, sentence: String) -> Vec<Effect> {
        assert!(!sentence.is_empty(), "a failed call says why");
        if !self.in_call {
            return Vec::new();
        }
        let mut effects = self.drop_call();
        effects.push(Effect::Send(ClientEvent::CallStop));
        self.mode = Mode::Error;
        self.error = Some(sentence);
        effects
    }

    /// Clears the call's flags; if there was a call, shuts the gate and stops its audio.
    fn drop_call(&mut self) -> Vec<Effect> {
        let had_call = self.in_call;
        self.in_call = false;
        self.muted = false;
        self.armed = false;
        self.tail = false;
        self.level = 0.0;
        self.awaiting_listen = false;
        if !had_call {
            return Vec::new();
        }
        vec![Effect::Arm(false), Effect::StopAudio, Effect::FlushPlayback]
    }

    fn mute(&mut self, on: bool) -> Vec<Effect> {
        if !self.in_call {
            return Vec::new();
        }
        self.muted = on;
        if !self.awaiting_listen {
            self.mode = self.input_mode();
        }
        vec![
            Effect::MuteMic(on),
            Effect::Send(ClientEvent::Mute { enabled: on }),
        ]
    }

    /// Cuts the reply off here first, then tells the daemon how much of it was heard.
    fn stop(&mut self, played_ms: u64) -> Vec<Effect> {
        if !self.in_call {
            return Vec::new();
        }
        self.tail = false;
        self.level = 0.0;
        self.mode = self.input_mode();
        let interrupt = ClientEvent::Interrupt {
            audio_end_ms: Some(played_ms),
        };
        vec![Effect::FlushPlayback, Effect::Send(interrupt)]
    }

    fn cancel_task(&mut self) -> Vec<Effect> {
        match &self.task {
            Some(task) if self.can_cancel_task() => vec![Effect::Send(ClientEvent::TaskCancel {
                delegation_id: task.delegation_id.clone(),
            })],
            _ => Vec::new(),
        }
    }

    fn connected(&mut self) -> Vec<Effect> {
        assert_eq!(self.link, Link::Connecting, "Connected answers a Connect");
        self.link = Link::Ready;
        if self.begin_when_connected {
            return self.begin_call();
        }
        self.mode = Mode::Idle;
        Vec::new()
    }

    fn connect_failed(&mut self, error: &ConnectError) -> Vec<Effect> {
        assert_eq!(
            self.link,
            Link::Connecting,
            "ConnectFailed answers a Connect"
        );
        self.link = Link::Offline;
        self.begin_when_connected = false;
        self.error = Some(connect_sentence(error));
        self.mode = Mode::Error;
        Vec::new()
    }

    /// The connection is over. A close this app asked for changes nothing: the state already
    /// says so. Any other close tears the call down, keeping an error frame's words: the frame
    /// ended the call before the daemon hung up, where a tool error left it running. A call
    /// that was still up says it lost its connection; a close between calls stays quiet.
    fn closed(&mut self, reason: &CloseReason) -> Vec<Effect> {
        if *reason == CloseReason::Local {
            return Vec::new();
        }
        let error_frame = !self.in_call && self.mode == Mode::Error;
        let had_call = self.in_call;
        let mut effects = self.drop_call();
        self.link = Link::Offline;
        self.begin_when_connected = false;
        let lost = lost_sentence(reason).filter(|_| had_call);
        if matches!(
            reason,
            CloseReason::Framing(_) | CloseReason::LineTooLong(_)
        ) {
            self.mode = Mode::Error;
            self.error = Some(DISAGREED.into());
        } else if let Some(sentence) = lost {
            self.mode = Mode::Error;
            self.error = Some(sentence.into());
        } else if !error_frame {
            // Between calls: the next Begin reconnects, so nothing needs saying.
            self.mode = Mode::Offline;
            self.error = None;
        }
        effects.push(Effect::Disconnect);
        effects
    }

    fn drained(&mut self) -> Vec<Effect> {
        self.tail = false;
        self.level = 0.0;
        Vec::new()
    }

    fn call_deadline(&mut self, call: u64) -> Vec<Effect> {
        if call != self.call_number || !self.in_call || !self.awaiting_listen {
            return Vec::new();
        }
        self.fail_call(NOT_STARTED.into())
    }

    fn event(&mut self, event: ServerEvent) -> Vec<Effect> {
        match event {
            ServerEvent::State { state } => self.turn_state(&state),
            ServerEvent::AudioDelta { audio } => self.audio(rms(&audio)),
            ServerEvent::PlaybackStop => self.playback_stop(),
            ServerEvent::ToolEvent { status, reason, .. } => self.tool_event(&status, reason),
            ServerEvent::Error(error) => self.server_error(&error),
            ServerEvent::Task(task) => self.task_update(task),
            // Facts about the call, kept after it ends (the final usage comes after
            // `call_stop`) and never a change of mode.
            ServerEvent::Usage(usage) => {
                self.usage = Some(*usage);
                Vec::new()
            }
            ServerEvent::Caption(caption) => {
                self.caption = Some(caption);
                Vec::new()
            }
            ServerEvent::CallReady(ready) => {
                self.engine = Some(ready.engine);
                self.call_id = Some(ready.call_id);
                Vec::new()
            }
            ServerEvent::ServerHello { .. }
            | ServerEvent::TranscriptDelta { .. }
            | ServerEvent::AssistantTextDelta { .. }
            | ServerEvent::Unknown(_) => Vec::new(),
        }
    }

    fn turn_state(&mut self, state: &TurnState) -> Vec<Effect> {
        if !self.in_call {
            return Vec::new();
        }
        let mut effects = Vec::new();
        match state {
            TurnState::Muted => self.muted = true,
            TurnState::Idle => self.muted = false,
            _ => {}
        }
        if matches!(state, TurnState::Muted | TurnState::Idle) {
            effects.push(Effect::MuteMic(self.muted));
        }
        let was_speaking = self.mode == Mode::Speaking;
        self.mode = self.presented(state);
        self.error = None;
        if *state == TurnState::Listening && !self.armed {
            self.armed = true;
            effects.push(Effect::Arm(true));
        }
        if *state == TurnState::Listening {
            self.awaiting_listen = false;
        }
        if was_speaking && self.mode != Mode::Speaking {
            effects.push(Effect::ResetAnchor);
        }
        effects
    }

    fn presented(&self, state: &TurnState) -> Mode {
        match state {
            TurnState::Listening => self.input_mode(),
            TurnState::Speaking => Mode::Speaking,
            TurnState::Muted => Mode::Muted,
            TurnState::Thinking => Mode::Thinking,
            TurnState::Reconnecting => Mode::Reconnecting,
            TurnState::Idle | TurnState::Other(_) => Mode::Idle,
        }
    }

    fn audio(&mut self, rms: f32) -> Vec<Effect> {
        if !self.in_call {
            return Vec::new();
        }
        self.mode = Mode::Speaking;
        self.tail = true;
        self.level = smooth(self.level, rms);
        Vec::new()
    }

    fn playback_stop(&mut self) -> Vec<Effect> {
        self.tail = false;
        self.level = 0.0;
        if self.in_call {
            self.mode = self.input_mode();
        }
        vec![Effect::FlushPlayback]
    }

    fn tool_event(&mut self, status: &ToolStatus, reason: Option<String>) -> Vec<Effect> {
        if !self.in_call {
            return Vec::new();
        }
        match status {
            ToolStatus::Completed => self.mode = self.input_mode(),
            ToolStatus::Running | ToolStatus::Other(_) => self.mode = Mode::ToolUse,
            ToolStatus::Error => {
                self.mode = Mode::Error;
                self.error = Some(match reason {
                    Some(reason) => format!("The tool did not finish ({reason})."),
                    None => TOOL_FAILED.into(),
                });
            }
        }
        Vec::new()
    }

    /// Terminal for the call. The daemon ended it, so no `call_stop` goes back.
    fn server_error(&mut self, error: &ServerError) -> Vec<Effect> {
        let effects = self.drop_call();
        self.mode = Mode::Error;
        self.error = Some(error_sentence(error));
        effects
    }

    /// A task moves the mode like a tool. A frame from an earlier revision of the same task is
    /// late, and dropped.
    fn task_update(&mut self, task: Task) -> Vec<Effect> {
        let late = matches!(&self.task, Some(current)
            if current.delegation_id == task.delegation_id && task.revision < current.revision);
        if late {
            return Vec::new();
        }
        let finished = task.status.is_terminal();
        self.task = Some(task);
        if self.in_call {
            self.mode = if finished {
                self.input_mode()
            } else {
                Mode::ToolUse
            };
        }
        Vec::new()
    }
}

/// The word, icon and palette role of every mode but `Error`, which shows its sentence.
fn look(mode: Mode) -> (&'static str, &'static str, Palette) {
    match mode {
        Mode::Offline => ("Not connected", "network-offline-symbolic", Palette::Faint),
        Mode::Connecting => (
            "Connecting…",
            "content-loading-symbolic",
            Palette::Secondary,
        ),
        Mode::Idle => ("Ready", "call-start-symbolic", Palette::Secondary),
        Mode::Listening => (
            "Listening",
            "audio-input-microphone-symbolic",
            Palette::Accent,
        ),
        Mode::Muted => ("Muted", "microphone-disabled-symbolic", Palette::Warning),
        Mode::Thinking => ("Thinking", "emoji-objects-symbolic", Palette::Secondary),
        Mode::Speaking => ("Speaking", "audio-volume-high-symbolic", Palette::Success),
        Mode::ToolUse => (
            "Running a tool",
            "applications-engineering-symbolic",
            Palette::Secondary,
        ),
        // Fermix re-dialling OpenAI, not this app reconnecting: the call is at risk.
        Mode::Reconnecting => (
            "Reconnecting to OpenAI…",
            "view-refresh-symbolic",
            Palette::Warning,
        ),
        Mode::Error => unreachable!("an error shows its own sentence"),
    }
}

/// Every failed attempt says why: a silent failure would read as "Ready" again.
fn connect_sentence(error: &ConnectError) -> String {
    match error {
        ConnectError::NotFound | ConnectError::Refused => NOT_OPENED.into(),
        ConnectError::Rejected(refusal) => error_sentence(refusal),
        ConnectError::Timeout => NOT_ANSWERED.into(),
        ConnectError::Io(_) => CONNECTION_FAILED.into(),
    }
}

/// Why a call that was up lost its connection. `None` for the closes that
/// already have their words: this app's own, and a frame it could not read.
fn lost_sentence(reason: &CloseReason) -> Option<&'static str> {
    match reason {
        CloseReason::Local | CloseReason::Framing(_) | CloseReason::LineTooLong(_) => None,
        CloseReason::Stalled | CloseReason::ControlTimeout => Some(NOT_READING),
        CloseReason::PeerClosed | CloseReason::ReadFailed(_) | CloseReason::WriteFailed(_) => {
            Some(CONNECTION_CLOSED)
        }
    }
}

/// The sentence for a daemon refusal (spec §1.4 and §4.3). Realtime errors carry no `kind`,
/// so a rule matches the kind or the reason.
pub fn error_sentence(error: &ServerError) -> String {
    let is = |word: &str| error.reason == word || error.kind.as_deref() == Some(word);
    if is("unsupported_protocol_version") || is("update_required") {
        return update_sentence(error).into();
    }
    if let Some(sentence) = limit_sentence(&is) {
        return sentence.into();
    }
    if let Some(detail) = &error.detail {
        return detail.clone();
    }
    if let Some(text) = error.reason.strip_prefix("provider_send_failed: ") {
        return format!("OpenAI refused the voice call: {text}.");
    }
    if is("provider_refused") {
        return "OpenAI refused the voice call.".into();
    }
    let violation = VIOLATIONS.contains(&error.reason.as_str())
        || VIOLATION_TUPLES.iter().any(|t| error.reason.starts_with(t));
    if violation {
        return DISAGREED.into();
    }
    if error.reason.starts_with("{:session_down") {
        return "Voice stopped unexpectedly.".into();
    }
    // The daemon's own word, kept visibly a word rather than dressed as prose.
    format!("Fermix stopped voice ({}).", error.reason)
}

fn update_sentence(error: &ServerError) -> &'static str {
    match error.direction {
        Some(Direction::ClientTooOld) => "Update this app to talk to this version of Fermix.",
        Some(Direction::ClientTooNew) => "Update Fermix to talk to this app.",
        _ => "This app and Fermix need matching versions to talk. Update the older one.",
    }
}

/// The refusals whose words are the app's own, whatever detail came with them.
fn limit_sentence(is: &impl Fn(&str) -> bool) -> Option<&'static str> {
    let sentence = if is("max_clients_reached") {
        "Four other voice clients are already connected to Fermix."
    } else if is("not_configured") {
        "Add the OpenAI API key in Providers settings, or turn voice off. \
         A ChatGPT sign-in does not cover voice; it needs an OpenAI API key."
    } else if is("provider_disconnected") {
        "The connection to OpenAI dropped."
    } else if is("max_session_duration") {
        "This call reached the time limit set in Voice settings."
    } else if is("cost_limit") {
        "This call reached the spending limit set in Voice settings."
    } else {
        return None;
    };
    Some(sentence)
}
