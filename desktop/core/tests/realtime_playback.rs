//! The playback queue between the socket's reader thread and GStreamer's streaming thread,
//! and the level and timing maths beside it.

use fermix_client::realtime::playback::{
    played_ms, rms, smooth, PlaybackQueue, MAX_QUEUED_SAMPLES, SAMPLE_RATE,
};

fn pcm(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

#[test]
fn the_wire_format_is_24_khz_and_the_queue_holds_30_seconds() {
    assert_eq!(SAMPLE_RATE, 24_000);
    assert_eq!(MAX_QUEUED_SAMPLES, 30 * 24_000);
}

#[test]
fn samples_come_out_in_order_and_little_endian() {
    let mut queue = PlaybackQueue::new();
    assert_eq!(queue.push_pcm16(&pcm(&[1, -2, 300])), 0);
    let mut out = [9i16; 3];
    let pull = queue.pull(&mut out);
    assert_eq!(out, [1, -2, 300]);
    assert_eq!(pull.real, 3);
}

#[test]
fn a_pull_past_the_end_is_filled_with_silence() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[5, 6]));
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
fn drained_is_the_edge_from_audio_to_none_and_fires_once() {
    let mut queue = PlaybackQueue::new();
    let mut out = [0i16; 2];
    assert!(!queue.pull(&mut out).drained, "an empty queue never drains");

    queue.push_pcm16(&pcm(&[1, 2, 3]));
    assert!(!queue.pull(&mut out).drained, "one sample is still queued");
    assert!(queue.pull(&mut out).drained, "the last sample went out");
    assert!(!queue.pull(&mut out).drained, "already empty");
    assert!(queue.is_empty());
}

#[test]
fn a_pull_that_empties_the_queue_exactly_is_drained() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[1, 2]));
    let mut out = [0i16; 2];
    let pull = queue.pull(&mut out);
    assert_eq!(pull.real, 2);
    assert!(pull.drained);
}

#[test]
fn at_capacity_the_oldest_samples_drop_and_the_count_is_returned() {
    let mut queue = PlaybackQueue::new();
    let full: Vec<i16> = (0..MAX_QUEUED_SAMPLES).map(|i| (i % 1000) as i16).collect();
    assert_eq!(queue.push_pcm16(&pcm(&full)), 0);
    assert_eq!(queue.push_pcm16(&pcm(&[-1, -2, -3])), 3);

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
    queue.push_pcm16(&pcm(&[7, 7]));
    let huge: Vec<i16> = (0..MAX_QUEUED_SAMPLES + 10)
        .map(|i| (i % 30_000) as i16)
        .collect();
    assert_eq!(queue.push_pcm16(&pcm(&huge)), 12);
    let mut first = [0i16; 1];
    queue.pull(&mut first);
    assert_eq!(first, [10]);
}

#[test]
fn a_trailing_odd_byte_is_not_a_sample() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&[1, 0, 2]);
    let mut out = [9i16; 2];
    assert_eq!(queue.pull(&mut out).real, 1);
    assert_eq!(out, [1, 0]);
}

#[test]
fn the_anchor_counts_real_samples_only_and_resets() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[1; 100]));
    let mut out = [0i16; 60];
    queue.pull(&mut out);
    queue.pull(&mut out);
    assert_eq!(queue.played_since_anchor(), 100, "silence is not counted");

    queue.reset_anchor();
    assert_eq!(queue.played_since_anchor(), 0);
    queue.push_pcm16(&pcm(&[1; 10]));
    queue.pull(&mut out);
    assert_eq!(queue.played_since_anchor(), 10);
}

#[test]
fn clear_empties_the_queue_and_resets_the_anchor() {
    let mut queue = PlaybackQueue::new();
    queue.push_pcm16(&pcm(&[1; 50]));
    let mut out = [0i16; 20];
    queue.pull(&mut out);
    queue.clear();
    assert!(queue.is_empty());
    assert_eq!(queue.played_since_anchor(), 0);
    let pull = queue.pull(&mut out);
    assert_eq!(pull.real, 0);
    assert!(!pull.drained, "a flush is not a drain");
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
