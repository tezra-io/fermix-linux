//! The voice reducer: what each input does to the model and which effects it asks for. Ported
//! from the macOS `AppModelTests` voice routing and presentation suites, plus the Linux rules of
//! spec §1.4 and §1.6 (Reconnecting…, the 12 s call deadline, the tool `running` word, the
//! gate-then-`call_stop` order, the reason → sentence table).

use fermix_client::realtime::client::{CloseReason, ConnectError, Incoming};
use fermix_client::realtime::playback::{rms, voiced, PlaybackQueue};
use fermix_client::realtime::protocol::{
    decode_line, CallReady, Caption, ClientEvent, DecodeError, Direction, ServerError, ServerEvent,
    Speaker, Task, TaskStatus, ToolStatus, TurnState, Usage,
};
use fermix_client::realtime::session::{
    error_sentence, Effect, Input, Mode, Palette, Session, CALL_START_DEADLINE, METER_CEILING_DB,
    METER_FLOOR_DB, TRANSCRIPT_LINES,
};
use std::time::Duration;

fn event(session: &mut Session, event: ServerEvent) -> Vec<Effect> {
    session.apply(Input::Wire(Incoming::Event(event)))
}

fn state(session: &mut Session, state: TurnState) -> Vec<Effect> {
    event(session, ServerEvent::State { state })
}

/// A chunk of voice at the linear level `rms`.
fn audio(session: &mut Session, rms: f32) -> Vec<Effect> {
    session.apply(Input::Wire(Incoming::Audio {
        bytes: 4_800,
        rms,
        voiced: true,
    }))
}

/// A chunk under the voiced line, at the linear level `rms`: Live's padding.
fn unvoiced(session: &mut Session, rms: f32) -> Vec<Effect> {
    session.apply(Input::Wire(Incoming::Audio {
        bytes: 4_800,
        rms,
        voiced: false,
    }))
}

fn closed(session: &mut Session, reason: CloseReason) -> Vec<Effect> {
    session.apply(Input::Wire(Incoming::Closed(reason)))
}

/// Connected, with no call.
fn ready() -> Session {
    let mut session = Session::new();
    session.apply(Input::Begin);
    session.apply(Input::End);
    session.apply(Input::Connected);
    session
}

/// `call_start` sent, the daemon not listening yet.
fn calling() -> Session {
    let mut session = ready();
    session.apply(Input::Begin);
    session
}

fn listening() -> Session {
    let mut session = calling();
    state(&mut session, TurnState::Listening);
    session
}

fn refusal(reason: &str) -> ServerError {
    ServerError {
        reason: reason.into(),
        kind: None,
        detail: None,
        direction: None,
        client_version: None,
        min_version: None,
        max_version: None,
        required_for: None,
    }
}

fn task(revision: u64, status: TaskStatus) -> Task {
    Task {
        delegation_id: "dg_01H9".into(),
        revision,
        status,
        summary: None,
    }
}

fn live_ready() -> ServerEvent {
    ServerEvent::CallReady(CallReady {
        engine: "openai_live".into(),
        call_id: "voice_live:17".into(),
        provider_session_id: None,
        expires_at: None,
        captions: true,
    })
}

const TEARDOWN: [Effect; 3] = [Effect::Arm(false), Effect::StopAudio, Effect::FlushPlayback];

// Connecting and beginning

#[test]
fn a_fresh_session_is_offline_and_holds_no_call() {
    let session = Session::new();
    assert_eq!(session.mode(), Mode::Offline);
    assert!(!session.in_call());
    let status = session.status();
    assert_eq!(status.label, "Not connected");
    assert_eq!(status.icon, "network-offline-symbolic");
    assert_eq!(status.palette, Palette::Faint);
}

#[test]
fn begin_while_offline_connects_first_and_says_connecting() {
    let mut session = Session::new();
    assert_eq!(session.apply(Input::Begin), vec![Effect::Connect]);
    assert_eq!(session.mode(), Mode::Connecting);
    assert_eq!(session.status().label, "Connecting…");
    assert!(!session.in_call(), "no call before the handshake");
    assert!(
        session.apply(Input::Begin).is_empty(),
        "one connection attempt at a time"
    );
}

#[test]
fn the_handshake_completing_begins_the_asked_for_call_with_the_gate_shut() {
    let mut session = Session::new();
    session.apply(Input::Begin);
    let effects = session.apply(Input::Connected);
    assert_eq!(
        effects,
        vec![
            Effect::Arm(false),
            Effect::MuteMic(false),
            Effect::NewPlayback,
            Effect::StartAudio,
            Effect::Send(ClientEvent::CallStart),
            Effect::WatchCallStart(1),
        ]
    );
    assert!(session.in_call());
    assert!(!session.armed(), "nothing streams before listening");
    assert_eq!(session.status().label, "Connecting…");
}

#[test]
fn begin_on_a_live_connection_starts_the_call_at_once() {
    let mut session = ready();
    assert_eq!(session.mode(), Mode::Idle);
    assert_eq!(session.status().label, "Ready");
    let effects = session.apply(Input::Begin);
    assert!(effects.contains(&Effect::StartAudio));
    assert!(effects.contains(&Effect::Send(ClientEvent::CallStart)));
    assert!(!effects.contains(&Effect::Connect));
}

#[test]
fn end_while_connecting_cancels_the_call_and_the_handshake_lands_on_ready() {
    let mut session = Session::new();
    session.apply(Input::Begin);
    assert!(session.apply(Input::End).is_empty());
    assert!(session.apply(Input::Connected).is_empty());
    assert!(!session.in_call());
    assert_eq!(session.mode(), Mode::Idle);
}

#[test]
fn a_second_begin_during_a_call_does_nothing() {
    let mut session = listening();
    assert!(session.apply(Input::Begin).is_empty());
}

#[test]
fn a_missing_or_refused_socket_says_to_restart_fermix() {
    for failure in [ConnectError::NotFound, ConnectError::Refused] {
        let mut session = Session::new();
        session.apply(Input::Begin);
        assert!(session.apply(Input::ConnectFailed(failure)).is_empty());
        assert_eq!(session.mode(), Mode::Error);
        assert_eq!(
            session.status().label,
            "Voice is on, but Fermix has not opened its voice connection. \
             Restarting Fermix usually fixes this."
        );
        assert_eq!(session.status().palette, Palette::Error);
        assert!(!session.in_call());
    }
}

#[test]
fn a_handshake_that_times_out_says_fermix_did_not_answer() {
    let mut session = Session::new();
    session.apply(Input::Begin);
    session.apply(Input::ConnectFailed(ConnectError::Timeout));
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "Fermix did not answer the voice connection."
    );
    assert_eq!(
        session.apply(Input::Begin),
        vec![Effect::Connect],
        "the next Begin tries again"
    );
}

#[test]
fn a_connection_that_breaks_during_the_handshake_says_so() {
    let mut session = Session::new();
    session.apply(Input::Begin);
    let broken = ConnectError::Io(std::io::Error::other("broken pipe"));
    session.apply(Input::ConnectFailed(broken));
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "The voice connection to Fermix failed."
    );
}

#[test]
fn a_version_refusal_says_which_side_to_update() {
    let cases = [
        (
            Direction::ClientTooOld,
            "Update this app to talk to this version of Fermix.",
        ),
        (
            Direction::ClientTooNew,
            "Update Fermix to talk to this app.",
        ),
    ];
    for (direction, sentence) in cases {
        let mut session = Session::new();
        session.apply(Input::Begin);
        let mut error = refusal("unsupported_protocol_version");
        error.direction = Some(direction);
        session.apply(Input::ConnectFailed(ConnectError::Rejected(Box::new(
            error,
        ))));
        assert_eq!(session.mode(), Mode::Error);
        assert_eq!(session.status().label, sentence);
    }
}

#[test]
fn a_fifth_client_is_told_about_the_other_four() {
    let mut session = Session::new();
    session.apply(Input::Begin);
    let full = ConnectError::Rejected(Box::new(refusal("max_clients_reached")));
    session.apply(Input::ConnectFailed(full));
    assert_eq!(
        session.status().label,
        "Four other voice clients are already connected to Fermix."
    );
}

// Turn states

#[test]
fn listening_arms_streaming_once_and_reads_as_listening() {
    let mut session = calling();
    let effects = state(&mut session, TurnState::Listening);
    assert_eq!(effects, vec![Effect::Arm(true)]);
    assert!(session.armed());
    assert_eq!(session.mode(), Mode::Listening);
    let status = session.status();
    assert_eq!(status.label, "Listening");
    assert_eq!(status.icon, "audio-input-microphone-symbolic");
    assert_eq!(status.palette, Palette::Accent);
    assert!(
        state(&mut session, TurnState::Listening).is_empty(),
        "arming is idempotent"
    );
}

#[test]
fn a_muted_turn_state_mutes_the_microphone_and_reads_as_muted() {
    let mut session = listening();
    let effects = state(&mut session, TurnState::Muted);
    assert!(effects.contains(&Effect::MuteMic(true)));
    assert!(session.muted());
    assert_eq!(session.mode(), Mode::Muted);
    assert_eq!(session.status().palette, Palette::Warning);
}

#[test]
fn returning_to_idle_unmutes_the_microphone() {
    let mut session = listening();
    state(&mut session, TurnState::Muted);
    let effects = state(&mut session, TurnState::Idle);
    assert!(effects.contains(&Effect::MuteMic(false)));
    assert!(!session.muted());
}

#[test]
fn listening_while_muted_still_reads_as_muted() {
    let mut session = listening();
    state(&mut session, TurnState::Muted);
    state(&mut session, TurnState::Listening);
    assert_eq!(session.mode(), Mode::Muted);
}

#[test]
fn an_unknown_turn_state_reads_as_idle_without_losing_the_call() {
    let mut session = listening();
    state(&mut session, TurnState::Other("dreaming".into()));
    assert_eq!(session.mode(), Mode::Idle);
    assert!(session.in_call());
}

#[test]
fn reconnecting_is_said_rather_than_shown_as_idle() {
    let mut session = listening();
    state(&mut session, TurnState::Reconnecting);
    assert_eq!(session.mode(), Mode::Reconnecting);
    assert_eq!(session.status().label, "Reconnecting to OpenAI…");
    assert_eq!(
        session.status().palette,
        Palette::Warning,
        "the call is at risk"
    );
    assert_eq!(session.status().icon, "view-refresh-symbolic");
    assert!(session.in_call());
}

#[test]
fn thinking_reads_as_thinking() {
    let mut session = listening();
    state(&mut session, TurnState::Thinking);
    assert_eq!(session.status().label, "Thinking");
    assert_eq!(session.status().palette, Palette::Secondary);
}

#[test]
fn turn_states_outside_a_call_change_nothing() {
    let mut session = ready();
    assert!(state(&mut session, TurnState::Listening).is_empty());
    assert_eq!(session.mode(), Mode::Idle);
    assert!(!session.armed());
}

// Audio, the speaking tail, playback_stop

#[test]
fn audio_marks_the_speaking_tail_and_smooths_the_level() {
    let mut session = listening();
    assert!(audio(&mut session, 1.0).is_empty());
    assert_eq!(session.mode(), Mode::Speaking);
    assert!(session.speaking_tail());
    assert!((session.level() - 0.35).abs() < 1e-6);
    audio(&mut session, 1.0);
    assert!(session.level() > 0.35);
    let status = session.status();
    assert_eq!(status.label, "Speaking");
    assert_eq!(status.palette, Palette::Success);
}

/// The daemon returns to listening as soon as it stops generating, while buffered audio keeps
/// playing: the pet keeps its speaking look until the audio has actually drained.
#[test]
fn the_speaking_tail_outlasts_the_daemon_until_playback_drains() {
    let mut session = listening();
    audio(&mut session, 0.5);
    state(&mut session, TurnState::Listening);
    assert_eq!(session.mode(), Mode::Listening);
    assert_eq!(session.visual_mode(), Mode::Speaking);
    assert_eq!(session.status().label, "Speaking");

    assert!(session.apply(Input::Drained).is_empty());
    assert!(!session.speaking_tail());
    assert_eq!(session.visual_mode(), Mode::Listening);
    assert_eq!(session.level(), 0.0);
}

/// The Live engine publishes no end of a reply, so once its audio ran out nothing moved the mode
/// off speaking, and the pet kept its speaking face until the user next spoke (macOS 2ee3413).
/// Speaking was the session's own reading of arriving audio, so the drain ends it too, and
/// leaving speaking resets the anchor as a state frame does.
#[test]
fn a_reply_that_finishes_playing_returns_the_pet_to_listening() {
    let mut session = listening();
    audio(&mut session, 0.5);
    assert_eq!(session.mode(), Mode::Speaking);
    assert_eq!(session.apply(Input::Drained), [Effect::ResetAnchor]);
    assert_eq!(session.mode(), Mode::Listening);
    assert_eq!(session.visual_mode(), Mode::Listening);
    assert_eq!(session.status().label, "Listening");
}

#[test]
fn a_drained_reply_on_a_muted_call_returns_the_pet_to_muted() {
    let mut session = listening();
    session.apply(Input::Mute(true));
    audio(&mut session, 0.5);
    session.apply(Input::Drained);
    assert_eq!(session.mode(), Mode::Muted);
}

#[test]
fn a_drained_reply_spoken_over_running_work_returns_the_pet_to_that_work() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Running)),
    );
    audio(&mut session, 0.5);
    session.apply(Input::Drained);
    assert_eq!(session.mode(), Mode::ToolUse);
}

#[test]
fn a_drain_leaves_a_mode_the_daemon_set_alone() {
    let mut session = listening();
    state(&mut session, TurnState::Thinking);
    assert!(session.apply(Input::Drained).is_empty());
    assert_eq!(session.mode(), Mode::Thinking);
}

#[test]
fn leaving_speaking_resets_the_utterance_anchor() {
    let mut session = listening();
    audio(&mut session, 0.5);
    let effects = state(&mut session, TurnState::Listening);
    assert!(effects.contains(&Effect::ResetAnchor));
    assert!(
        !state(&mut session, TurnState::Thinking).contains(&Effect::ResetAnchor),
        "only the edge out of speaking resets it"
    );
}

#[test]
fn playback_stop_flushes_reports_what_played_and_returns_to_the_microphone() {
    let mut session = listening();
    audio(&mut session, 0.5);
    let effects = event(&mut session, ServerEvent::PlaybackStop);
    assert_eq!(effects, vec![Effect::FlushPlayback, Effect::ReportPlayback]);
    assert!(!session.speaking_tail());
    assert_eq!(session.mode(), Mode::Listening);
}

#[test]
fn audio_outside_a_call_does_not_fake_speaking() {
    let mut session = ready();
    audio(&mut session, 0.9);
    assert_eq!(session.visual_mode(), Mode::Idle);
    assert!(!session.speaking_tail());
}

/// As on macOS, where every chunk played feeds the pet's pulse: a chunk under the voiced line
/// moves the level, so a pause in the voice closes the mouth, but it never starts speaking nor
/// holds it.
#[test]
fn a_chunk_under_the_voiced_line_moves_the_level_but_never_shows_speaking() {
    let mut session = listening();
    assert!(unvoiced(&mut session, 0.001).is_empty());
    assert_eq!(session.visual_mode(), Mode::Listening);
    assert!(!session.speaking_tail());
    assert!((session.level() - 0.000_35).abs() < 1e-7);

    audio(&mut session, 1.0);
    let voiced_level = session.level();
    unvoiced(&mut session, 0.0);
    assert!((session.level() - 0.65 * voiced_level).abs() < 1e-6);
    assert_eq!(
        session.visual_mode(),
        Mode::Speaking,
        "the voice is still playing"
    );
    session.apply(Input::Drained);
    unvoiced(&mut session, 0.0);
    assert_eq!(session.visual_mode(), Mode::Listening);
    assert_eq!(session.level(), 0.0);
}

// A call's reply audio, as the app plays it

const SERVER: &str = include_str!("../contracts/realtime/fixtures/server_events.jsonl");
/// How long after a drain of voice the app reports it, unless voice is queued again by then
/// (the app's `DRAIN_GRACE`).
const GRACE_MS: u64 = 350;

/// One line of the contract's golden server events, as the daemon sends it.
fn golden(index: usize) -> Input {
    let line = SERVER
        .lines()
        .nth(index)
        .expect("the fixture has this line");
    let event = decode_line(line.as_bytes()).expect("a golden server line decodes");
    Input::Wire(Incoming::Event(event))
}

/// The contract's Live `call_ready` and Grok `call_ready`.
const LIVE_READY: usize = 9;
const GROK_READY: usize = 10;
/// The contract's `audio_delta`: digital silence, Live's padding.
const PADDING_DELTA: usize = 2;

/// `samples` of a steady `level`: its int16 RMS is `level`.
fn steady(level: i16, samples: usize) -> Vec<u8> {
    level.to_le_bytes().repeat(samples)
}

/// 100 ms of voice, at int16 RMS 2,000.
fn voice() -> Vec<u8> {
    steady(2_000, 2_400)
}

/// 100 ms of Live's padding, digital silence.
fn padding() -> Vec<u8> {
    steady(0, 2_400)
}

/// A call's reply audio as the app plays it. Each chunk goes to the playback queue and to the
/// session as the client hands it on, the speaker pulls 20 ms at a time, the queue's effects are
/// performed, and a drain of voice is reported `GRACE_MS` after it unless voice is queued again
/// by then, as the app's drain reporter does.
struct Played {
    session: Session,
    queue: PlaybackQueue,
    now_ms: u64,
    /// When the newest drain of voice is due to be reported.
    report_at: Option<u64>,
    /// Each pull that drained voice or emptied the queue: when, and which.
    edges: Vec<(u64, bool, bool)>,
}

impl Played {
    fn new(session: Session) -> Played {
        Played {
            session,
            queue: PlaybackQueue::new(),
            now_ms: 0,
            report_at: None,
            edges: Vec::new(),
        }
    }

    fn apply(&mut self, input: Input) {
        for effect in self.session.apply(input) {
            match effect {
                Effect::FlushPlayback => self.queue.clear(),
                Effect::ResetAnchor => self.queue.reset_anchor(),
                Effect::NewPlayback => self.queue.start_call(),
                _ => {}
            }
        }
    }

    fn state(&mut self, state: TurnState) {
        self.apply(Input::Wire(Incoming::Event(ServerEvent::State { state })));
    }

    /// One `audio_delta`, as the client hands it on.
    fn arrive(&mut self, pcm: &[u8], playback_id: Option<&str>) {
        self.queue.push_pcm16(pcm, playback_id);
        self.apply(Input::Wire(Incoming::Audio {
            bytes: pcm.len(),
            rms: rms(pcm),
            voiced: voiced(pcm),
        }));
    }

    /// Plays `ms`, 20 ms a pull.
    fn play(&mut self, ms: u64) {
        for _ in 0..ms / 20 {
            let pull = self.queue.pull(&mut [0; 480]);
            self.now_ms += 20;
            if pull.drained || pull.emptied {
                self.edges.push((self.now_ms, pull.drained, pull.emptied));
            }
            if pull.drained {
                self.report_at = Some(self.now_ms + GRACE_MS);
            }
            if self.report_at.is_some_and(|at| self.now_ms >= at) {
                self.report_at = None;
                if !self.queue.voice_queued() {
                    self.apply(Input::Drained);
                }
            }
        }
    }

    /// One chunk arriving, then 100 ms played: Live's pace, one chunk every 100 ms.
    fn chunk(&mut self, pcm: &[u8]) {
        self.arrive(pcm, None);
        self.play(100);
    }

    fn shows(&self) -> Mode {
        self.session.visual_mode()
    }
}

/// The contract's Live call sequence: `listening`, `call_ready`, then audio every 100 ms for
/// the whole call, padding before a reply's voice and after it.
#[test]
fn a_live_reply_shows_speaking_through_its_voice_and_listening_once_it_has_played() {
    let mut call = Played::new(calling());
    call.apply(golden(1));
    call.apply(golden(LIVE_READY));
    assert_eq!(call.session.engine(), Some("openai_live"));
    for _ in 0..5 {
        call.chunk(&padding());
        assert_eq!(call.shows(), Mode::Listening, "padding before the reply");
    }
    for chunk in 1..=5 {
        call.chunk(&voice());
        assert_eq!(call.shows(), Mode::Speaking, "voice chunk {chunk}");
    }
    // The network runs a chunk ahead: padding stays queued from here on.
    call.arrive(&padding(), None);
    call.chunk(&padding());
    assert_eq!(
        call.shows(),
        Mode::Speaking,
        "the voice has only just played"
    );
    for _ in 0..3 {
        call.chunk(&padding());
    }
    assert_eq!(call.shows(), Mode::Listening);
    assert_eq!(call.session.mode(), Mode::Listening);
    assert!(!call.session.speaking_tail());
    assert!(!call.queue.is_empty(), "with padding still queued");
    for _ in 0..20 {
        call.chunk(&padding());
        assert_eq!(call.shows(), Mode::Listening, "padding after the reply");
    }
    assert_eq!(call.session.level(), 0.0, "digital silence");
    let drains: Vec<u64> = call.edges.iter().filter(|e| e.1).map(|e| e.0).collect();
    assert_eq!(
        drains,
        [600, 700, 800, 900, 1_000],
        "each voice chunk's end"
    );
}

#[test]
fn a_live_reply_over_running_work_returns_to_that_work_with_padding_still_arriving() {
    let mut call = Played::new(listening());
    call.apply(golden(LIVE_READY));
    call.apply(Input::Wire(Incoming::Event(ServerEvent::Task(task(
        1,
        TaskStatus::Running,
    )))));
    call.chunk(&padding());
    assert_eq!(call.shows(), Mode::ToolUse);
    for _ in 0..3 {
        call.chunk(&voice());
        assert_eq!(call.shows(), Mode::Speaking);
    }
    for _ in 0..6 {
        call.chunk(&padding());
    }
    assert_eq!(call.shows(), Mode::ToolUse, "the drain returns to the work");
    call.apply(Input::Wire(Incoming::Event(ServerEvent::Task(task(
        1,
        TaskStatus::Completed,
    )))));
    call.chunk(&padding());
    assert_eq!(call.shows(), Mode::Listening);
}

/// Live's padding, measured at int16 RMS 0 to 49, runs from the call's first second to its
/// last. Alone it is no reply: the pet never speaks for it, and it takes the turn from no state
/// the daemon set.
#[test]
fn lives_padding_alone_never_shows_speaking() {
    let mut call = Played::new(listening());
    call.apply(golden(LIVE_READY));
    for level in [0, 49, 12, 49, 0] {
        call.chunk(&steady(level, 2_400));
        assert_eq!(call.shows(), Mode::Listening, "padding at {level}");
        assert!(!call.session.speaking_tail());
    }
    call.apply(golden(PADDING_DELTA));
    assert_eq!(call.shows(), Mode::Listening, "the contract's own padding");
    call.state(TurnState::Thinking);
    for _ in 0..10 {
        call.chunk(&padding());
    }
    assert_eq!(call.shows(), Mode::Thinking);
    assert!(call.edges.iter().all(|edge| !edge.1), "no drain of voice");
}

#[test]
fn the_daemons_state_still_moves_the_pet_while_padding_arrives() {
    let mut call = Played::new(listening());
    call.apply(golden(LIVE_READY));
    call.state(TurnState::Speaking);
    call.chunk(&padding());
    assert_eq!(call.shows(), Mode::Speaking, "the daemon said speaking");
    call.state(TurnState::Listening);
    call.chunk(&padding());
    assert_eq!(
        call.shows(),
        Mode::Listening,
        "padding holds no speaking look"
    );
    call.state(TurnState::Thinking);
    call.chunk(&padding());
    assert_eq!(call.shows(), Mode::Thinking);

    call.arrive(&voice(), None);
    call.arrive(&voice(), None);
    call.state(TurnState::Listening);
    call.chunk(&padding());
    assert_eq!(
        call.shows(),
        Mode::Speaking,
        "the voice outlasts the daemon's listening"
    );
    for _ in 0..6 {
        call.chunk(&padding());
    }
    assert_eq!(call.shows(), Mode::Listening);
    call.state(TurnState::Muted);
    call.chunk(&padding());
    assert_eq!(call.shows(), Mode::Muted);
}

/// The Grok call sequence sends no padding: when every chunk of a reply is voice, its voice ends
/// where its audio does, so the pet behaves as it did before padding was told apart, and its
/// playback id drains with it.
#[test]
fn a_reply_without_padding_shows_speaking_until_its_audio_has_played_as_before() {
    let mut call = Played::new(listening());
    call.apply(golden(GROK_READY));
    call.state(TurnState::Speaking);
    for level in [3_667, 244, 1_500] {
        call.arrive(&steady(level, 2_400), Some("p1"));
    }
    call.state(TurnState::Listening);
    call.play(280);
    assert_eq!(call.shows(), Mode::Speaking);
    call.play(20);
    assert_eq!(
        call.edges,
        [(300, true, true)],
        "the drain is the queue's end"
    );
    // Reported at the first pull past 300 + GRACE_MS: 660.
    call.play(340);
    assert_eq!(call.shows(), Mode::Speaking, "within the grace");
    call.play(20);
    assert_eq!(call.shows(), Mode::Listening);
    let report = call.queue.take_progress();
    assert_eq!(report.len(), 1);
    assert_eq!((report[0].played_ms, report[0].drained), (300, true));
}

/// The one change for engines that send no padding, and on purpose: a reply whose last chunks
/// are under the voiced line has played its voice before they have played. The pet leaves
/// speaking once the drain of its last voiced chunk has stood for the grace, while the quiet
/// tail still plays, as on macOS, where only voiced buffers keep a reply speaking.
#[test]
fn a_reply_without_padding_leaves_speaking_while_its_quiet_last_chunks_still_play() {
    let mut call = Played::new(listening());
    call.apply(golden(GROK_READY));
    call.state(TurnState::Speaking);
    for _ in 0..2 {
        call.arrive(&voice(), Some("p1"));
    }
    for _ in 0..5 {
        call.arrive(&steady(60, 2_400), Some("p1")); // int16 RMS 60: under the line
    }
    call.state(TurnState::Listening);
    call.play(200);
    assert_eq!(
        call.edges,
        [(200, true, false)],
        "the voice's end, the tail queued"
    );
    // Reported at the first pull past 200 + GRACE_MS: 560.
    call.play(340);
    assert_eq!(call.shows(), Mode::Speaking, "within the grace");
    call.play(20);
    assert_eq!(call.shows(), Mode::Listening);
    assert!(!call.queue.is_empty(), "the quiet tail still plays");
    call.play(140);
    assert_eq!(call.edges, [(200, true, false), (700, false, true)]);
}

// Tools and tasks

#[test]
fn the_daemons_running_tool_reads_as_a_tool_and_completed_returns_to_listening() {
    let mut session = listening();
    let running = ServerEvent::ToolEvent {
        status: ToolStatus::Running,
        name: Some("calendar".into()),
        reason: None,
    };
    event(&mut session, running);
    assert_eq!(session.mode(), Mode::ToolUse);
    assert_eq!(session.status().label, "Running a tool");

    let completed = ServerEvent::ToolEvent {
        status: ToolStatus::Completed,
        name: Some("calendar".into()),
        reason: None,
    };
    event(&mut session, completed);
    assert_eq!(session.mode(), Mode::Listening);
}

#[test]
fn a_tool_status_this_build_cannot_read_still_reads_as_a_tool() {
    let mut session = listening();
    let odd = ServerEvent::ToolEvent {
        status: ToolStatus::Other("paused".into()),
        name: None,
        reason: None,
    };
    event(&mut session, odd);
    assert_eq!(session.mode(), Mode::ToolUse);
}

#[test]
fn a_failed_tool_carries_the_daemons_reason_and_keeps_the_call() {
    let mut session = listening();
    let failed = ServerEvent::ToolEvent {
        status: ToolStatus::Error,
        name: Some("write".into()),
        reason: Some("write_refused".into()),
    };
    assert!(event(&mut session, failed).is_empty());
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "The tool did not finish (write_refused)."
    );
    assert!(session.in_call());

    state(&mut session, TurnState::Listening);
    assert_eq!(session.mode(), Mode::Listening, "the next state moves on");
    assert_eq!(session.error(), None);
}

#[test]
fn a_failed_tool_without_a_reason_says_it_did_not_finish() {
    let mut session = listening();
    let failed = ServerEvent::ToolEvent {
        status: ToolStatus::Error,
        name: None,
        reason: None,
    };
    event(&mut session, failed);
    assert_eq!(session.status().label, "The tool did not finish.");
}

#[test]
fn a_running_task_reads_as_a_tool_and_a_finished_one_returns_to_the_microphone() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Running)),
    );
    assert_eq!(session.mode(), Mode::ToolUse);
    assert_eq!(session.task_line().as_deref(), Some("Running"));

    let mut done = task(1, TaskStatus::Completed);
    done.summary = Some("checked".into());
    event(&mut session, ServerEvent::Task(done.clone()));
    assert_eq!(session.mode(), Mode::Listening);
    assert_eq!(session.task(), Some(&done));
    assert_eq!(session.task_line().as_deref(), Some("Finished"));
}

#[test]
fn a_failed_task_returns_to_the_microphone_and_keeps_its_summary() {
    let mut session = listening();
    let mut failed = task(2, TaskStatus::Failed);
    failed.summary = Some("the calendar refused".into());
    event(&mut session, ServerEvent::Task(failed));
    assert_eq!(session.mode(), Mode::Listening);
    assert_eq!(
        session.task().and_then(|t| t.summary.as_deref()),
        Some("the calendar refused")
    );
    assert_eq!(session.task_line().as_deref(), Some("Did not finish"));
}

#[test]
fn a_task_status_this_build_cannot_read_does_not_end_the_work() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Other("paused".into()))),
    );
    assert_eq!(session.mode(), Mode::ToolUse);
    assert_eq!(session.task_line().as_deref(), Some("paused"));
}

#[test]
fn a_late_frame_from_an_earlier_revision_is_dropped() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::Task(task(2, TaskStatus::Running)),
    );
    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Completed)),
    );
    assert_eq!(session.task(), Some(&task(2, TaskStatus::Running)));
    assert_eq!(session.mode(), Mode::ToolUse);
}

#[test]
fn cancel_is_sent_only_for_a_running_task_on_the_live_engine() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Running)),
    );
    assert!(!session.can_cancel_task(), "the Realtime engine refuses it");
    assert!(session.apply(Input::CancelTask).is_empty());

    event(&mut session, live_ready());
    assert!(session.can_cancel_task());
    assert_eq!(
        session.apply(Input::CancelTask),
        vec![Effect::Send(ClientEvent::TaskCancel {
            delegation_id: "dg_01H9".into()
        })]
    );

    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Pending)),
    );
    assert!(!session.can_cancel_task(), "nothing to call off yet");
}

#[test]
fn a_grok_call_can_cancel_its_task_as_a_live_call_does() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::CallReady(CallReady {
            engine: "xai_realtime".into(),
            call_id: "voice_live:18".into(),
            provider_session_id: None,
            expires_at: None,
            captions: true,
        }),
    );
    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Running)),
    );
    assert!(session.can_cancel_task());
}

// Call facts: call_ready, the transcript, usage

#[test]
fn call_ready_records_the_engine_and_call_without_moving_the_mode() {
    let mut session = listening();
    assert!(event(&mut session, live_ready()).is_empty());
    assert_eq!(session.engine(), Some("openai_live"));
    assert_eq!(session.call_id(), Some("voice_live:17"));
    assert_eq!(session.mode(), Mode::Listening);
}

fn caption(speaker: Speaker, delta: &str) -> ServerEvent {
    ServerEvent::Caption(Caption {
        speaker,
        delta: delta.into(),
        start_ms: 0,
        end_ms: 440,
    })
}

fn said(text: &str) -> ServerEvent {
    ServerEvent::TranscriptDelta {
        text: text.into(),
        role: Some("user".into()),
    }
}

fn answered(text: &str) -> ServerEvent {
    ServerEvent::AssistantTextDone { text: text.into() }
}

fn lines(session: &Session) -> Vec<(&str, &str)> {
    session
        .transcript()
        .iter()
        .map(|line| (line.who(), line.text.as_str()))
        .collect()
}

#[test]
fn what_you_said_and_what_fermix_answered_are_kept_in_order() {
    let mut session = listening();
    assert!(session.transcript().is_empty());
    assert!(event(&mut session, said("what time is it")).is_empty());
    assert!(event(&mut session, answered("It is noon.")).is_empty());
    assert_eq!(
        lines(&session),
        [("You", "what time is it"), ("Fermix", "It is noon.")]
    );
    assert_eq!(session.mode(), Mode::Listening);
}

/// The reply's streamed deltas are the same words as its final text, which ends the turn.
#[test]
fn a_reply_is_a_line_once_it_is_done_and_its_deltas_add_nothing() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::AssistantTextDelta { text: "It ".into() },
    );
    event(
        &mut session,
        ServerEvent::AssistantTextDelta { text: "is".into() },
    );
    assert!(session.transcript().is_empty());
    event(&mut session, answered("It is"));
    assert_eq!(lines(&session), [("Fermix", "It is")]);
}

#[test]
fn each_utterance_is_its_own_line_and_one_with_no_words_adds_none() {
    let mut session = listening();
    event(&mut session, said("open the"));
    event(&mut session, said(" \n"));
    event(&mut session, said("browser"));
    assert_eq!(lines(&session), [("You", "open the"), ("You", "browser")]);
}

#[test]
fn a_transcript_without_a_role_is_yours_and_another_role_keeps_its_word() {
    let mut session = listening();
    let unlisted = ServerEvent::TranscriptDelta {
        text: "hello".into(),
        role: None,
    };
    event(&mut session, unlisted);
    let other = ServerEvent::TranscriptDelta {
        text: "so".into(),
        role: Some("narrator".into()),
    };
    event(&mut session, other);
    assert_eq!(lines(&session), [("You", "hello"), ("narrator", "so")]);
}

/// Live captions are fragments of one stream per speaker, joined verbatim.
#[test]
fn live_captions_join_into_their_speakers_line_until_the_speaker_changes() {
    let mut session = listening();
    assert!(event(&mut session, caption(Speaker::User, "what is ")).is_empty());
    event(&mut session, caption(Speaker::User, "the time"));
    event(&mut session, caption(Speaker::Assistant, "It is "));
    event(&mut session, caption(Speaker::Assistant, "noon."));
    event(&mut session, caption(Speaker::User, "thanks"));
    assert_eq!(
        lines(&session),
        [
            ("You", "what is the time"),
            ("Fermix", "It is noon."),
            ("You", "thanks")
        ]
    );
}

#[test]
fn the_transcript_keeps_only_its_last_lines() {
    let mut session = listening();
    for n in 0..TRANSCRIPT_LINES + 3 {
        event(&mut session, said(&format!("line {n}")));
    }
    let kept = lines(&session);
    assert_eq!(kept.len(), TRANSCRIPT_LINES);
    assert_eq!(kept[0], ("You", "line 3"));
    assert_eq!(
        kept[TRANSCRIPT_LINES - 1].1,
        format!("line {}", TRANSCRIPT_LINES + 2)
    );
}

/// It stays to be read after the call, and the next call starts a fresh one.
#[test]
fn the_transcript_outlasts_its_call_until_the_next_one_begins() {
    let mut session = listening();
    event(&mut session, said("bye"));
    session.apply(Input::End);
    event(&mut session, answered("Bye!"));
    assert_eq!(lines(&session), [("You", "bye"), ("Fermix", "Bye!")]);
    session.apply(Input::Begin);
    assert!(session.transcript().is_empty());
}

// The microphone meter

/// What the microphone sends, as the linear RMS of a block, read on a decibel scale.
#[test]
fn the_meter_reads_the_sent_level_in_decibels() {
    let mut session = listening();
    let level = |db: f32| 10f32.powf(db / 20.0);
    let cases = [
        (0.0, 0.0),
        (level(-80.0), 0.0),
        (level(METER_FLOOR_DB), 0.0),
        (level((METER_FLOOR_DB + METER_CEILING_DB) / 2.0), 0.5),
        (level(METER_CEILING_DB), 1.0),
        (1.0, 1.0),
    ];
    for (rms, meter) in cases {
        assert!(session.apply(Input::MicLevel(rms)).is_empty());
        let shown = session.mic_level().expect("the microphone streams");
        assert!(
            (shown - meter).abs() < 1e-4,
            "{rms} read as {shown}, not {meter}"
        );
    }
}

#[test]
fn the_meter_shows_only_while_the_microphone_streams() {
    let mut session = calling();
    session.apply(Input::MicLevel(0.1));
    assert_eq!(session.mic_level(), None, "not armed yet");
    state(&mut session, TurnState::Listening);
    assert_eq!(
        session.mic_level(),
        Some(0.0),
        "armed, nothing measured yet"
    );
    session.apply(Input::MicLevel(0.1));
    assert!(session.mic_level().unwrap() > 0.0);
    session.apply(Input::Mute(true));
    assert_eq!(session.mic_level(), None);
    session.apply(Input::Mute(false));
    assert_eq!(
        session.mic_level(),
        Some(0.0),
        "a muted block is never shown again"
    );
    session.apply(Input::End);
    assert_eq!(session.mic_level(), None);
}

#[test]
#[should_panic(expected = "a level is a finite RMS")]
fn a_level_that_is_not_an_rms_is_a_bug() {
    listening().apply(Input::MicLevel(f32::NAN));
}

/// A bill is a fact, not a state: what a limit does to the call arrives as its own error.
#[test]
fn a_usage_frame_is_recorded_and_changes_no_state() {
    let mut session = listening();
    let usage = Usage {
        status: Some("live".into()),
        voice_seconds: Some(64.2),
        voice_cost_cents: Some(5.35),
        backend_turns: Some(2),
        backend_cost: Some("unknown".into()),
        accounting: Some("running".into()),
        ..Usage::default()
    };
    assert!(event(&mut session, ServerEvent::Usage(Box::new(usage.clone()))).is_empty());
    assert_eq!(session.usage(), Some(&usage));
    assert_eq!(session.usage_line().as_deref(), Some("$0.05"));
    assert_eq!(session.mode(), Mode::Listening);
    assert!(session.in_call());
}

#[test]
fn usage_without_a_voice_cost_shows_no_money_line() {
    let mut session = listening();
    let realtime = Usage {
        status: Some("reported".into()),
        cost_cents: Some(1.84),
        ..Usage::default()
    };
    event(&mut session, ServerEvent::Usage(Box::new(realtime)));
    assert_eq!(session.usage_line(), None);
}

#[test]
fn a_new_call_does_not_inherit_the_last_calls_facts() {
    let mut session = listening();
    event(&mut session, live_ready());
    event(&mut session, caption(Speaker::User, "what is "));
    event(
        &mut session,
        ServerEvent::Task(task(1, TaskStatus::Running)),
    );
    let usage = Usage {
        voice_cost_cents: Some(5.35),
        ..Usage::default()
    };
    event(&mut session, ServerEvent::Usage(Box::new(usage)));
    session.apply(Input::End);

    session.apply(Input::Begin);
    assert_eq!(session.engine(), None);
    assert_eq!(session.call_id(), None);
    assert!(session.transcript().is_empty());
    assert_eq!(session.task(), None);
    assert_eq!(session.usage(), None);
}

#[test]
fn the_final_usage_after_the_call_ended_is_still_recorded() {
    let mut session = listening();
    session.apply(Input::End);
    let settled = Usage {
        voice_cost_cents: Some(7.0),
        accounting: Some("complete".into()),
        ..Usage::default()
    };
    event(&mut session, ServerEvent::Usage(Box::new(settled)));
    assert_eq!(session.usage_line().as_deref(), Some("$0.07"));
}

#[test]
fn reply_deltas_unknown_events_and_a_second_hello_change_nothing() {
    let quiet = [
        ServerEvent::AssistantTextDelta { text: "hi".into() },
        ServerEvent::Unknown("weather".into()),
        ServerEvent::ServerHello {
            min_version: 1,
            max_version: 2,
        },
    ];
    for frame in quiet {
        let mut session = listening();
        let before = session.clone();
        assert!(event(&mut session, frame).is_empty());
        assert_eq!(session, before);
    }
}

// The user's controls

#[test]
fn mute_shuts_the_microphone_tells_the_daemon_and_reads_as_muted() {
    let mut session = listening();
    assert_eq!(
        session.apply(Input::Mute(true)),
        vec![
            Effect::MuteMic(true),
            Effect::Send(ClientEvent::Mute { enabled: true })
        ]
    );
    assert_eq!(session.mode(), Mode::Muted);
    assert_eq!(session.status().icon, "microphone-disabled-symbolic");
    session.apply(Input::Mute(false));
    assert_eq!(session.mode(), Mode::Listening);
    assert!(!session.muted());
}

#[test]
fn mute_before_the_daemon_listens_keeps_saying_connecting() {
    let mut session = calling();
    session.apply(Input::Mute(true));
    assert!(session.muted());
    assert_eq!(session.mode(), Mode::Connecting);
}

#[test]
fn mute_outside_a_call_does_nothing() {
    let mut session = ready();
    assert!(session.apply(Input::Mute(true)).is_empty());
    assert!(!session.muted());
}

/// `played_ms` is read before the flush: clearing the queue resets its anchor. What played of
/// each playback id goes out before `interrupt`, which is where a Grok call cuts the reply.
#[test]
fn stop_flushes_locally_reports_playback_then_sends_interrupt_with_what_was_played() {
    let mut session = listening();
    audio(&mut session, 0.5);
    let effects = session.apply(Input::Stop { played_ms: 1_500 });
    assert_eq!(
        effects,
        vec![
            Effect::FlushPlayback,
            Effect::ReportPlayback,
            Effect::Send(ClientEvent::Interrupt {
                audio_end_ms: Some(1_500)
            }),
        ]
    );
    assert!(!session.speaking_tail());
    assert_eq!(session.level(), 0.0);
    assert_eq!(session.mode(), Mode::Listening);
    assert!(session.in_call());
}

#[test]
fn stop_outside_a_call_does_nothing() {
    let mut session = ready();
    assert!(session.apply(Input::Stop { played_ms: 10 }).is_empty());
}

#[test]
fn stop_is_offered_only_while_thinking_or_visibly_speaking() {
    let mut session = listening();
    assert!(!session.can_stop());
    state(&mut session, TurnState::Thinking);
    assert!(session.can_stop());
    state(&mut session, TurnState::Listening);
    audio(&mut session, 0.5);
    state(&mut session, TurnState::Listening);
    assert!(session.can_stop(), "the tail is still playing");
    session.apply(Input::Drained);
    assert!(!session.can_stop());
}

/// The gate shuts and the pipeline stops before `call_stop` goes out: audio after it is
/// `not_connected` to the daemon, which then hangs up.
#[test]
fn end_shuts_the_microphone_then_sends_call_stop_and_keeps_the_connection() {
    let mut session = listening();
    let effects = session.apply(Input::End);
    let mut expected = TEARDOWN.to_vec();
    expected.push(Effect::Send(ClientEvent::CallStop));
    assert_eq!(effects, expected);
    assert!(!effects.contains(&Effect::Disconnect));
    assert!(!session.in_call());
    assert!(!session.armed());
    assert_eq!(session.mode(), Mode::Idle);
    assert_eq!(session.status().label, "Ready");

    assert!(
        state(&mut session, TurnState::Idle).is_empty(),
        "the daemon's idle answer"
    );
    assert_eq!(session.mode(), Mode::Idle);
}

// Deadlines and failures

#[test]
fn the_call_start_deadline_is_twelve_seconds() {
    assert_eq!(CALL_START_DEADLINE, Duration::from_secs(12));
}

#[test]
fn a_call_the_daemon_never_starts_ends_with_a_sentence() {
    let mut session = calling();
    let effects = session.apply(Input::CallDeadline(1));
    let mut expected = TEARDOWN.to_vec();
    expected.push(Effect::Send(ClientEvent::CallStop));
    assert_eq!(effects, expected);
    assert!(!session.in_call());
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "Fermix did not start the call in time."
    );
}

#[test]
fn the_deadline_is_moot_once_the_daemon_listens() {
    let mut session = listening();
    assert!(session.apply(Input::CallDeadline(1)).is_empty());
    assert!(session.in_call());
}

#[test]
fn a_deadline_left_over_from_an_earlier_call_is_ignored() {
    let mut session = calling();
    session.apply(Input::End);
    let effects = session.apply(Input::Begin);
    assert!(effects.contains(&Effect::WatchCallStart(2)));
    assert!(session.apply(Input::CallDeadline(1)).is_empty());
    assert!(session.in_call());
}

#[test]
fn an_audio_failure_ends_the_call_with_its_own_sentence() {
    let mut session = listening();
    let sentence = "No microphone is connected.";
    let effects = session.apply(Input::AudioFailed(sentence.into()));
    let mut expected = TEARDOWN.to_vec();
    expected.push(Effect::Send(ClientEvent::CallStop));
    assert_eq!(effects, expected);
    assert!(!session.in_call());
    assert_eq!(session.status().label, sentence);
    assert_eq!(session.status().icon, "dialog-warning-symbolic");
}

/// An error frame may mean the socket is unusable, so the microphone is torn down before an
/// in-flight buffer can race back to it.
#[test]
fn a_server_error_tears_the_call_down_in_the_daemons_words() {
    let mut session = listening();
    audio(&mut session, 0.5);
    state(&mut session, TurnState::Muted);
    let effects = event(&mut session, ServerEvent::Error(refusal("cost_limit")));
    assert_eq!(effects, TEARDOWN.to_vec());
    assert!(!session.in_call());
    assert!(!session.muted());
    assert!(!session.speaking_tail());
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "This call reached the spending limit set in Voice settings."
    );
}

/// The daemon's frame when OpenAI refuses the key before the call is configured
/// (engine fix of 2026-09-26): the start ends at once, in the daemon's sentence,
/// instead of waiting out the start deadline.
#[test]
fn a_refused_key_ends_the_start_in_the_daemons_sentence() {
    let mut session = calling();
    let frame = br#"{"type":"error","reason":"provider_refused","kind":"provider_refused","detail":"OpenAI did not accept the API key (invalid_api_key)."}"#;
    let refused = decode_line(frame).expect("the engine's refusal frame decodes");
    event(&mut session, refused);
    assert!(!session.in_call());
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "OpenAI did not accept the API key (invalid_api_key)."
    );
    assert!(session.apply(Input::CallDeadline(1)).is_empty());
}

#[test]
fn losing_the_connection_mid_call_tears_down_and_says_so() {
    let mut session = listening();
    let effects = closed(&mut session, CloseReason::PeerClosed);
    let mut expected = TEARDOWN.to_vec();
    expected.push(Effect::Disconnect);
    assert_eq!(effects, expected);
    assert!(!session.in_call());
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "The voice connection to Fermix closed."
    );
    assert_eq!(session.apply(Input::Begin), vec![Effect::Connect]);
}

#[test]
fn a_daemon_that_stops_reading_mid_call_is_named() {
    for reason in [CloseReason::Stalled, CloseReason::ControlTimeout] {
        let mut session = listening();
        closed(&mut session, reason.clone());
        assert_eq!(session.mode(), Mode::Error, "{reason:?}");
        assert_eq!(session.status().label, "Fermix stopped taking voice audio.");
    }
}

#[test]
fn a_close_between_calls_stays_quiet() {
    let mut session = ready();
    assert_eq!(
        closed(&mut session, CloseReason::PeerClosed),
        vec![Effect::Disconnect]
    );
    assert_eq!(session.mode(), Mode::Offline, "the next Begin reconnects");
    assert_eq!(session.error(), None);
}

#[test]
fn the_hang_up_after_an_error_frame_keeps_the_errors_words() {
    let mut session = listening();
    event(
        &mut session,
        ServerEvent::Error(refusal("max_session_duration")),
    );
    assert_eq!(
        closed(&mut session, CloseReason::PeerClosed),
        vec![Effect::Disconnect]
    );
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.status().label,
        "This call reached the time limit set in Voice settings."
    );
}

#[test]
fn a_hang_up_after_a_tool_error_says_the_connection_closed_not_the_tools_words() {
    let mut session = listening();
    let failed = ServerEvent::ToolEvent {
        status: ToolStatus::Error,
        name: None,
        reason: Some("write_refused".into()),
    };
    event(&mut session, failed);
    closed(&mut session, CloseReason::PeerClosed);
    assert_eq!(session.mode(), Mode::Error);
    assert_eq!(
        session.error(),
        Some("The voice connection to Fermix closed.")
    );
}

#[test]
fn the_error_sentence_is_only_reported_while_the_mode_is_error() {
    let mut session = listening();
    let failed = ServerEvent::ToolEvent {
        status: ToolStatus::Error,
        name: None,
        reason: Some("write_refused".into()),
    };
    event(&mut session, failed);
    assert_eq!(
        session.error(),
        Some("The tool did not finish (write_refused).")
    );
    audio(&mut session, 0.5);
    assert_eq!(session.mode(), Mode::Speaking);
    assert_eq!(session.error(), None);
}

#[test]
fn a_frame_the_app_cannot_read_ends_voice_as_a_disagreement() {
    let mut session = listening();
    closed(
        &mut session,
        CloseReason::Framing(DecodeError::NotJson("eof".into())),
    );
    assert_eq!(
        session.status().label,
        "Voice stopped: this app and Fermix could not understand each other."
    );
}

#[test]
fn a_close_this_app_asked_for_changes_nothing() {
    let mut session = listening();
    assert!(closed(&mut session, CloseReason::Local).is_empty());
    assert!(session.in_call());
}

// The words

#[test]
fn every_mode_has_a_word_an_icon_and_a_palette_role() {
    let idle = ready();
    let mut thinking = listening();
    state(&mut thinking, TurnState::Thinking);
    let mut tool = listening();
    event(&mut tool, ServerEvent::Task(task(1, TaskStatus::Running)));
    let rows = [
        (
            Session::new(),
            "Not connected",
            "network-offline-symbolic",
            Palette::Faint,
        ),
        (
            calling(),
            "Connecting…",
            "content-loading-symbolic",
            Palette::Secondary,
        ),
        (idle, "Ready", "call-start-symbolic", Palette::Secondary),
        (
            listening(),
            "Listening",
            "audio-input-microphone-symbolic",
            Palette::Accent,
        ),
        (
            thinking,
            "Thinking",
            "emoji-objects-symbolic",
            Palette::Secondary,
        ),
        (
            tool,
            "Running a tool",
            "applications-engineering-symbolic",
            Palette::Secondary,
        ),
    ];
    for (session, label, icon, palette) in rows {
        let status = session.status();
        assert_eq!(
            (status.label.as_str(), status.icon, status.palette),
            (label, icon, palette)
        );
    }
    let mut speaking = listening();
    audio(&mut speaking, 0.5);
    assert_eq!(speaking.status().icon, "audio-volume-high-symbolic");
}

#[test]
fn the_reason_to_sentence_table() {
    let violations = [
        "handshake_required",
        "unexpected_client_hello",
        "invalid_json",
        "invalid_event",
        "missing_type",
        "{:unknown_event, \"x\"}",
        "missing_audio",
        "invalid_audio_base64",
        "{:chunk_too_large, 20000, 16384}",
        "invalid_audio_end_ms",
        "missing_delegation_id",
        "missing_protocol_version",
        "invalid_protocol_version",
        "line_too_large",
        "not_connected",
        "unsupported_by_engine",
        "unknown_playback",
        "playback_regressed",
        "playback_beyond_sent",
    ];
    for reason in violations {
        assert_eq!(
            error_sentence(&refusal(reason)),
            "Voice stopped: this app and Fermix could not understand each other.",
            "{reason}"
        );
    }
    let rows = [
        (
            "not_configured",
            "Add the voice provider's API key in Voice settings, or turn voice off. \
             A subscription sign-in does not cover voice; it needs an API key.",
        ),
        (
            "provider_send_failed: {:http, 401}",
            "The voice provider refused the call: {:http, 401}.",
        ),
        (
            "provider_disconnected",
            "The connection to the voice provider dropped.",
        ),
        ("call_in_progress", "Another voice call is already up."),
        (
            "playback_stalled",
            "Fermix stopped voice (playback_stalled).",
        ),
        (
            "max_session_duration",
            "This call reached the time limit set in Voice settings.",
        ),
        (
            "cost_limit",
            "This call reached the spending limit set in Voice settings.",
        ),
        ("provider_refused", "The voice provider refused the call."),
        ("{:session_down, :killed}", "Voice stopped unexpectedly."),
        ("voice_disabled", "Fermix stopped voice (voice_disabled)."),
    ];
    for (reason, sentence) in rows {
        assert_eq!(error_sentence(&refusal(reason)), sentence, "{reason}");
    }
}

#[test]
fn a_live_error_is_read_by_its_kind_and_shows_the_providers_detail() {
    let mut expired = refusal("session_expired");
    expired.kind = Some("session_expired".into());
    expired.detail = Some("Your session hit the maximum duration of 60 minutes.".into());
    assert_eq!(
        error_sentence(&expired),
        "Your session hit the maximum duration of 60 minutes."
    );

    let mut refused = refusal("provider_refused");
    refused.kind = Some("provider_refused".into());
    refused.detail = Some("Incorrect API key provided.".into());
    assert_eq!(error_sentence(&refused), "Incorrect API key provided.");

    let mut kind_only = refusal("something_new");
    kind_only.kind = Some("cost_limit".into());
    assert_eq!(
        error_sentence(&kind_only),
        "This call reached the spending limit set in Voice settings."
    );

    let unsure = refusal("unsupported_protocol_version");
    assert_eq!(
        error_sentence(&unsure),
        "This app and Fermix need matching versions to talk. Update the older one."
    );

    let mut update = refusal("unsupported_protocol_version");
    update.kind = Some("update_required".into());
    update.direction = Some(Direction::ClientTooOld);
    update.required_for = Some("openai_live".into());
    assert_eq!(
        error_sentence(&update),
        "Update this app to talk to this version of Fermix."
    );
}

/// A timer set during one call checks the number before it acts on a later one.
#[test]
fn each_call_has_its_own_number() {
    let mut session = calling();
    let first = session.call_number();
    session.apply(Input::End);
    assert_eq!(session.call_number(), first, "ending keeps the number");
    session.apply(Input::Begin);
    assert_ne!(session.call_number(), first);
}
