//! The realtime wire's types and framing, beyond the golden lines: the shapes the engine sends
//! that its schema does not list (spec R6), the open vocabularies, and the caps.

use fermix_client::realtime::protocol::{
    decode_line, ClientEvent, DecodeError, Direction, LineBuffer, LineTooLong, ServerEvent,
    Speaker, TaskStatus, ToolStatus, TurnState, MAX_FRAME_BYTES, MAX_LINE_BYTES,
    MAX_UNSCANNED_BYTES,
};

fn decode(line: &str) -> ServerEvent {
    decode_line(line.as_bytes()).unwrap_or_else(|e| panic!("{line}: {e:?}"))
}

#[test]
fn the_caps_are_the_macos_frame_limits_and_the_daemons_line_cap() {
    assert_eq!(MAX_FRAME_BYTES, 1_048_576);
    assert_eq!(MAX_UNSCANNED_BYTES, 2_097_152);
    assert_eq!(MAX_LINE_BYTES, 65_536);
}

#[test]
fn an_unknown_event_type_keeps_its_name_and_is_not_an_error() {
    assert_eq!(
        decode(r#"{"type":"weather","sky":"clear"}"#),
        ServerEvent::Unknown("weather".into())
    );
}

#[test]
fn unknown_fields_on_a_known_event_are_ignored() {
    assert_eq!(
        decode(r#"{"type":"playback_stop","why":"barge_in","n":3}"#),
        ServerEvent::PlaybackStop
    );
}

#[test]
fn a_frame_that_is_not_an_object_or_has_no_type_is_refused() {
    assert!(matches!(
        decode_line(b"{not json"),
        Err(DecodeError::NotJson(_))
    ));
    assert_eq!(decode_line(b"[1,2]"), Err(DecodeError::NotAnObject));
    assert_eq!(
        decode_line(br#"{"state":"idle"}"#),
        Err(DecodeError::MissingType)
    );
    assert_eq!(decode_line(br#"{"type":7}"#), Err(DecodeError::MissingType));
}

#[test]
fn a_known_event_missing_a_required_field_is_refused_by_name() {
    let Err(DecodeError::BadEvent { kind, .. }) = decode_line(br#"{"type":"server_hello"}"#) else {
        panic!("a server_hello without its window is not an event")
    };
    assert_eq!(kind, "server_hello");
}

#[test]
fn every_documented_turn_state_decodes_and_an_unknown_one_keeps_its_word() {
    let cases = [
        ("idle", TurnState::Idle),
        ("listening", TurnState::Listening),
        ("speaking", TurnState::Speaking),
        ("muted", TurnState::Muted),
        ("thinking", TurnState::Thinking),
        ("reconnecting", TurnState::Reconnecting),
        ("dreaming", TurnState::Other("dreaming".into())),
    ];
    for (word, state) in cases {
        let line = format!(r#"{{"type":"state","state":"{word}"}}"#);
        assert_eq!(decode(&line), ServerEvent::State { state });
    }
}

#[test]
fn the_daemons_running_tool_event_is_running_and_keeps_its_unlisted_name() {
    assert_eq!(
        decode(r#"{"type":"tool_event","status":"running","name":"calendar"}"#),
        ServerEvent::ToolEvent {
            status: ToolStatus::Running,
            name: Some("calendar".into()),
            reason: None
        }
    );
    assert_eq!(
        decode(r#"{"type":"tool_event","status":"error","name":"write","reason":"write_refused"}"#),
        ServerEvent::ToolEvent {
            status: ToolStatus::Error,
            name: Some("write".into()),
            reason: Some("write_refused".into())
        }
    );
    let ServerEvent::ToolEvent { status, .. } = decode(r#"{"type":"tool_event","status":"x"}"#)
    else {
        panic!("a tool_event")
    };
    assert_eq!(status, ToolStatus::Other("x".into()));
}

#[test]
fn a_tool_event_without_a_status_reads_as_running() {
    let ServerEvent::ToolEvent { status, .. } = decode(r#"{"type":"tool_event","name":"x"}"#)
    else {
        panic!("a tool_event")
    };
    assert_eq!(status, ToolStatus::Running);
}

#[test]
fn a_realtime_transcript_keeps_its_unlisted_role() {
    assert_eq!(
        decode(r#"{"type":"transcript_delta","role":"user","text":"what time is it"}"#),
        ServerEvent::TranscriptDelta {
            text: "what time is it".into(),
            role: Some("user".into())
        }
    );
}

#[test]
fn the_realtime_estimated_usage_shape_decodes() {
    let line = r#"{"type":"usage","status":"estimated","estimated":{"input_audio_ms":1200,"input_audio_tokens":0,"cost_cents":0.12,"transcription_ms":1200,"transcription_cost_cents":0.01},"reported":{"cost_cents":0.0}}"#;
    let ServerEvent::Usage(usage) = decode(line) else {
        panic!("usage")
    };
    assert_eq!(usage.status.as_deref(), Some("estimated"));
    let estimated = usage.estimated.expect("the estimate");
    assert_eq!(estimated.input_audio_ms, Some(1200));
    assert_eq!(estimated.input_audio_tokens, Some(0));
    assert_eq!(estimated.cost_cents, Some(0.12));
    assert_eq!(estimated.transcription_ms, Some(1200));
    assert_eq!(estimated.transcription_cost_cents, Some(0.01));
    assert_eq!(usage.reported.expect("the report").cost_cents, Some(0.0));
}

#[test]
fn the_realtime_reported_and_limit_usage_shapes_decode() {
    let ServerEvent::Usage(reported) =
        decode(r#"{"type":"usage","status":"reported","cost_cents":1.84}"#)
    else {
        panic!("usage")
    };
    assert_eq!(reported.cost_cents, Some(1.84));

    let line = r#"{"type":"usage","status":"limit_reached","reason":"cost_limit"}"#;
    let ServerEvent::Usage(limit) = decode(line) else {
        panic!("usage")
    };
    assert_eq!(limit.status.as_deref(), Some("limit_reached"));
    assert_eq!(limit.reason.as_deref(), Some("cost_limit"));
}

#[test]
fn the_live_usage_shape_carries_the_unknown_backend_cost_as_a_word() {
    let line = r#"{"type":"usage","status":"live","voice_seconds":64.2,"voice_cost_cents":5.35,"backend_turns":2,"backend_cost":"unknown","accounting":"running"}"#;
    let ServerEvent::Usage(usage) = decode(line) else {
        panic!("usage")
    };
    assert_eq!(usage.voice_seconds, Some(64.2));
    assert_eq!(usage.voice_cost_cents, Some(5.35));
    assert_eq!(usage.backend_turns, Some(2));
    assert_eq!(usage.backend_cost.as_deref(), Some("unknown"));
    assert_eq!(usage.accounting.as_deref(), Some("running"));
}

#[test]
fn an_error_carries_its_kind_detail_and_version_window() {
    let line = r#"{"type":"error","reason":"unsupported_protocol_version","direction":"client_too_new","client_version":99,"min_version":1,"max_version":2}"#;
    let ServerEvent::Error(error) = decode(line) else {
        panic!("error")
    };
    assert_eq!(error.reason, "unsupported_protocol_version");
    assert_eq!(error.kind, None);
    assert_eq!(error.direction, Some(Direction::ClientTooNew));
    assert_eq!(error.client_version, Some(99));
    assert_eq!((error.min_version, error.max_version), (Some(1), Some(2)));

    let line = r#"{"type":"error","reason":"provider_refused","kind":"provider_refused","detail":"Your key is invalid."}"#;
    let ServerEvent::Error(error) = decode(line) else {
        panic!("error")
    };
    assert_eq!(error.kind.as_deref(), Some("provider_refused"));
    assert_eq!(error.detail.as_deref(), Some("Your key is invalid."));
}

#[test]
fn an_error_with_a_direction_this_build_has_never_seen_still_decodes() {
    let line = r#"{"type":"error","reason":"unsupported_protocol_version","direction":"sideways"}"#;
    let ServerEvent::Error(error) = decode(line) else {
        panic!("error")
    };
    assert_eq!(error.direction, Some(Direction::Other("sideways".into())));
}

#[test]
fn audio_delta_is_decoded_from_base64_and_bad_base64_is_refused() {
    assert_eq!(
        decode(r#"{"type":"audio_delta","audio":"AQACAA=="}"#),
        ServerEvent::AudioDelta {
            audio: vec![1, 0, 2, 0]
        }
    );
    assert!(matches!(
        decode_line(br#"{"type":"audio_delta","audio":"not base64!"}"#),
        Err(DecodeError::BadEvent { .. })
    ));
}

#[test]
fn a_caption_from_an_unknown_speaker_keeps_its_word_and_its_bytes() {
    let line = r#"{"type":"caption","speaker":"narrator","delta":"  so ","start_ms":0,"end_ms":5}"#;
    let ServerEvent::Caption(caption) = decode(line) else {
        panic!("caption")
    };
    assert_eq!(caption.speaker, Speaker::Other("narrator".into()));
    assert_eq!(caption.delta, "  so ");
}

#[test]
fn only_completed_failed_and_cancelled_tasks_are_terminal() {
    assert!(TaskStatus::Completed.is_terminal());
    assert!(TaskStatus::Failed.is_terminal());
    assert!(TaskStatus::Cancelled.is_terminal());
    assert!(!TaskStatus::Pending.is_terminal());
    assert!(!TaskStatus::Running.is_terminal());
    assert!(!TaskStatus::Other("paused".into()).is_terminal());
    let ServerEvent::Task(task) =
        decode(r#"{"type":"task","delegation_id":"dg_1","revision":2,"status":"paused"}"#)
    else {
        panic!("task")
    };
    assert_eq!(task.status, TaskStatus::Other("paused".into()));
    assert_eq!(task.summary, None);
}

#[test]
fn a_call_ready_without_a_provider_session_or_expiry_decodes() {
    let line =
        r#"{"type":"call_ready","engine":"openai_realtime","call_id":"c1","captions":false}"#;
    let ServerEvent::CallReady(ready) = decode(line) else {
        panic!("call_ready")
    };
    assert_eq!(ready.provider_session_id, None);
    assert_eq!(ready.expires_at, None);
    assert!(!ready.captions);
}

#[test]
fn client_events_are_written_as_one_json_line_each() {
    let hello = ClientEvent::ClientHello {
        protocol_version: 2,
    };
    assert_eq!(
        hello.line().unwrap(),
        "{\"type\":\"client_hello\",\"protocol_version\":2}\n"
    );
    let stop = ClientEvent::Interrupt {
        audio_end_ms: Some(1500),
    };
    assert_eq!(
        stop.line().unwrap(),
        "{\"type\":\"interrupt\",\"audio_end_ms\":1500}\n"
    );
    let bare = ClientEvent::Interrupt { audio_end_ms: None };
    assert_eq!(bare.line().unwrap(), "{\"type\":\"interrupt\"}\n");
    assert_eq!(
        ClientEvent::CallStop.line().unwrap(),
        "{\"type\":\"call_stop\"}\n"
    );
}

#[test]
fn an_audio_chunk_carries_its_pcm_as_base64() {
    assert_eq!(
        ClientEvent::audio_chunk(b"1234"),
        ClientEvent::AudioChunk {
            audio: "MTIzNA==".into()
        }
    );
}

#[test]
fn a_line_the_daemon_would_refuse_is_never_written() {
    let long = ClientEvent::TaskCancel {
        delegation_id: "d".repeat(MAX_LINE_BYTES),
    };
    let Err(LineTooLong(bytes)) = long.line() else {
        panic!("a line over the daemon's cap must be refused")
    };
    assert!(bytes >= MAX_LINE_BYTES);

    // The largest legal chunk, 16 384 decoded bytes, fits with room to spare.
    let chunk = ClientEvent::audio_chunk(&vec![0u8; 16_384]);
    assert!(chunk.line().unwrap().len() < MAX_LINE_BYTES);
}

#[test]
fn complete_lines_come_out_in_order_and_empty_lines_are_skipped() {
    let mut buffer = LineBuffer::new();
    let lines = buffer.push(b"{\"a\":1}\n\n{\"b\":2}\n").unwrap();
    assert_eq!(lines, vec![b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec()]);
}

#[test]
fn a_partial_line_is_held_until_its_newline_arrives() {
    let mut buffer = LineBuffer::new();
    assert!(buffer.push(b"{\"type\":").unwrap().is_empty());
    assert!(buffer.push(b"\"call").unwrap().is_empty());
    let lines = buffer.push(b"_stop\"}\n{\"x\"").unwrap();
    assert_eq!(lines, vec![b"{\"type\":\"call_stop\"}".to_vec()]);
    assert_eq!(buffer.push(b":1}\n").unwrap(), vec![b"{\"x\":1}".to_vec()]);
}

#[test]
fn a_line_at_the_frame_limit_passes_and_one_byte_more_is_refused() {
    let mut at_limit = vec![b'x'; MAX_FRAME_BYTES];
    at_limit.push(b'\n');
    assert_eq!(LineBuffer::new().push(&at_limit).unwrap().len(), 1);

    let mut over = vec![b'x'; MAX_FRAME_BYTES + 1];
    over.push(b'\n');
    assert_eq!(
        LineBuffer::new().push(&over),
        Err(DecodeError::FrameTooLarge(MAX_FRAME_BYTES + 1))
    );
}

#[test]
fn an_unterminated_run_is_refused_once_it_passes_the_frame_limit() {
    let mut buffer = LineBuffer::new();
    assert!(buffer
        .push(&vec![b'x'; MAX_FRAME_BYTES])
        .unwrap()
        .is_empty());
    assert_eq!(
        buffer.push(b"y"),
        Err(DecodeError::FrameTooLarge(MAX_FRAME_BYTES + 1))
    );
}

#[test]
fn a_burst_larger_than_the_unscanned_limit_is_refused_before_scanning() {
    let mut buffer = LineBuffer::new();
    let burst = vec![b'\n'; MAX_UNSCANNED_BYTES + 1];
    assert_eq!(
        buffer.push(&burst),
        Err(DecodeError::BufferTooLarge(MAX_UNSCANNED_BYTES + 1))
    );
}
