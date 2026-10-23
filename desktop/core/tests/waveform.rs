//! A voice note's waveform (macOS `SoundLevels`, `SoundPower`, `ChatWaveformLayout`): power on a
//! decibel curve from a floor of silence up to the loudest stretch, a quiet room never stretched to
//! fill, stretches shared out over what was really decoded and merged to the bars that fit, and
//! what a levels pass ends in. Pure: no file is read.

use fermix_client::waveform::{
    admit, bar_height, bar_levels, bars_fitting, heard, played, spans, Levels, Power, Unheard,
    BUCKETS, CAPACITY, FLOOR, MAX_BYTES, MAX_FRAMES, QUIET_TOP, RATE, RESTING, WINDOW_FRAMES,
};

fn close(lhs: &[f32], rhs: &[f32]) -> bool {
    lhs.len() == rhs.len() && lhs.iter().zip(rhs).all(|(l, r)| (l - r).abs() < 0.0001)
}

fn power_of(window_frames: u64, samples: &[f32]) -> Power {
    let mut power = Power::new(window_frames);
    power.add(samples);
    power
}

/// `seconds` of a tone at `RATE`, as the appsink hands it over.
fn tone(seconds: u64) -> Vec<f32> {
    (0..seconds * u64::from(RATE))
        .map(|frame| (frame as f32 * 0.5).sin() * 0.5)
        .collect()
}

#[test]
fn the_starting_numbers_are_the_macs() {
    assert_eq!(BUCKETS, 80);
    assert_eq!(FLOOR, -60.0);
    assert_eq!(QUIET_TOP, -30.0);
    assert_eq!(RATE, 8_000);
    assert_eq!(WINDOW_FRAMES, 80, "10 ms at the rate");
    assert_eq!(CAPACITY, 1_024);
    assert_eq!(RESTING, 0.15);
    assert_eq!(MAX_FRAMES, 10 * 60 * 8_000);
    assert_eq!(MAX_BYTES, 50 * 1024 * 1024);
}

/// −20 dBFS is the loudest and fills the height; −40 is halfway up the 40 dB from the floor; the
/// floor and anything under it, or nothing at all, is silence.
#[test]
fn levels_run_in_decibels_from_the_floor_to_the_loudest_stretch() {
    let levels = Levels::from_power(&[1e-2, 1e-4, 1e-6, 1e-8, 0.0], 5.0);

    assert!(close(levels.values(), &[1.0, 0.5, 0.0, 0.0, 0.0]));
    assert_eq!(levels.seconds(), 5.0);
    assert!(close(Levels::from_power(&[1.0], 1.0).values(), &[1.0]));
    assert!(close(
        Levels::from_power(&[0.0, 1.0, 0.0], 3.0).values(),
        &[0.0, 1.0, 0.0]
    ));
}

/// Nothing louder than −30 dBFS is a quiet room: it is drawn against −30, so its noise is never
/// stretched into a wave.
#[test]
fn a_quiet_sound_is_never_stretched_to_fill_the_height() {
    let levels = Levels::from_power(&[1e-4, 1e-5, 1e-7], 3.0);
    assert!(close(levels.values(), &[20.0 / 30.0, 10.0 / 30.0, 0.0]));

    let room = Levels::from_power(&[1e-4; 4], 4.0);
    assert!(close(room.values(), &[2.0 / 3.0; 4]), "{:?}", room.values());

    assert_eq!(Levels::from_power(&[0.0, 0.0], 2.0).values(), &[0.0, 0.0]);
    assert!(Levels::from_power(&[], 0.0).values().is_empty());
    assert_eq!(Levels::from_power(&[f64::NAN], 1.0).values(), &[0.0]);
}

#[test]
fn things_are_shared_out_among_parts_evenly_never_empty_while_there_are_any() {
    assert_eq!(spans(10, 4), vec![0..2, 2..5, 5..7, 7..10]);
    assert_eq!(spans(3, 6), vec![0..1, 0..1, 1..2, 1..2, 2..3, 2..3]);
    assert_eq!(spans(4, 4), vec![0..1, 1..2, 2..3, 3..4]);
    assert_eq!(spans(0, 3), vec![0..0, 0..0, 0..0]);
    assert!(spans(5, 0).is_empty());

    let tens = spans(800, 80);
    assert_eq!(tens.len(), 80);
    assert!(tens
        .iter()
        .enumerate()
        .all(|(i, span)| *span == (i * 10..i * 10 + 10)));
    let few = spans(5, 80);
    assert_eq!(few.len(), 80);
    assert!(few.iter().all(|span| span.len() == 1 && span.end <= 5));
}

/// Fewer bars than stretches keep each one's loudest, so a peak is never lost; more repeat them.
#[test]
fn levels_merge_to_the_bars_that_fit_each_the_loudest_it_covers() {
    let levels = Levels::new(vec![0.1, 0.9, 0.2, 0.4], 4.0);

    assert_eq!(levels.bars(2), vec![0.9, 0.4]);
    assert_eq!(levels.bars(4), levels.values().to_vec());
    assert_eq!(levels.bars(8), vec![0.1, 0.1, 0.9, 0.9, 0.2, 0.2, 0.4, 0.4]);
    assert!(levels.bars(0).is_empty());
    assert_eq!(Levels::new(Vec::new(), 0.0).bars(3), vec![0.0, 0.0, 0.0]);
}

#[test]
fn power_is_summed_in_windows_the_open_one_counted_every_frame_counted() {
    let mut power = power_of(2, &[1.0, 1.0, 0.0, 0.0, 0.5, 0.5]);

    assert_eq!(power.stretches(3), vec![1.0, 0.0, 0.25]);
    assert_eq!(power.stretches(1), vec![2.5 / 6.0]);
    assert_eq!(power.frames(), 6);

    power.add(&[1.0]);
    assert_eq!(power.stretches(4), vec![1.0, 0.0, 0.25, 1.0]);
    assert_eq!(power.frames(), 7);
    assert!(Power::new(2).stretches(4).is_empty());
}

/// Nothing about a note's length is known before it ends, so its windows merge in pairs as it
/// runs long: four times the capacity of single frames ends in windows of eight, every mean kept.
#[test]
fn a_long_sounds_windows_merge_in_pairs_so_it_costs_no_more_to_hold() {
    let half = CAPACITY / 2;
    let mut samples = vec![1.0; half];
    samples.extend(vec![0.0; half]);
    let mut power = power_of(1, &samples);

    assert_eq!(power.window_frames(), 2);
    assert_eq!(power.stretches(2), vec![1.0, 0.0]);

    power.add(&vec![1.0; CAPACITY * 3]);
    assert_eq!(power.window_frames(), 8);
    assert_eq!(power.frames(), (CAPACITY * 4) as u64);
    assert_eq!(
        power.stretches(1),
        vec![(CAPACITY * 7 / 2) as f64 / (CAPACITY * 4) as f64]
    );
}

#[test]
fn a_steady_tone_reads_as_even_bars() {
    let power = power_of(WINDOW_FRAMES, &tone(3));
    let stretches = power.stretches(BUCKETS);

    assert_eq!(stretches.len(), BUCKETS);
    let (low, high) = stretches
        .iter()
        .fold((f64::MAX, f64::MIN), |(low, high), p| {
            (low.min(*p), high.max(*p))
        });
    assert!(high / low < 1.05, "{low} to {high}");
}

/// The bars as many as fit, the last with no gap after it; the resting bars, low and even, until
/// the levels are read, so nothing moves when they come.
#[test]
fn the_waveform_lays_out_its_bars() {
    assert_eq!(bars_fitting(0.0), 0);
    assert_eq!(bars_fitting(1.0), 0);
    assert_eq!(bars_fitting(2.0), 1);
    assert_eq!(bars_fitting(5.0), 1);
    assert_eq!(bars_fitting(6.0), 2);
    assert_eq!(bars_fitting(10.0), 3);
    assert_eq!(bars_fitting(238.0), 60);
    assert_eq!(bars_fitting(-4.0), 0);
    assert!(bars_fitting(318.0) <= BUCKETS);

    assert_eq!(bar_levels(None, 12), vec![RESTING; 12]);
    let levels = Levels::new(vec![0.1, 0.9], 2.0);
    assert_eq!(bar_levels(Some(&levels), 4), vec![0.1, 0.1, 0.9, 0.9]);
}

/// A bar fills once the place passes its middle; a bar's height never falls under the shortest,
/// so silence still reads as a line.
#[test]
fn bars_fill_as_it_plays_and_stand_from_a_minimum() {
    assert_eq!(played(0.5, 10), 5);
    assert_eq!(played(0.04, 10), 0);
    assert_eq!(played(0.05, 10), 1);
    assert_eq!(played(2.0, 10), 10);
    assert_eq!(played(-1.0, 10), 0);
    assert_eq!(played(f64::NAN, 10), 0);

    assert_eq!(bar_height(0.0, 28.0), 3.0);
    assert_eq!(bar_height(1.0, 28.0), 28.0);
    assert_eq!(bar_height(0.5, 28.0), 3.0 + 25.0 / 2.0);
    assert_eq!(bar_height(2.0, 28.0), 28.0);
    assert_eq!(bar_height(-1.0, 28.0), 3.0);
    assert_eq!(bar_height(1.0, 2.0), 3.0);
}

#[test]
fn a_note_past_the_size_a_voice_note_can_be_is_not_read() {
    assert_eq!(admit(0), Ok(()));
    assert_eq!(admit(MAX_BYTES), Ok(()));
    assert_eq!(
        admit(MAX_BYTES + 1),
        Err(Unheard::TooBig {
            bytes: MAX_BYTES + 1
        })
    );
}

/// The length is the frames really decoded over the rate, never the container's word.
#[test]
fn a_pass_that_ends_gives_the_levels_and_the_decoded_length() {
    let power = power_of(WINDOW_FRAMES, &tone(3));
    let levels = heard(&power, None).expect("levels");

    assert_eq!(levels.seconds(), 3.0);
    assert_eq!(levels.values().len(), BUCKETS);
    assert!(levels.values().iter().all(|v| (0.0..=1.0).contains(v)));

    let short = power_of(WINDOW_FRAMES, &[0.5; 4_000]);
    assert_eq!(heard(&short, None).expect("short").seconds(), 0.5);
}

/// A failed pass is no Ogg Opus this app decodes: the note falls back to its file tile. One that
/// decoded nothing, or ran past ten minutes, keeps its resting bars and still plays.
#[test]
fn a_pass_that_fails_decodes_nothing_or_runs_long_has_no_levels() {
    let power = power_of(WINDOW_FRAMES, &tone(1));
    let failed = heard(&power, Some("no opusdec link".to_owned()));
    assert_eq!(
        failed,
        Err(Unheard::Undecodable("no opusdec link".to_owned()))
    );
    assert!(failed.unwrap_err().falls_back());

    let silent = heard(&Power::new(WINDOW_FRAMES), None);
    assert_eq!(silent, Err(Unheard::Silent));
    assert!(!silent.unwrap_err().falls_back());

    let mut long = Power::new(WINDOW_FRAMES);
    let minute = vec![0.25; 60 * RATE as usize];
    for _ in 0..10 {
        long.add(&minute);
    }
    assert_eq!(long.frames(), MAX_FRAMES);
    assert_eq!(heard(&long, None).expect("ten minutes").seconds(), 600.0);
    long.add(&[0.25]);
    assert_eq!(heard(&long, None), Err(Unheard::TooLong));
    assert!(!Unheard::TooLong.falls_back());
    assert!(!Unheard::TooBig { bytes: 1 }.falls_back());
}
