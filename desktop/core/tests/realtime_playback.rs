//! The playback queue between the socket's reader thread and GStreamer's streaming thread,
//! and the level, voice and timing maths beside it.

use fermix_client::realtime::playback::{
    played_ms, rms, smooth, voiced, PlaybackQueue, Pull, MAX_QUEUED_SAMPLES, SAMPLE_RATE,
    VOICED_RMS,
};
use fermix_client::realtime::protocol::{decode_line, PlaybackProgress, ServerEvent};

const SERVER: &str = include_str!("../contracts/realtime/fixtures/server_events.jsonl");
/// 100 ms at 24 kHz: one Live chunk.
const CHUNK: usize = 2_400;
/// 20 ms: one pull of the pipeline's.
const BLOCK: usize = 480;

fn pcm(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// `samples` of a steady `level`: its int16 RMS is `level`.
fn steady(level: i16, samples: usize) -> Vec<u8> {
    pcm(&vec![level; samples])
}

/// 100 ms of voice, at int16 RMS 2,000.
fn voice() -> Vec<u8> {
    steady(2_000, CHUNK)
}

/// 100 ms of Live's padding, digital silence.
fn padding() -> Vec<u8> {
    steady(0, CHUNK)
}

fn pull(queue: &mut PlaybackQueue, samples: usize) -> Pull {
    queue.pull(&mut vec![0; samples])
}

#[test]
fn the_wire_format_is_24_khz_and_the_queue_holds_30_seconds() {
    assert_eq!(SAMPLE_RATE, 24_000);
    assert_eq!(MAX_QUEUED_SAMPLES, 30 * 24_000);
}

#[test]
fn samples_come_out_in_order_and_little_endian() {
    let mut queue = PlaybackQueue::new();
    assert_eq!(queue.push_pcm16(&pcm(&[1, -2, 300]), None), 0);
    let mut out = [9i16; 3];
    let pull = queue.pull(&mut out);
    assert_eq!(out, [1, -2, 300]);
    assert_eq!(pull.real, 3);
}

#[test]
fn a_pull_past_the_end_is_filled_with_silence() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[5, 6]), None);
    let mut out = [9i16; 5];
    let pull = queue.pull(&mut out);
    assert_eq!(out, [5, 6, 0, 0, 0]);
    assert_eq!(pull.real, 2);

    let mut again = [9i16; 4];
    let idle = queue.pull(&mut again);
    assert_eq!(again, [0; 4]);
    assert_eq!(idle.real, 0);
}

#[test]
fn emptied_is_the_edge_from_audio_to_none_and_fires_once() {
    let mut queue = PlaybackQueue::new();
    let mut out = [0i16; 2];
    assert!(
        !queue.pull(&mut out).emptied,
        "an empty queue never empties"
    );

    queue.push_pcm16(&pcm(&[1, 2, 3]), None);
    assert!(!queue.pull(&mut out).emptied, "one sample is still queued");
    assert!(queue.pull(&mut out).emptied, "the last sample went out");
    assert!(!queue.pull(&mut out).emptied, "already empty");
    assert!(queue.is_empty());
}

#[test]
fn drained_is_the_edge_from_voice_to_none_and_fires_once() {
    let mut queue = PlaybackQueue::new();
    let mut out = [0i16; 2];
    assert!(!queue.pull(&mut out).drained, "an empty queue never drains");

    queue.push_pcm16(&pcm(&[1_000, -1_000, 1_000]), None);
    assert!(queue.voice_queued());
    assert!(
        !queue.pull(&mut out).drained,
        "one voiced sample is still queued"
    );
    assert!(
        queue.pull(&mut out).drained,
        "the last voiced sample went out"
    );
    assert!(!queue.pull(&mut out).drained, "already drained");
    assert!(!queue.voice_queued());
}

#[test]
fn a_pull_that_empties_the_queue_exactly_is_drained() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[1_000, -1_000]), None);
    let mut out = [0i16; 2];
    let pull = queue.pull(&mut out);
    assert_eq!(pull.real, 2);
    assert!(pull.drained);
    assert!(pull.emptied);
}

#[test]
fn at_capacity_the_oldest_samples_drop_and_the_count_is_returned() {
    let mut queue = PlaybackQueue::new();
    let full: Vec<i16> = (0..MAX_QUEUED_SAMPLES).map(|i| (i % 1000) as i16).collect();
    assert_eq!(queue.push_pcm16(&pcm(&full), None), 0);
    assert_eq!(queue.push_pcm16(&pcm(&[-1, -2, -3]), None), 3);

    let mut first = [0i16; 2];
    queue.pull(&mut first);
    assert_eq!(first, [3, 4], "samples 0, 1 and 2 were dropped");

    let mut rest = vec![0i16; MAX_QUEUED_SAMPLES];
    let pull = queue.pull(&mut rest);
    assert_eq!(pull.real, MAX_QUEUED_SAMPLES - 2);
    assert_eq!(&rest[pull.real - 3..pull.real], &[-1, -2, -3]);
}

#[test]
fn one_push_larger_than_the_queue_keeps_its_newest_samples() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[7, 7]), None);
    let huge: Vec<i16> = (0..MAX_QUEUED_SAMPLES + 10)
        .map(|i| (i % 30_000) as i16)
        .collect();
    assert_eq!(queue.push_pcm16(&pcm(&huge), None), 12);
    let mut first = [0i16; 1];
    queue.pull(&mut first);
    assert_eq!(first, [10]);
}

#[test]
fn a_trailing_odd_byte_is_not_a_sample() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&[1, 0, 2], None);
    let mut out = [9i16; 2];
    assert_eq!(queue.pull(&mut out).real, 1);
    assert_eq!(out, [1, 0]);
}

#[test]
fn the_anchor_counts_real_samples_only_and_resets() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[1; 100]), None);
    let mut out = [0i16; 60];
    queue.pull(&mut out);
    queue.pull(&mut out);
    assert_eq!(queue.played_since_anchor(), 100, "silence is not counted");

    queue.reset_anchor();
    assert_eq!(queue.played_since_anchor(), 0);
    queue.push_pcm16(&pcm(&[1; 10]), None);
    queue.pull(&mut out);
    assert_eq!(queue.played_since_anchor(), 10);
}

/// Realtime says `listening` at `response.done`, while the reply's tail is still queued. The
/// anchor resets then, so a Stop early in the next reply must not count that tail: OpenAI
/// refuses a truncate past the item's own audio.
#[test]
fn the_tail_still_queued_at_the_anchor_belongs_to_the_reply_before_it() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[1; 100]), None);
    let mut out = [0i16; 40];
    queue.pull(&mut out);
    queue.reset_anchor();
    queue.push_pcm16(&pcm(&[2; 30]), None);
    queue.pull(&mut out);
    assert_eq!(queue.played_since_anchor(), 0, "40 of the 60-sample tail");
    queue.pull(&mut out);
    assert_eq!(
        queue.played_since_anchor(),
        20,
        "the tail's last 20, then 20 new"
    );
    queue.pull(&mut out);
    assert_eq!(queue.played_since_anchor(), 30);
}

#[test]
fn samples_dropped_at_capacity_come_off_the_tail_first() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&vec![1; MAX_QUEUED_SAMPLES]), None);
    queue.reset_anchor();
    assert_eq!(queue.push_pcm16(&pcm(&[2; 3]), None), 3);
    let mut tail = vec![0i16; MAX_QUEUED_SAMPLES - 3];
    queue.pull(&mut tail);
    assert_eq!(queue.played_since_anchor(), 0);
    let mut next = [0i16; 3];
    queue.pull(&mut next);
    assert_eq!(next, [2; 3]);
    assert_eq!(queue.played_since_anchor(), 3);
}

#[test]
fn clear_empties_the_queue_and_resets_the_anchor() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&steady(1_000, 50), None);
    let mut out = [0i16; 20];
    queue.pull(&mut out);
    queue.clear();
    assert!(queue.is_empty());
    assert!(!queue.voice_queued());
    assert_eq!(queue.played_since_anchor(), 0);
    let pull = queue.pull(&mut out);
    assert_eq!(pull.real, 0);
    assert!(!pull.drained, "a flush is not a drain");
    assert!(!pull.emptied, "nor the queue running dry");
}

#[test]
fn played_ms_is_samples_over_24_less_the_sink_latency_and_never_negative() {
    assert_eq!(played_ms(24_000, 0), 1_000);
    assert_eq!(played_ms(36_000, 100), 1_400);
    assert_eq!(played_ms(2_400, 250), 0);
    assert_eq!(played_ms(0, 0), 0);
    assert_eq!(played_ms(u64::MAX, 0), u64::MAX / 24);
}

#[test]
fn rms_is_zero_for_silence_and_one_for_full_scale() {
    assert_eq!(rms(&[]), 0.0);
    assert_eq!(rms(&pcm(&[0; 480])), 0.0);
    assert!((rms(&pcm(&[i16::MAX; 480])) - 1.0).abs() < 1e-6);
    assert!(rms(&pcm(&[i16::MIN; 480])) <= 1.0, "clamped at full scale");
    let half = rms(&pcm(&[16_384, -16_384, 16_384, -16_384]));
    assert!((half - 0.5).abs() < 1e-3, "{half}");
}

#[test]
fn smooth_moves_35_percent_of_the_way_to_the_new_level() {
    assert!((smooth(0.0, 1.0) - 0.35).abs() < 1e-6);
    assert!((smooth(1.0, 0.0) - 0.65).abs() < 1e-6);
    assert!((smooth(0.4, 0.4) - 0.4).abs() < 1e-6);
}

fn progress(id: &str, played_ms: u64, drained: bool) -> PlaybackProgress {
    PlaybackProgress {
        playback_id: id.into(),
        played_ms,
        drained,
    }
}

/// 24 samples a millisecond, so `ms(n)` is n milliseconds of audio.
fn ms(n: usize) -> Vec<u8> {
    pcm(&vec![1; n * 24])
}

#[test]
fn an_id_reports_what_was_pulled_and_drains_when_all_of_it_was() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&ms(30), Some("p1"));
    assert_eq!(queue.take_progress(), vec![progress("p1", 0, false)]);
    assert_eq!(
        queue.take_progress(),
        vec![],
        "nothing changed, nothing said"
    );

    queue.pull(&mut [0; 20 * 24]);
    assert_eq!(queue.take_progress(), vec![progress("p1", 20, false)]);
    queue.pull(&mut [0; 20 * 24]);
    assert_eq!(queue.take_progress(), vec![progress("p1", 30, true)]);

    queue.push_pcm16(&ms(5), Some("p1"));
    assert_eq!(
        queue.take_progress(),
        vec![progress("p1", 30, false)],
        "more audio under a drained id undrains it"
    );
}

#[test]
fn ids_report_in_the_order_their_audio_began() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&ms(10), Some("p1"));
    queue.push_pcm16(&ms(10), Some("p2"));
    queue.pull(&mut [0; 15 * 24]);
    assert_eq!(
        queue.take_progress(),
        vec![progress("p1", 10, true), progress("p2", 5, false)]
    );
}

#[test]
fn a_stop_keeps_what_played_and_the_id_never_drains_again() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&ms(40), Some("p1"));
    queue.pull(&mut [0; 10 * 24]);
    queue.clear();
    assert_eq!(queue.take_progress(), vec![progress("p1", 10, false)]);
    queue.pull(&mut [0; 10 * 24]);
    assert_eq!(queue.take_progress(), vec![], "a flush is not played");
}

#[test]
fn audio_without_an_id_is_never_reported_and_holds_its_place_in_the_queue() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&ms(10), None);
    queue.push_pcm16(&ms(10), Some("p1"));
    queue.pull(&mut [0; 10 * 24]);
    assert_eq!(queue.take_progress(), vec![progress("p1", 0, false)]);
    queue.pull(&mut [0; 10 * 24]);
    assert_eq!(queue.take_progress(), vec![progress("p1", 10, true)]);
}

#[test]
fn audio_dropped_past_the_bound_counts_as_played() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&vec![1; MAX_QUEUED_SAMPLES]), Some("p1"));
    queue.push_pcm16(&ms(10), Some("p2"));
    assert_eq!(
        queue.take_progress(),
        vec![progress("p1", 10, false), progress("p2", 0, false)]
    );
}

#[test]
fn a_new_call_forgets_the_last_calls_ids() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&ms(10), Some("p1"));
    queue.pull(&mut [0; 10 * 24]);
    queue.take_progress();
    queue.start_call();
    queue.push_pcm16(&ms(10), Some("p1"));
    assert_eq!(
        queue.take_progress(),
        vec![progress("p1", 0, false)],
        "a reused id starts at nothing played"
    );
}

// Voice and Live's padding

/// The engine's line and the macOS app's: int16 RMS 100, exactly, on whole samples.
#[test]
fn a_chunk_is_voice_from_int16_rms_100() {
    assert_eq!(VOICED_RMS, 100);
    assert!(!voiced(&steady(99, CHUNK)));
    assert!(voiced(&steady(100, CHUNK)));
    assert!(voiced(&steady(101, CHUNK)));
    assert!(voiced(&steady(-100, CHUNK)), "the sign does not matter");
    assert!(voiced(&pcm(&[100, -100, 100, -100])));
    // Mean squares of 9,900.5 and 10,001: RMS 99.5 and 100.005.
    assert!(!voiced(&pcm(&[100, 99])));
    assert!(voiced(&pcm(&[101, 99])));
    assert!(voiced(&pcm(&[i16::MIN; 4])), "full scale");
}

#[test]
fn a_chunk_with_no_whole_sample_is_not_voice_and_an_odd_byte_is_ignored() {
    assert!(!voiced(&[]));
    assert!(!voiced(&[0x7f]));
    let mut loud = steady(100, 3);
    loud.push(0x00);
    assert!(voiced(&loud), "100, 100, 100 and an odd byte");
    let mut quiet = steady(99, 3);
    quiet.push(0x7f);
    assert!(!voiced(&quiet), "the odd byte is not a loud sample");
}

/// The contract's own `audio_delta` is digital silence, Live's padding; Live's padding measured
/// up to 49 and its quietest voice 244.
#[test]
fn the_contracts_padding_and_lives_measured_padding_are_not_voice() {
    let line = SERVER.lines().nth(2).expect("the fixture's audio_delta");
    let ServerEvent::AudioDelta { audio, playback_id } =
        decode_line(line.as_bytes()).expect("a golden server line decodes")
    else {
        panic!("line 3 is an audio_delta: {line}");
    };
    assert_eq!(playback_id, None, "an OpenAI call's audio");
    assert!(!voiced(&audio));
    assert!(!voiced(&padding()));
    assert!(!voiced(&steady(49, CHUNK)));
    assert!(voiced(&steady(244, CHUNK)));
}

/// Live pads its output: the reply has played when its last voiced chunk has, with the padding
/// still queued behind it.
#[test]
fn the_drain_lands_at_the_end_of_the_last_voiced_chunk_with_padding_behind_it() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&voice(), None);
    queue.push_pcm16(&padding(), None);
    queue.push_pcm16(&voice(), None);
    queue.push_pcm16(&padding(), None);
    queue.push_pcm16(&padding(), None);
    for block in 1..15 {
        let pull = pull(&mut queue, BLOCK);
        assert!(
            !pull.drained,
            "block {block}: the second chunk of voice is still queued"
        );
    }
    assert!(queue.voice_queued());
    let at_end = pull(&mut queue, BLOCK);
    assert!(at_end.drained, "the last voiced sample went out");
    assert!(!at_end.emptied);
    assert!(!queue.voice_queued());
    assert_eq!(queue.played_since_anchor() as usize, 3 * CHUNK);
    for _ in 0..10 {
        let pull = pull(&mut queue, BLOCK);
        assert!(!pull.drained, "padding drains no voice");
    }
    assert!(queue.is_empty());
}

/// A drain lands on the pull that takes the voice's last sample, inside the block.
#[test]
fn the_drain_lands_inside_the_pull_that_takes_the_last_voiced_sample() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&steady(2_000, 1_000), None);
    queue.push_pcm16(&padding(), None);
    assert!(!pull(&mut queue, 999).drained);
    let pull = pull(&mut queue, BLOCK);
    assert!(pull.drained);
    assert_eq!(pull.real, BLOCK, "the block also carried padding");
}

#[test]
fn padding_alone_never_drains_and_voice_after_it_drains_at_its_own_end() {
    let mut queue = PlaybackQueue::new();
    for _ in 0..3 {
        queue.push_pcm16(&padding(), None);
        assert!(!queue.voice_queued());
        for _ in 0..5 {
            assert!(!pull(&mut queue, BLOCK).drained);
        }
    }
    queue.push_pcm16(&padding(), None);
    queue.push_pcm16(&voice(), None);
    queue.push_pcm16(&padding(), None);
    assert!(!pull(&mut queue, CHUNK).drained, "the padding before it");
    assert!(!pull(&mut queue, CHUNK - 1).drained);
    assert!(pull(&mut queue, 1).drained);
    assert!(!pull(&mut queue, CHUNK).drained);
}

/// At the bound the oldest samples drop, voice or not: the voice's end moves with them, and
/// voice dropped whole counts as played, so the next pull reports its drain.
#[test]
fn a_drop_at_the_bound_moves_the_voices_end_and_voice_dropped_whole_still_drains() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&voice(), None);
    queue.push_pcm16(&steady(0, MAX_QUEUED_SAMPLES - CHUNK), None);
    assert_eq!(queue.push_pcm16(&steady(0, 1_000), None), 1_000);
    assert!(!pull(&mut queue, CHUNK - 1_000 - 1).drained);
    assert!(
        pull(&mut queue, 1).drained,
        "1,000 of its 2,400 samples dropped"
    );

    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&voice(), None);
    assert_eq!(
        queue.push_pcm16(&steady(0, MAX_QUEUED_SAMPLES), None),
        CHUNK
    );
    assert!(!queue.voice_queued(), "dropped whole");
    let pull_after = pull(&mut queue, BLOCK);
    assert!(pull_after.drained, "the dropped voice counts as played");
    assert!(!pull(&mut queue, BLOCK).drained, "once");
}

#[test]
fn a_flush_is_not_a_drain_and_forgets_the_voice() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&voice(), None);
    queue.push_pcm16(&padding(), None);
    pull(&mut queue, BLOCK);
    queue.clear();
    assert!(!queue.voice_queued());
    queue.push_pcm16(&padding(), None);
    for _ in 0..6 {
        assert!(
            !pull(&mut queue, BLOCK).drained,
            "the flushed voice never drains"
        );
    }

    queue.push_pcm16(&voice(), None);
    queue.start_call();
    queue.push_pcm16(&padding(), None);
    for _ in 0..6 {
        assert!(
            !pull(&mut queue, BLOCK).drained,
            "nor a call's voice in the next call"
        );
    }
}

/// The anchor is about what `interrupt` reports; the voice's end does not move with it.
#[test]
fn a_new_anchor_keeps_the_voices_end() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&voice(), None);
    queue.push_pcm16(&padding(), None);
    pull(&mut queue, BLOCK);
    queue.reset_anchor();
    assert!(!pull(&mut queue, CHUNK - BLOCK - 1).drained);
    assert!(pull(&mut queue, 1).drained);
}

/// Padding plays: it counts as played since the anchor, and under its playback id, as all
/// audio does.
#[test]
fn padding_counts_as_played_as_any_audio_does() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&steady(2_000, 480), Some("p1"));
    queue.push_pcm16(&steady(0, 480), Some("p1"));
    pull(&mut queue, 960);
    assert_eq!(queue.played_since_anchor(), 960);
    assert_eq!(queue.take_progress(), vec![progress("p1", 40, true)]);
}

/// The Realtime and Grok engines send no padding: a reply whose every chunk is voice drains as
/// its queue runs dry, on the same pull, as before Live's padding was told apart.
#[test]
fn a_reply_without_padding_drains_as_its_queue_runs_dry_as_before() {
    let mut queue = PlaybackQueue::new();
    for level in [3_667, 244, 1_500] {
        queue.push_pcm16(&steady(level, 1_200), None);
    }
    let mut pulls = Vec::new();
    while !queue.is_empty() {
        assert_eq!(queue.voice_queued(), !queue.is_empty());
        pulls.push(pull(&mut queue, BLOCK));
    }
    let edges: Vec<(bool, bool)> = pulls.iter().map(|p| (p.drained, p.emptied)).collect();
    let mut expected = vec![(false, false); 7];
    expected.push((true, true));
    assert_eq!(edges, expected, "3,600 samples in 480-sample pulls");
}

/// The one change for engines that send no padding, and on purpose: a reply whose last chunk is
/// under the voiced line drains at its last voiced chunk, not at the queue's end. That quiet
/// chunk still plays and still counts under its playback id, but the reply's voice is over, so
/// nothing waits on it. The macOS app does the same: `AudioController` counts only voiced
/// buffers into `pendingVoiceBuffers`, and reports the drain when the last of them has played.
#[test]
fn a_reply_without_padding_drains_at_its_last_voiced_chunk_before_a_quiet_last_chunk() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&steady(1_500, 1_200), Some("p1"));
    queue.push_pcm16(&steady(1_500, 1_200), Some("p1"));
    queue.push_pcm16(&steady(60, 480), Some("p1")); // int16 RMS 60: under the line
    let edges: Vec<(bool, bool)> = (0..6)
        .map(|_| pull(&mut queue, BLOCK))
        .map(|p| (p.drained, p.emptied))
        .collect();
    let mut expected = vec![(false, false); 4];
    expected.extend([(true, false), (false, true)]);
    assert_eq!(
        edges, expected,
        "the drain at 2,400 samples, the queue's end at 2,880"
    );
    assert_eq!(queue.take_progress(), vec![progress("p1", 120, true)]);
}
