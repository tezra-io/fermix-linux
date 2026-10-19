//! The chat page's one player of voice notes, as state (macOS `AppPlayer`, `ChatSoundState`): one
//! note at a time, from the press that asked for its file; stopped at its end, when its chip goes,
//! when another note's file comes, and while a call is up; and what a chip shows of it.

use fermix_client::player::{written, Key, Player, Pressed, Shown, STEP, TICK_MS};

const LAMP: &str = "lamp-note";
const SHOP: &str = "shop-note";

/// A player holding `note`, playing from its start.
fn playing(note: &str) -> Player {
    let mut player = Player::default();
    let Pressed::Ask(press) = player.press(note) else {
        panic!("a first press asks for the file");
    };
    assert!(player.granted(press));
    player
}

fn shown(held: bool, playing: bool, position: f64, length: Option<f64>) -> Shown {
    Shown {
        held,
        playing,
        position,
        length,
    }
}

#[test]
fn the_starting_numbers_are_the_macs() {
    assert_eq!(STEP, 5.0);
    assert_eq!(TICK_MS, 250);
}

#[test]
fn a_press_asks_for_the_file_and_its_grant_plays_it_from_the_start() {
    let mut player = Player::default();
    assert_eq!(player.press(LAMP), Pressed::Ask(1));
    assert!(
        !player.holds(LAMP),
        "nothing is held while the file is asked for"
    );

    assert!(player.granted(1));
    assert!(player.holds(LAMP));
    assert_eq!(player.shown(LAMP, None), shown(true, true, 0.0, None));
    assert!(!player.granted(1), "one grant plays once");
}

#[test]
fn a_press_on_the_held_note_pauses_and_resumes_it() {
    let mut player = playing(LAMP);
    player.moved(2.0);

    assert_eq!(player.press(LAMP), Pressed::Pause);
    assert_eq!(player.shown(LAMP, None), shown(true, false, 2.0, None));
    assert_eq!(player.press(LAMP), Pressed::Resume);
    assert_eq!(player.shown(LAMP, None), shown(true, true, 2.0, None));
}

/// The first note plays on while the second's file is asked for, and stops when it comes.
#[test]
fn another_notes_file_stops_the_one_playing() {
    let mut player = playing(LAMP);
    let Pressed::Ask(press) = player.press(SHOP) else {
        panic!("another note asks for its own file");
    };
    assert!(player.holds(LAMP));

    assert!(player.granted(press));
    assert!(player.holds(SHOP));
    assert!(!player.holds(LAMP));
    assert_eq!(
        player.shown(LAMP, Some(6.0)),
        shown(false, false, 0.0, Some(6.0))
    );
}

#[test]
fn a_press_overtaken_by_another_plays_nothing_when_its_file_comes() {
    let mut player = Player::default();
    let Pressed::Ask(first) = player.press(LAMP) else {
        panic!("asks");
    };
    let Pressed::Ask(second) = player.press(SHOP) else {
        panic!("asks");
    };

    assert!(!player.granted(first));
    assert!(player.held().is_none());
    assert!(player.granted(second));
    assert!(player.holds(SHOP));
}

/// A note that played to its end looks as the Mac's does, back at its start with its whole length
/// shown and nothing filled, but is let go with its file: the next press asks for the file again.
#[test]
fn a_note_that_ends_shows_its_start_and_its_length_and_asks_again() {
    let mut player = playing(LAMP);
    player.length_read(6.69);
    player.moved(6.69);

    player.let_go(LAMP);
    assert!(player.held().is_none());
    let still = player.shown(LAMP, None);
    assert_eq!(still, shown(false, false, 0.0, Some(6.69)));
    assert_eq!(still.shown_time(), Some(6.69));
    assert_eq!(still.filled(), 0.0);
    assert_eq!(
        player.shown(LAMP, Some(6.5)).length,
        Some(6.5),
        "the decoded length still wins"
    );
    assert_eq!(
        player.shown(SHOP, None).length,
        None,
        "each note its own length"
    );
    assert_eq!(player.press(LAMP), Pressed::Ask(2));
}

/// A chip going away lets go of its own note, held or asked for, and of nothing else.
#[test]
fn a_chip_going_away_stops_its_own_note_only() {
    let mut player = playing(LAMP);
    player.let_go(SHOP);
    assert!(player.holds(LAMP));

    let Pressed::Ask(press) = player.press(SHOP) else {
        panic!("asks");
    };
    player.let_go(SHOP);
    assert!(!player.granted(press), "the chip that asked is gone");
    assert!(player.holds(LAMP));

    player.let_go(LAMP);
    assert!(player.held().is_none());
}

/// Leaving the chat page stops whatever it held, and any press still asking.
#[test]
fn a_stop_lets_go_of_everything() {
    let mut player = playing(LAMP);
    let Pressed::Ask(press) = player.press(SHOP) else {
        panic!("asks");
    };

    player.stop();
    assert!(player.held().is_none());
    assert!(!player.granted(press));
}

/// Nothing plays while a call is up: it stops what played, drops a press still asking, and offers
/// nothing until the call is over.
#[test]
fn a_call_coming_up_stops_the_player_until_it_is_over() {
    let mut player = playing(LAMP);
    let Pressed::Ask(press) = player.press(SHOP) else {
        panic!("asks");
    };

    player.call(true);
    assert!(player.call_up());
    assert!(player.held().is_none());
    assert!(!player.granted(press));
    assert_eq!(player.press(LAMP), Pressed::Nothing);
    assert_eq!(player.seek(LAMP, 1.0, None), None);

    player.call(false);
    assert!(!player.call_up());
    assert!(matches!(player.press(LAMP), Pressed::Ask(_)));
}

/// A note not held has no file to move along until it is played.
#[test]
fn only_the_held_note_seeks() {
    let mut player = playing(LAMP);

    assert_eq!(player.seek(SHOP, 3.0, None), None);
    assert_eq!(player.seek(LAMP, 3.0, None), Some(3.0));
    assert_eq!(player.shown(LAMP, None).position, 3.0);
    assert_eq!(player.seek(LAMP, -1.0, None), Some(0.0));
}

/// A seek lands no further than the note's end: its decoded length, else the one the pipeline
/// read; with neither known there is no end to hold it to.
#[test]
fn a_seek_lands_no_further_than_the_notes_end() {
    let mut player = playing(LAMP);
    assert_eq!(player.seek(LAMP, 9.0, None), Some(9.0));

    player.length_read(4.0);
    assert_eq!(player.seek(LAMP, 9.0, None), Some(4.0));
    assert_eq!(player.seek(LAMP, 9.0, Some(6.0)), Some(6.0));
    assert_eq!(player.shown(LAMP, Some(6.0)).position, 6.0);
    assert_eq!(player.seek(LAMP, 2.0, Some(6.0)), Some(2.0));
}

/// The length a chip goes by is the one its levels decoded, and the player's until then: a
/// container's duration can lie (AVFoundation read a 6.69 s Opus note as 9.36 s).
#[test]
fn the_decoded_length_wins_over_the_players() {
    let mut player = playing(LAMP);
    assert_eq!(player.shown(LAMP, None).length, None);

    player.length_read(9.36);
    assert_eq!(player.shown(LAMP, None).length, Some(9.36));
    assert_eq!(player.shown(LAMP, Some(6.69)).length, Some(6.69));

    player.length_read(f64::NAN);
    player.length_read(0.0);
    assert_eq!(
        player.shown(LAMP, None).length,
        Some(9.36),
        "no length is not a length"
    );
}

#[test]
fn a_note_not_held_shows_its_length_and_an_empty_track() {
    let state = shown(false, false, 0.0, Some(84.0));

    assert!(!state.under_way());
    assert_eq!(state.shown_time(), Some(84.0));
    assert_eq!(state.filled(), 0.0);
}

#[test]
fn a_note_playing_or_paused_past_its_start_shows_where_it_is() {
    let playing = shown(true, true, 21.0, Some(84.0));
    assert!(playing.under_way());
    assert_eq!(playing.shown_time(), Some(21.0));
    assert_eq!(playing.filled(), 0.25);

    let paused = shown(true, false, 30.0, Some(60.0));
    assert!(paused.under_way());
    assert_eq!(paused.shown_time(), Some(30.0));
    assert_eq!(paused.filled(), 0.5);
}

#[test]
fn a_note_whose_length_is_not_known_shows_no_time_and_fills_nothing() {
    let state = shown(false, false, 0.0, None);

    assert_eq!(state.shown_time(), None);
    assert_eq!(state.filled(), 0.0);
    assert_eq!(state.at(0.5), None);
}

#[test]
fn an_arrow_key_moves_five_seconds_held_to_the_note() {
    assert_eq!(shown(true, true, 30.0, Some(60.0)).stepped(true), 35.0);
    assert_eq!(shown(true, true, 30.0, Some(60.0)).stepped(false), 25.0);
    assert_eq!(shown(true, false, 3.0, Some(60.0)).stepped(false), 0.0);
    assert_eq!(shown(true, true, 58.0, Some(60.0)).stepped(true), 60.0);
    assert_eq!(shown(false, false, 0.0, None).stepped(true), 5.0);
    assert_eq!(shown(true, true, 4.0, Some(6.69)).stepped(true), 6.69);
}

/// The waveform's keys, a slider's: Home to the start, End to the end once the length is known,
/// Left and Right five seconds, held to the note.
#[test]
fn the_waveforms_keys_move_as_a_sliders_do() {
    let known = shown(true, true, 30.0, Some(60.0));
    assert_eq!(known.keyed(Key::Home), Some(0.0));
    assert_eq!(known.keyed(Key::End), Some(60.0));
    assert_eq!(known.keyed(Key::Left), Some(25.0));
    assert_eq!(known.keyed(Key::Right), Some(35.0));

    let near_the_end = shown(true, true, 58.0, Some(60.0));
    assert_eq!(near_the_end.keyed(Key::Right), Some(60.0));
    let near_the_start = shown(true, true, 3.0, Some(60.0));
    assert_eq!(near_the_start.keyed(Key::Left), Some(0.0));

    let unknown = shown(true, true, 3.0, None);
    assert_eq!(unknown.keyed(Key::Home), Some(0.0));
    assert_eq!(unknown.keyed(Key::End), None, "no end to go to");
    assert_eq!(unknown.keyed(Key::Right), Some(8.0));
}

#[test]
fn a_click_lands_along_the_track_and_never_past_either_end() {
    let state = shown(false, false, 0.0, Some(80.0));

    assert_eq!(state.at(0.25), Some(20.0));
    assert_eq!(state.at(-0.5), Some(0.0));
    assert_eq!(state.at(1.5), Some(80.0));
    assert_eq!(shown(true, true, 6.69, Some(6.69)).at(0.5), Some(3.345));
}

#[test]
fn the_fill_never_runs_past_the_track() {
    assert_eq!(shown(true, true, 90.0, Some(60.0)).filled(), 1.0);
    assert_eq!(shown(true, true, 10.0, Some(0.0)).filled(), 0.0);
    assert_eq!(shown(true, true, 6.69, Some(6.69)).filled(), 1.0);
}

/// Rounded to the second, half to even as the Mac's formatter; the hours only where it has any.
#[test]
fn the_time_is_written_with_the_hours_only_where_it_has_any() {
    assert_eq!(written(0.0), "0:00");
    assert_eq!(written(6.69), "0:07");
    assert_eq!(written(65.0), "1:05");
    assert_eq!(written(599.6), "10:00");
    assert_eq!(written(3600.0), "1:00:00");
    assert_eq!(written(3725.0), "1:02:05");
    assert_eq!(written(-3.0), "0:00");
    assert_eq!(written(f64::NAN), "0:00");
}
