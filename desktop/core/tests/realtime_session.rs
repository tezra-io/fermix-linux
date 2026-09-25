//! The voice reducer: what each input does to the model and which effects it asks for. Ported
//! from the macOS `AppModelTests` voice routing and presentation suites, plus the Linux rules of
//! spec §1.4 and §1.6 (Reconnecting…, the 12 s call deadline, the tool `running` word, the
//! gate-then-`call_stop` order, the reason → sentence table).

use fermix_client::realtime::client::{CloseReason, ConnectError, Incoming};
use fermix_client::realtime::protocol::{
    CallReady, Caption, ClientEvent, DecodeError, Direction, ServerError, ServerEvent, Speaker,
    Task, TaskStatus, ToolStatus, TurnState, Usage,
};
use fermix_client::realtime::session::{
    error_sentence, Effect, Input, Mode, Palette, Session, CALL_START_DEADLINE,
};
use std::time::Duration;

fn event(session: &mut Session, event: ServerEvent) -> Vec<Effect> {
    session.apply(Input::Wire(Incoming::Event(event)))
}

fn state(session: &mut Session, state: TurnState) -> Vec<Effect> {
    event(session, ServerEvent::State { state })
}

fn audio(session: &mut Session, rms: f32) -> Vec<Effect> {
    session.apply(Input::Wire(Incoming::Audio { bytes: 4_800, rms }))
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
            Effect::FlushPlayback,
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
fn playback_stop_flushes_clears_the_tail_and_returns_to_the_microphone() {
    let mut session = listening();
    audio(&mut session, 0.5);
    let effects = event(&mut session, ServerEvent::PlaybackStop);
    assert_eq!(effects, vec![Effect::FlushPlayback]);
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

// Call facts: call_ready, captions, usage

#[test]
fn call_ready_records_the_engine_and_call_without_moving_the_mode() {
    let mut session = listening();
    assert!(event(&mut session, live_ready()).is_empty());
    assert_eq!(session.engine(), Some("openai_live"));
    assert_eq!(session.call_id(), Some("voice_live:17"));
    assert_eq!(session.mode(), Mode::Listening);
}

#[test]
fn the_last_caption_is_kept_verbatim_with_who_said_it() {
    let mut session = listening();
    assert_eq!(session.caption_line(), None);
    let user = Caption {
        speaker: Speaker::User,
        delta: "what is ".into(),
        start_ms: 0,
        end_ms: 440,
    };
    assert!(event(&mut session, ServerEvent::Caption(user)).is_empty());
    assert_eq!(session.caption_line().as_deref(), Some("You: what is "));

    let fermix = Caption {
        speaker: Speaker::Assistant,
        delta: "the ".into(),
        start_ms: 300,
        end_ms: 520,
    };
    event(&mut session, ServerEvent::Caption(fermix));
    assert_eq!(session.caption_line().as_deref(), Some("Fermix: the "));

    let other = Caption {
        speaker: Speaker::Other("narrator".into()),
        delta: "so".into(),
        start_ms: 0,
        end_ms: 1,
    };
    event(&mut session, ServerEvent::Caption(other));
    assert_eq!(session.caption_line().as_deref(), Some("narrator: so"));
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
    let caption = Caption {
        speaker: Speaker::User,
        delta: "what is ".into(),
        start_ms: 0,
        end_ms: 1,
    };
    event(&mut session, ServerEvent::Caption(caption));
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
    assert_eq!(session.caption_line(), None);
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
fn transcripts_unknown_events_and_a_second_hello_change_nothing() {
    let quiet = [
        ServerEvent::TranscriptDelta {
            text: "hello".into(),
            role: Some("user".into()),
        },
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

/// `played_ms` is read before the flush: clearing the queue resets its anchor.
#[test]
fn stop_flushes_locally_then_sends_interrupt_with_what_was_played() {
    let mut session = listening();
    audio(&mut session, 0.5);
    let effects = session.apply(Input::Stop { played_ms: 1_500 });
    assert_eq!(
        effects,
        vec![
            Effect::FlushPlayback,
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
    let sentence = "No microphone is available.";
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
            "Add the OpenAI API key in Providers settings, or turn voice off. \
             A ChatGPT sign-in does not cover voice; it needs an OpenAI API key.",
        ),
        (
            "provider_send_failed: {:http, 401}",
            "OpenAI refused the voice call: {:http, 401}.",
        ),
        ("provider_disconnected", "The connection to OpenAI dropped."),
        (
            "max_session_duration",
            "This call reached the time limit set in Voice settings.",
        ),
        (
            "cost_limit",
            "This call reached the spending limit set in Voice settings.",
        ),
        ("provider_refused", "OpenAI refused the voice call."),
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
