//! The speech gate (spec_voice §3.3): only the user's voice reaches the daemon. The capture's
//! 48 kHz goes down to the wire's 24 kHz and, beside it, to the detector's 16 kHz; a frame no
//! voice is near goes out as digital silence, so a key click or a fan never reaches the
//! provider's own turn detection.

use fermix_client::realtime::speech::{
    Decimator, SpeechGate, Uplink, CAPTURE_FRAME, CAPTURE_RATE, CHUNK_BYTES, HANGOVER_FRAMES,
    PRE_ROLL_FRAMES, WIRE_FRAME,
};
use std::f64::consts::PI;

/// 1.504 s of the JFK inaugural address at 48 kHz: 320 ms of room tone, then speech.
const SPEECH: &[u8] = include_bytes!("fixtures/speech/jfk_48k_s16le.raw");
const SPEECH_FRAMES: usize = 94;
/// A frame of the fixture this loud is speech; its room tone sits near -43 dBFS.
const SPEECH_DB: f64 = -30.0;
/// The first frame of the fixture above `SPEECH_DB`, at 320 ms.
const ONSET: usize = 20;
/// Output samples a filter needs before its history is all real input.
const SETTLE: usize = 200;
const PRE_ROLL: usize = PRE_ROLL_FRAMES as usize;
const HANGOVER: usize = HANGOVER_FRAMES as usize;

fn pcm16(bytes: &[u8]) -> Vec<i16> {
    let (pairs, odd) = bytes.as_chunks::<2>();
    assert!(odd.is_empty(), "whole PCM16 samples");
    pairs.iter().map(|pair| i16::from_le_bytes(*pair)).collect()
}

fn speech() -> Vec<i16> {
    let samples = pcm16(SPEECH);
    assert_eq!(samples.len(), SPEECH_FRAMES * CAPTURE_FRAME);
    samples
}

fn scaled(samples: &[i16], db: f64) -> Vec<i16> {
    let gain = 10f64.powf(db / 20.0);
    samples
        .iter()
        .map(|&s| (f64::from(s) * gain).round() as i16)
        .collect()
}

fn tone(hz: f64, amplitude: f64, samples: usize) -> Vec<i16> {
    let rate = f64::from(CAPTURE_RATE);
    (0..samples)
        .map(|n| (amplitude * 32_767.0 * (2.0 * PI * hz * n as f64 / rate).sin()).round() as i16)
        .collect()
}

fn level_db(samples: &[i16]) -> f64 {
    let power: f64 = samples
        .iter()
        .map(|&s| (f64::from(s) / 32_768.0).powi(2))
        .sum::<f64>()
        / samples.len() as f64;
    10.0 * (power + 1e-20).log10()
}

/// A deterministic noise source in [-1, 1], so the tests never depend on a crate's RNG.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// Typing: a press and, 70 to 120 ms later, a softer release, each a broadband burst decaying
/// over `tau_ms`, `per_second` times a second at up to `peak_db`, over a -54 dBFS room.
fn typing(per_second: f64, peak_db: f64, tau_ms: f64, seconds: f64) -> Vec<i16> {
    let rate = f64::from(CAPTURE_RATE);
    let len = (seconds * rate) as usize;
    let mut noise = Noise(per_second.to_bits() ^ peak_db.to_bits() ^ tau_ms.to_bits());
    let floor = 10f64.powf(-54.0 / 20.0) * 3f64.sqrt();
    let mut out: Vec<f64> = (0..len).map(|_| floor * noise.next()).collect();
    let tau = tau_ms / 1000.0 * rate;
    let mut at = (0.05 * rate) as usize;
    while at + (0.2 * rate) as usize <= len {
        let peak = 10f64.powf((peak_db - 6.0 * (noise.next() + 1.0) / 2.0) / 20.0);
        let release = at + ((0.07 + 0.05 * (noise.next() + 1.0) / 2.0) * rate) as usize;
        for (start, amplitude) in [(at, peak), (release, peak / 2.0)] {
            for i in 0..(6.0 * tau) as usize {
                out[start + i] += amplitude * (-(i as f64) / tau).exp() * noise.next();
            }
        }
        let gap = 1.0 / per_second * (0.5 + (noise.next() + 1.0) / 2.0);
        at += (gap * rate) as usize;
    }
    out.iter()
        .map(|&x| (x * 32_767.0).round().clamp(-32_768.0, 32_767.0) as i16)
        .collect()
}

fn decimate(mut decimator: Decimator, input: &[i16]) -> Vec<i16> {
    let mut out = Vec::new();
    decimator.push(input, &mut out);
    out
}

/// Pushes `input` in pieces of `piece` samples, the microphone open, and returns every block.
fn send(uplink: &mut Uplink, input: &[i16], piece: usize) -> Vec<Vec<u8>> {
    input
        .chunks(piece)
        .flat_map(|chunk| uplink.push(chunk, true))
        .collect()
}

/// The same with the microphone muted or not yet armed: what comes back is never sent.
fn mute(uplink: &mut Uplink, input: &[i16], piece: usize) -> Vec<Vec<u8>> {
    input
        .chunks(piece)
        .flat_map(|chunk| uplink.push(chunk, false))
        .collect()
}

/// White noise at `db` dBFS: a fan, to the detector.
fn fan(db: f64, seconds: f64) -> Vec<i16> {
    let mut noise = Noise(7);
    let amplitude = 10f64.powf(db / 20.0) * 3f64.sqrt() * 32_767.0;
    (0..(seconds * f64::from(CAPTURE_RATE)) as usize)
        .map(|_| (amplitude * noise.next()).round() as i16)
        .collect()
}

fn samples(blocks: &[Vec<u8>]) -> Vec<i16> {
    blocks.iter().flat_map(|block| pcm16(block)).collect()
}

/// The speech, then enough silence for every one of its frames to be decided and sent.
fn speech_sent(speech: &[i16]) -> Vec<i16> {
    let mut input = speech.to_vec();
    input.resize(input.len() + (PRE_ROLL + 25) * CAPTURE_FRAME, 0);
    let sent = samples(&send(&mut Uplink::new(), &input, 441));
    assert!(sent.len() >= SPEECH_FRAMES * WIRE_FRAME);
    sent
}

fn frame_open(sent: &[i16], frame: usize) -> bool {
    sent[frame * WIRE_FRAME..(frame + 1) * WIRE_FRAME]
        .iter()
        .any(|&s| s != 0)
}

// The decimators

#[test]
fn both_decimators_keep_a_steady_level_at_unity_gain() {
    let steady = vec![10_000i16; CAPTURE_RATE as usize / 2];
    for decimator in [Decimator::wire(), Decimator::detector()] {
        let out = decimate(decimator, &steady);
        assert!(
            out[SETTLE..].iter().all(|&s| (s - 10_000).abs() <= 2),
            "{:?}",
            &out[SETTLE..SETTLE + 8]
        );
    }
}

#[test]
fn a_speech_band_tone_passes_both_decimators() {
    let input = tone(1_000.0, 0.5, CAPTURE_RATE as usize);
    let input_db = level_db(&input);
    for decimator in [Decimator::wire(), Decimator::detector()] {
        let out = decimate(decimator, &input);
        let loss = input_db - level_db(&out[SETTLE..]);
        assert!(loss.abs() <= 0.5, "1 kHz lost {loss:.2} dB");
    }
}

/// Above the new Nyquist a tone would fold back into the band as a false one.
#[test]
fn tones_past_each_new_nyquist_are_rejected() {
    let cases = [
        (Decimator::wire(), 12_500.0),
        (Decimator::wire(), 18_000.0),
        (Decimator::detector(), 8_500.0),
        (Decimator::detector(), 14_000.0),
    ];
    for (decimator, hz) in cases {
        let input = tone(hz, 0.5, CAPTURE_RATE as usize);
        let out = decimate(decimator, &input);
        let rejected = level_db(&input) - level_db(&out[SETTLE..]);
        assert!(rejected >= 40.0, "{hz} Hz only {rejected:.1} dB down");
    }
}

#[test]
fn a_decimator_gives_the_same_samples_however_its_input_is_split() {
    let input = speech();
    let makers: [fn() -> Decimator; 2] = [Decimator::wire, Decimator::detector];
    for make in makers {
        let expected = decimate(make(), &input);
        for piece in [1, 2, 3, 441, 480, 767, 4_800] {
            let mut split = make();
            let mut out = Vec::new();
            for chunk in input.chunks(piece) {
                split.push(chunk, &mut out);
            }
            assert_eq!(out, expected, "pieces of {piece}");
        }
    }
}

/// The wire and the detector see the same moment: one click lands at the same time in both.
#[test]
fn the_two_decimators_delay_alike() {
    let mut click = vec![0i16; CAPTURE_RATE as usize / 10];
    click[1_000] = 30_000;
    let centre = |out: &[i16], rate: f64| {
        let weight: f64 = out.iter().map(|&s| f64::from(s)).sum();
        let moment: f64 = out
            .iter()
            .enumerate()
            .map(|(n, &s)| n as f64 * f64::from(s))
            .sum();
        moment / weight / rate
    };
    let wire = centre(&decimate(Decimator::wire(), &click), 24_000.0);
    let detector = centre(&decimate(Decimator::detector(), &click), 16_000.0);
    assert!(
        (wire - detector).abs() < 1e-4,
        "{wire:.6} s against {detector:.6} s"
    );
}

// The gate

fn decisions(voiced: &[usize], frames: usize) -> Vec<bool> {
    let mut gate = SpeechGate::new();
    let decided: Vec<bool> = (0..frames)
        .filter_map(|frame| gate.push(voiced.contains(&frame)))
        .collect();
    assert_eq!(decided.len(), frames - PRE_ROLL);
    decided
}

fn open_frames(decided: &[bool]) -> Vec<usize> {
    (0..decided.len()).filter(|&k| decided[k]).collect()
}

#[test]
fn the_gate_decides_each_frame_pre_roll_frames_late() {
    let mut gate = SpeechGate::new();
    for _ in 0..PRE_ROLL {
        assert_eq!(gate.push(true), None);
    }
    assert_eq!(gate.push(false), Some(true), "frame 0, voiced");
}

#[test]
fn one_voiced_frame_opens_its_pre_roll_and_its_hangover() {
    let voiced = 30;
    let expected: Vec<usize> = (voiced - PRE_ROLL..=voiced + HANGOVER).collect();
    assert_eq!(open_frames(&decisions(&[voiced], 80)), expected);
}

#[test]
fn a_pause_shorter_than_the_hangover_and_pre_roll_stays_open() {
    let bridged = 30 + HANGOVER + PRE_ROLL + 1;
    let expected: Vec<usize> = (30 - PRE_ROLL..=bridged + HANGOVER).collect();
    assert_eq!(open_frames(&decisions(&[30, bridged], 100)), expected);

    let apart = bridged + 1;
    let decided = decisions(&[30, apart], 100);
    assert!(
        !decided[30 + HANGOVER + 1],
        "the frame between them is closed"
    );
    assert!(decided[30 + HANGOVER] && decided[apart - PRE_ROLL]);
}

#[test]
fn a_gate_that_heard_no_voice_stays_closed() {
    assert!(open_frames(&decisions(&[], 60)).is_empty());
}

// The uplink

/// What goes out is whole 100 ms blocks, `PRE_ROLL_FRAMES` behind the capture: a frame leaves
/// once the frames after it have said whether voice was coming.
#[test]
fn the_uplink_sends_whole_100_ms_blocks_pre_roll_late() {
    // 25 frames of 16 ms are exactly four 100 ms blocks.
    let input = vec![0i16; (PRE_ROLL + 25) * CAPTURE_FRAME];
    let (first, last) = input.split_at(input.len() - CAPTURE_FRAME);
    let mut uplink = Uplink::new();
    let before = send(&mut uplink, first, 441);
    assert_eq!(before.len(), 3, "24 frames out, the 25th still held");
    let after = send(&mut uplink, last, 441);
    assert_eq!(after.len(), 1, "the last frame captured lets the 25th out");
    assert!(before
        .iter()
        .chain(&after)
        .all(|block| block.len() == CHUNK_BYTES));
}

#[test]
fn silence_goes_out_as_silence() {
    let input = vec![0i16; CAPTURE_RATE as usize];
    let blocks = send(&mut Uplink::new(), &input, 480);
    assert!(!blocks.is_empty());
    assert!(samples(&blocks).iter().all(|&s| s == 0));
}

#[test]
fn keyboard_clicks_go_out_as_silence() {
    for (per_second, peak_db, tau_ms) in [
        (3.0, -30.0, 1.0),
        (7.0, -22.0, 1.0),
        (7.0, -22.0, 4.0),
        (12.0, -16.0, 2.0),
    ] {
        let input = typing(per_second, peak_db, tau_ms, 4.0);
        let sent = samples(&send(&mut Uplink::new(), &input, 480));
        assert!(
            sent.len() >= 3 * 24_000,
            "the clicks went through the uplink"
        );
        let leaked = sent.iter().filter(|&&s| s != 0).count();
        assert_eq!(
            leaked, 0,
            "{per_second}/s at {peak_db} dB, {tau_ms} ms: {leaked} samples"
        );
    }
}

/// Speech goes out whole from its first word, and exactly as the wire's decimation of it.
#[test]
fn speech_goes_out_from_its_onset_untouched() {
    let input = speech();
    let sent = speech_sent(&input);
    assert!(frame_open(&sent, ONSET), "the first word's onset was cut");
    let loud: Vec<usize> = (0..SPEECH_FRAMES)
        .filter(|&k| level_db(&input[k * CAPTURE_FRAME..(k + 1) * CAPTURE_FRAME]) > SPEECH_DB)
        .collect();
    let passed = loud.iter().filter(|&&k| frame_open(&sent, k)).count();
    assert_eq!(passed, loud.len(), "speech frames silenced");

    let wire = decimate(Decimator::wire(), &input);
    for k in loud {
        let frame = k * WIRE_FRAME..(k + 1) * WIRE_FRAME;
        assert_eq!(sent[frame.clone()], wire[frame], "frame {k} was altered");
    }
}

/// The room before the first word is not voice: past the pre-roll it goes out as silence.
#[test]
fn room_tone_before_speech_goes_out_as_silence() {
    let sent = speech_sent(&speech());
    let room = ONSET - PRE_ROLL;
    let open: Vec<usize> = (0..room).filter(|&k| frame_open(&sent, k)).collect();
    assert!(open.is_empty(), "room tone sent in frames {open:?}");
}

/// A built-in laptop microphone hears speech about 24 dB quieter than this recording.
#[test]
fn quiet_speech_at_a_laptop_microphone_level_still_goes_out() {
    let input = speech();
    let quiet = scaled(&input, -24.0);
    let sent = speech_sent(&quiet);
    assert!(frame_open(&sent, ONSET), "the quiet first word was cut");
    let loud: Vec<usize> = (0..SPEECH_FRAMES)
        .filter(|&k| level_db(&input[k * CAPTURE_FRAME..(k + 1) * CAPTURE_FRAME]) > SPEECH_DB)
        .collect();
    let passed = loud.iter().filter(|&&k| frame_open(&sent, k)).count();
    assert_eq!(passed, loud.len(), "quiet speech frames silenced");
}

/// While the microphone is muted or not yet armed, the detector still hears the room, so it
/// is warm when the microphone opens; but the wire gets silence.
#[test]
fn while_muted_everything_the_uplink_gives_back_is_silence() {
    let words = speech();
    let mut muted = Uplink::new();
    let back = samples(&mute(&mut muted, &words, 480));
    assert!(
        back.len() >= 70 * WIRE_FRAME,
        "the muted capture went through"
    );
    assert!(back.iter().all(|&s| s == 0), "muted speech came back");
    // The control: the same speech with the microphone open does come back.
    let open = samples(&send(&mut Uplink::new(), &words, 480));
    assert!(open.iter().any(|&s| s != 0));
}

/// Nothing captured before the microphone opens is sent after it: not the frames waiting for
/// their decision, not the partial block, not the filter's memory of it.
#[test]
fn nothing_captured_while_muted_is_sent_once_the_microphone_opens() {
    let words = &speech()[..(70 * CAPTURE_FRAME + 123)];
    let silence = vec![0i16; (PRE_ROLL + 25) * CAPTURE_FRAME];

    // The control: had the microphone been open, the held frames of speech would go out.
    let mut open = Uplink::new();
    send(&mut open, words, 441);
    let after = samples(&send(&mut open, &silence, 441));
    assert!(after.iter().any(|&s| s != 0), "nothing was held");

    let mut muted = Uplink::new();
    mute(&mut muted, words, 441);
    let after = samples(&send(&mut muted, &silence, 441));
    assert!(after.len() >= 25 * WIRE_FRAME, "the uplink kept sending");
    assert!(
        after.iter().all(|&s| s == 0),
        "speech captured while muted was sent"
    );
}

/// A mute shorter than the filter still silences everything held before it, the filter's
/// memory too.
#[test]
fn even_a_moment_of_mute_silences_everything_held() {
    let words = &speech()[..(70 * CAPTURE_FRAME + 123)];
    let silence = vec![0i16; (PRE_ROLL + 25) * CAPTURE_FRAME];
    let mut uplink = Uplink::new();
    send(&mut uplink, words, 441);
    assert!(mute(&mut uplink, &[0; 50], 50).is_empty());
    let after = samples(&send(&mut uplink, &silence, 441));
    assert!(after.len() >= 25 * WIRE_FRAME);
    let leaked = after.iter().filter(|&&s| s != 0).count();
    assert_eq!(leaked, 0, "{leaked} samples from before the mute were sent");
}

/// Speech from the moment the microphone opens goes out as the wire's decimation of it alone:
/// the speech before, muted, blends into none of it.
#[test]
fn speech_just_after_the_microphone_opens_goes_out_untouched() {
    let input = speech();
    let muted_frames = 70;
    let mut opened = input[43 * CAPTURE_FRAME..].to_vec();
    opened.resize(opened.len() + (PRE_ROLL + 25) * CAPTURE_FRAME, 0);

    let mut uplink = Uplink::new();
    let before = samples(&mute(
        &mut uplink,
        &input[..muted_frames * CAPTURE_FRAME],
        441,
    ));
    let sent = samples(&send(&mut uplink, &opened, 441));
    // The wire's samples, numbered from the start of the stream.
    let mut stream = before;
    stream.extend_from_slice(&sent);
    let mut wire_input = vec![0i16; muted_frames * CAPTURE_FRAME];
    wire_input.extend_from_slice(&opened);
    let expected = decimate(Decimator::wire(), &wire_input);

    assert!(stream[..muted_frames * WIRE_FRAME].iter().all(|&s| s == 0));
    assert!(
        frame_open(&stream, muted_frames),
        "the first word after opening was cut"
    );
    let open: Vec<usize> = (muted_frames..stream.len() / WIRE_FRAME)
        .filter(|&k| frame_open(&stream, k))
        .collect();
    assert!(open.len() >= 30);
    for k in open {
        let frame = k * WIRE_FRAME..(k + 1) * WIRE_FRAME;
        assert_eq!(
            stream[frame.clone()],
            expected[frame],
            "frame {k} was altered"
        );
    }
}

/// A fan already running when the microphone opens is not sent: the detector has been hearing
/// it all along. A detector that started cold would take its first frames for a voice.
#[test]
fn a_fan_already_running_when_the_microphone_opens_is_not_sent() {
    let room = fan(-30.0, 2.0);
    let mut uplink = Uplink::new();
    mute(&mut uplink, &room, 480);
    let sent = samples(&send(&mut uplink, &fan(-30.0, 2.0), 480));
    assert!(sent.len() >= 24_000);
    let leaked = sent.iter().filter(|&&s| s != 0).count();
    assert_eq!(leaked, 0, "{leaked} samples of fan noise sent");

    // The control: a detector that starts cold takes the fan's first frames for a voice.
    let cold = samples(&send(&mut Uplink::new(), &fan(-30.0, 2.0), 480));
    assert!(cold.iter().any(|&s| s != 0), "a cold start passed nothing");
}
