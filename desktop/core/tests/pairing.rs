//! The Phone dialog's pure half (M60 §3.3 to §3.5): the reducer over the daemon's answers, the
//! guards an answer is held to before anything is drawn, and the code drawn from the link.
//!
//! Every answer is the vendored contract's own golden, changed in exactly the field a case is
//! about, so a reducer that passes here passes on the shapes the engine publishes.

use fermix_client::mobile::{
    MobileStatus, PairingSession, PairingStart, PairingSummary, SessionState,
};
use fermix_client::pairing::{
    guards, reduce, Answer, Compare, EndAction, Ending, PairingCode, PairingLink, Progress, Step,
    StepKind, TurnOn,
};
use fermix_client::phone::{
    ENDED_CANCELLED, ENDED_DENIED, ENDED_ELSEWHERE, ENDED_EXPIRED, ENDED_UNREADABLE, PAIR_AGAIN,
    RESTART, START_OVER, TURN_ON_AND_RESTART,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const SESSION: &str = "5b0c7d2e-8f41-4a6b-9c3d-2e7f1a8b4c60";
const OTHER: &str = "0d9e8f7a-6b5c-4d3e-8f2a-1b0c9d8e7f6a";
const HARDWARE: &str = "Fermix does not check a phone's secure hardware yet.";

/// The golden named `name`, its result changed by `change`, decoded as the app decodes it.
fn golden<T: DeserializeOwned>(name: &str, change: impl FnOnce(&mut Value)) -> T {
    let line = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no golden named {name}"));
    let mut result = line["response"]["result"].clone();
    change(&mut result);
    serde_json::from_value(result).unwrap_or_else(|e| panic!("{name} decodes: {e}"))
}

fn status(change: impl FnOnce(&mut Value)) -> MobileStatus {
    golden("mobile_status", change)
}

fn start(name: &str, change: impl FnOnce(&mut Value)) -> PairingStart {
    golden(name, change)
}

fn session(name: &str) -> PairingSession {
    golden(name, |_| {})
}

fn session_with(name: &str, change: impl FnOnce(&mut Value)) -> PairingSession {
    golden(name, change)
}

fn waiting() -> Step {
    Step::Waiting { session: None }
}

/// Scan, as the golden start opens it.
fn scanning() -> Step {
    reduce(
        &waiting(),
        Answer::Started(start("mobile_pair_start", |_| {})),
    )
    .step
}

/// Compare, as a read of the golden phone waiting for a decision gives it.
fn comparing() -> Step {
    reduce(
        &scanning(),
        Answer::Session(session("mobile_pair_get_awaiting_decision")),
    )
    .step
}

fn ended(sentence: &str) -> Step {
    Step::Ended(Ending {
        sentence: sentence.to_owned(),
        action: EndAction::PairAgain,
    })
}

fn golden_link() -> String {
    let start: Value = serde_json::from_str(
        SUCCESS
            .lines()
            .find(|l| l.contains("\"name\": \"mobile_pair_start\""))
            .expect("the start golden"),
    )
    .expect("JSON");
    start["response"]["result"]["uri"]
        .as_str()
        .expect("a link")
        .to_owned()
}

// Opening

#[test]
fn pairing_opens_the_window_on_a_running_channel_and_turn_on_on_one_that_is_not() {
    let running = reduce(&waiting(), Answer::Status(status(|_| {})));
    assert_eq!(running.step, waiting());

    let owed = reduce(
        &waiting(),
        Answer::Status(status(|s| s["started"] = json!(false))),
    );
    assert_eq!(owed.step, Step::TurnOn(TurnOn::new(false)));

    let off = reduce(
        &waiting(),
        Answer::Status(status(|s| {
            s["enabled"] = json!(false);
            s["started"] = json!(false);
        })),
    );
    assert_eq!(off.step, Step::TurnOn(TurnOn::new(true)));
    assert_eq!(TurnOn::new(true).action_title(), TURN_ON_AND_RESTART);
    assert_eq!(TurnOn::new(false).action_title(), RESTART);
    assert_eq!(TurnOn::new(true).progress, Progress::Idle);
}

// Every transition, on the goldens

#[test]
fn the_start_opens_scan_with_the_link_it_hands_back_once() {
    let transition = reduce(
        &waiting(),
        Answer::Started(start("mobile_pair_start", |_| {})),
    );
    let Step::Scan(scan) = &transition.step else {
        panic!("expected Scan, got {:?}", transition.step.kind());
    };
    assert_eq!(scan.session, SESSION);
    assert_eq!(scan.ttl_ms, 120_000);
    assert_eq!(
        scan.link.text(),
        golden_link(),
        "Can't scan the code? shows the link the code was drawn from"
    );
    let link = guards::link(&golden_link()).expect("the golden link passes");
    assert_eq!(scan.code, PairingCode::make(&link).expect("a code"));
    assert_eq!(transition.abandons, None);
}

#[test]
fn a_start_refused_with_the_channel_off_ends_with_the_daemons_sentence() {
    let transition = reduce(
        &waiting(),
        Answer::Started(start("mobile_pair_start_channel_off", |_| {})),
    );
    assert_eq!(transition.step, ended("The mobile channel is turned off."));
    assert_eq!(transition.abandons, None, "a refused start opened nothing");
}

#[test]
fn scan_follows_the_daemons_clock_and_moves_to_compare_when_a_phone_has_scanned() {
    let scan = scanning();
    let next = reduce(
        &scan,
        Answer::Session(session("mobile_pair_get_awaiting_scan")),
    );
    let (Step::Scan(first), Step::Scan(later)) = (&scan, &next.step) else {
        panic!("expected Scan, got {:?}", next.step.kind());
    };
    assert_eq!(later.ttl_ms, 112_000);
    assert_eq!(later.code, first.code, "no read repeats the link");
    assert_eq!(later.link, first.link);

    let compare = reduce(
        &scan,
        Answer::Session(session("mobile_pair_get_awaiting_decision")),
    );
    assert_eq!(
        compare.step,
        Step::Compare(Compare {
            session: SESSION.into(),
            device_name: "Sam's phone".into(),
            model: "Google Pixel 9 Pro".into(),
            digits: "481062".into(),
            hardware: HARDWARE.into(),
            ttl_ms: 83_400,
        })
    );
    assert_eq!(compare.abandons, None);
}

/// 78a7292: the daemon counts the window down to zero before it says it expired, so a read in
/// that last moment carries zero and the code stays up.
#[test]
fn a_code_read_in_its_last_moment_stays_up_until_the_daemon_says_it_expired() {
    let last = reduce(
        &scanning(),
        Answer::Session(session_with("mobile_pair_get_awaiting_scan", |s| {
            s["ttl_ms"] = json!(0)
        })),
    );
    let Step::Scan(scan) = &last.step else {
        panic!("expected Scan, got {:?}", last.step.kind());
    };
    assert_eq!(scan.ttl_ms, 0);
    assert_eq!(last.abandons, None);
}

#[test]
fn approve_and_a_read_of_an_approved_session_both_end_on_paired() {
    for name in ["mobile_pair_decide", "mobile_pair_get_approved"] {
        let paired = reduce(&comparing(), Answer::Session(session(name)));
        assert_eq!(
            paired.step,
            Step::Paired {
                name: "Sam's phone".into()
            },
            "{name}"
        );
    }
}

/// The daemon writes a sentence only for `failed`; the three endings it reports by reason alone
/// take the app's words.
#[test]
fn each_ending_says_what_the_daemon_reported() {
    let cases = [
        ("mobile_pair_get_denied", ENDED_DENIED),
        ("mobile_pair_get_expired", ENDED_EXPIRED),
        ("mobile_pair_get_cancelled", ENDED_CANCELLED),
        ("mobile_pair_cancel", ENDED_CANCELLED),
        (
            "mobile_pair_get_failed",
            "The phone disconnected before you decided. Start pairing again.",
        ),
    ];
    for (name, sentence) in cases {
        let transition = reduce(&comparing(), Answer::Session(session(name)));
        assert_eq!(transition.step, ended(sentence), "{name}");
        assert_eq!(transition.abandons, None, "{name} is over already");
    }
    assert_eq!(
        ENDED_EXPIRED,
        "The code expired. Pairing codes last two minutes."
    );
    assert_eq!(ENDED_DENIED, "You denied this phone.");
    assert_eq!(ENDED_CANCELLED, "Pairing was cancelled.");
}

#[test]
fn an_ending_is_worded_by_its_reason_not_by_its_state() {
    let timeout_as_cancelled = session_with("mobile_pair_get_cancelled", |s| {
        s["outcome"]["reason"] = json!("timeout")
    });
    let transition = reduce(&comparing(), Answer::Session(timeout_as_cancelled));
    assert_eq!(transition.step, ended(ENDED_EXPIRED));
}

#[test]
fn a_refusal_ends_the_session_with_the_daemons_own_words() {
    let sentence = "Only the owner can pair or forget a phone; run this from your own terminal.";
    for step in [waiting(), scanning(), comparing()] {
        let transition = reduce(&step, Answer::Refused(sentence.into()));
        assert_eq!(transition.step, ended(sentence));
        let Step::Ended(ending) = &transition.step else {
            unreachable!()
        };
        assert_eq!(ending.action_title(), PAIR_AGAIN);
    }
}

#[test]
fn an_answer_about_another_window_changes_nothing() {
    for step in [scanning(), comparing()] {
        let stray = session_with("mobile_pair_get_expired", |s| {
            s["session_id"] = json!(OTHER)
        });
        let transition = reduce(&step, Answer::Session(stray));
        assert_eq!(transition.step, step);
        assert_eq!(transition.abandons, None);
    }
    let over = ended(ENDED_CANCELLED);
    let late = reduce(
        &over,
        Answer::Session(session("mobile_pair_get_awaiting_decision")),
    );
    assert_eq!(
        late.step, over,
        "a read that reaches a dialog that has moved on is dropped"
    );
}

// Busy

#[test]
fn busy_with_a_phone_waiting_elsewhere_resumes_that_window_for_compare() {
    let pairing = PairingSummary {
        session_id: OTHER.into(),
        state: SessionState::AwaitingDecision,
    };
    let resumed = reduce(&waiting(), Answer::Busy(Some(pairing)));
    assert_eq!(
        resumed.step,
        Step::Waiting {
            session: Some(OTHER.into())
        }
    );
    let compare = reduce(
        &resumed.step,
        Answer::Session(session_with("mobile_pair_get_awaiting_decision", |s| {
            s["session_id"] = json!(OTHER)
        })),
    );
    assert_eq!(compare.step.kind(), StepKind::Compare);
}

#[test]
fn busy_with_a_code_waiting_elsewhere_offers_to_start_over() {
    let pairing = PairingSummary {
        session_id: OTHER.into(),
        state: SessionState::AwaitingScan,
    };
    let elsewhere = reduce(&waiting(), Answer::Busy(Some(pairing)));
    let expected = Ending {
        sentence: ENDED_ELSEWHERE.into(),
        action: EndAction::StartOver {
            session: Some(OTHER.into()),
        },
    };
    assert_eq!(elsewhere.step, Step::Ended(expected.clone()));
    assert_eq!(expected.action_title(), START_OVER);
    assert_eq!(
        ENDED_ELSEWHERE,
        "A pairing code is already open somewhere else."
    );

    let unnamed = reduce(&waiting(), Answer::Busy(None));
    let Step::Ended(ending) = unnamed.step else {
        panic!("busy with nothing named ends")
    };
    assert_eq!(ending.action, EndAction::StartOver { session: None });

    // A window resumed for Compare that turns out to be waiting for a scan has no code to draw.
    let resumed = Step::Waiting {
        session: Some(OTHER.into()),
    };
    let scan_elsewhere = reduce(
        &resumed,
        Answer::Session(session_with("mobile_pair_get_awaiting_scan", |s| {
            s["session_id"] = json!(OTHER)
        })),
    );
    let Step::Ended(ending) = scan_elsewhere.step else {
        panic!("a code this dialog did not open ends")
    };
    assert_eq!(ending.sentence, ENDED_ELSEWHERE);
}

// The guards

#[test]
fn the_link_is_held_to_its_prefix_its_length_and_its_characters() {
    assert!(guards::link(&golden_link()).is_some());
    assert!(guards::link("https://example.com/pair?x=1").is_none());
    assert!(guards::link("fermix://pairing?x=1").is_none());
    let ceiling = format!("fermix://pair?s={}", "a".repeat(2048 - 16));
    assert_eq!(ceiling.len(), 2048);
    assert!(guards::link(&ceiling).is_some());
    assert!(guards::link(&format!("{ceiling}a")).is_none());
    assert!(guards::link("fermix://pair?v=2\n&secret=x").is_none());
    assert!(guards::link("fermix://pair?v=2\u{7}").is_none());
    assert!(guards::link("fermix://pair?v=2\u{85}").is_none());
    assert_eq!(guards::LINK_PREFIX, "fermix://pair?");
    assert_eq!(guards::MAX_LINK_BYTES, 2048);
}

#[test]
fn a_start_whose_answer_fails_a_guard_ends_and_cancels_the_window_it_opened() {
    let changes = [
        ("a bad link", "uri", json!("https://x")),
        ("no link", "uri", Value::Null),
        ("a zero window", "ttl_ms", json!(0)),
        ("a long window", "ttl_ms", json!(120_001)),
        ("no window", "ttl_ms", Value::Null),
    ];
    for (case, field, value) in changes {
        let transition = reduce(
            &waiting(),
            Answer::Started(start("mobile_pair_start", |s| s[field] = value)),
        );
        assert_eq!(transition.step, ended(ENDED_UNREADABLE), "{case}");
        assert_eq!(transition.abandons.as_deref(), Some(SESSION), "{case}");
    }
    assert_eq!(
        ENDED_UNREADABLE,
        "Fermix answered a pairing code this app cannot show."
    );
}

#[test]
fn a_window_opens_with_one_to_120000_milliseconds_and_a_later_read_may_carry_zero() {
    assert_eq!(guards::ttl(Some(1)), Some(1));
    assert_eq!(guards::ttl(Some(120_000)), Some(120_000));
    for bad in [Some(0), Some(-1), Some(120_001), None] {
        assert_eq!(guards::ttl(bad), None, "{bad:?}");
    }
    assert_eq!(guards::remaining(Some(0)), Some(0));
    assert_eq!(guards::remaining(Some(120_000)), Some(120_000));
    for bad in [Some(-1), Some(120_001), None] {
        assert_eq!(guards::remaining(bad), None, "{bad:?}");
    }
    let over = reduce(
        &scanning(),
        Answer::Session(session_with("mobile_pair_get_awaiting_scan", |s| {
            s["ttl_ms"] = json!(120_001)
        })),
    );
    assert_eq!(over.step, ended(ENDED_UNREADABLE));
    assert_eq!(over.abandons.as_deref(), Some(SESSION));
}

#[test]
fn the_code_is_six_digits() {
    assert!(guards::digits("481062"));
    for bad in ["48106", "4810623", "48106a", "４８１０６２", ""] {
        assert!(!guards::digits(bad), "{bad}");
    }
    let letters = reduce(
        &scanning(),
        Answer::Session(session_with("mobile_pair_get_awaiting_decision", |s| {
            s["request"]["sas"] = json!("48106x")
        })),
    );
    assert_eq!(letters.step, ended(ENDED_UNREADABLE));
    assert_eq!(letters.abandons.as_deref(), Some(SESSION));
}

#[test]
fn the_phones_name_and_model_are_present_and_at_most_128_bytes() {
    assert!(guards::field("Sam's phone"));
    assert!(guards::field(&"a".repeat(128)));
    assert!(!guards::field(&"a".repeat(129)));
    assert!(!guards::field(""));
    for key in ["device_name", "model"] {
        for bad in [String::new(), "é".repeat(65)] {
            let answer = session_with("mobile_pair_get_awaiting_decision", |s| {
                s["request"][key] = json!(bad)
            });
            let transition = reduce(&scanning(), Answer::Session(answer));
            assert_eq!(transition.step, ended(ENDED_UNREADABLE), "{key}");
        }
    }
    let nameless = session_with("mobile_pair_get_approved", |s| {
        s["request"]["device_name"] = json!("")
    });
    let transition = reduce(&comparing(), Answer::Session(nameless));
    assert_eq!(transition.step, ended(ENDED_UNREADABLE));
    assert_eq!(transition.abandons, None, "an approved window is over");
}

#[test]
fn a_state_or_a_reason_this_app_cannot_read_ends_with_its_own_sentence() {
    let new_state = session_with("mobile_pair_get_awaiting_scan", |s| {
        s["state"] = json!("awaiting_wonder")
    });
    let transition = reduce(&scanning(), Answer::Session(new_state));
    assert_eq!(transition.step, ended(ENDED_UNREADABLE));
    assert_eq!(
        transition.abandons.as_deref(),
        Some(SESSION),
        "a window in a state this app cannot read may still be open"
    );

    let new_reason = session_with("mobile_pair_get_denied", |s| {
        s["outcome"]["reason"] = json!("vanished")
    });
    let transition = reduce(&comparing(), Answer::Session(new_reason));
    assert_eq!(transition.step, ended(ENDED_UNREADABLE));
    assert_eq!(transition.abandons, None);

    let silent_failure = session_with("mobile_pair_get_failed", |s| s["failure"] = Value::Null);
    let transition = reduce(&comparing(), Answer::Session(silent_failure));
    assert_eq!(transition.step, ended(ENDED_UNREADABLE));
}

// Closing

#[test]
fn closing_cancels_the_window_in_scan_and_compare_and_nothing_once_it_has_ended() {
    assert_eq!(scanning().open_session(), Some(SESSION));
    assert_eq!(comparing().open_session(), Some(SESSION));
    assert_eq!(
        Step::Waiting {
            session: Some(OTHER.into())
        }
        .open_session(),
        Some(OTHER)
    );
    let closed = [
        waiting(),
        Step::TurnOn(TurnOn::new(true)),
        Step::Paired { name: "x".into() },
        ended(ENDED_DENIED),
        Step::Phones,
    ];
    for step in closed {
        assert_eq!(step.open_session(), None, "{:?}", step.kind());
    }
}

// The link and its code

#[test]
fn neither_the_link_nor_its_code_reaches_a_debug_print() {
    let scan = scanning();
    let printed = format!("{scan:?}");
    assert!(!printed.contains("fermix://"), "{printed}");
    assert!(!printed.contains("EXAMPLE-ONE-USE-SECRET"), "{printed}");
    let Step::Scan(scan) = scan else {
        unreachable!()
    };
    assert_eq!(format!("{:?}", scan.link), "PairingLink(withheld)");
    assert!(format!("{:?}", scan.code).contains("withheld"));
    let started = start("mobile_pair_start", |_| {});
    assert!(!format!("{started:?}").contains("EXAMPLE-ONE-USE-SECRET"));
}

/// Reads a code back the way a phone's camera would: drawn at four pixels a module on its quiet
/// zone, then decoded by an independent reader.
fn read_back(code: &PairingCode) -> String {
    let scale = 4;
    let side = code.span() * scale;
    let zone = PairingCode::QUIET_ZONE;
    let mut image = rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| {
        let (column, row) = (x / scale, y / scale);
        let inside = (zone..zone + code.width()).contains(&column)
            && (zone..zone + code.width()).contains(&row);
        if inside && code.dark(row - zone, column - zone) {
            0
        } else {
            255
        }
    });
    let grids = image.detect_grids();
    assert_eq!(grids.len(), 1, "one code is found");
    grids[0].decode().expect("the code decodes").1
}

#[test]
fn the_golden_links_code_reads_back_as_the_link_the_right_way_up() {
    let link = guards::link(&golden_link()).expect("the golden link passes");
    let code = PairingCode::make(&link).expect("a code");
    assert_eq!(read_back(&code), golden_link());
    // The three finder patterns sit top left, top right and bottom left.
    let width = code.width();
    for (row, column) in [(0, 0), (0, width - 7), (width - 7, 0)] {
        assert!(code.dark(row, column) && code.dark(row + 6, column + 6));
        assert!(!code.dark(row + 1, column + 1));
    }
}

/// The modules sit as the encoder lays them out, x across and y down, so the card is drawn the
/// right way up: the dark module every code carries sits beside the bottom left finder (ISO/IEC
/// 18004 §7.9.1). A reader tolerant of mirror images would decode a transposed card too.
#[test]
fn the_modules_are_read_across_then_down() {
    let link = guards::link(&golden_link()).expect("passes");
    let code = PairingCode::make(&link).expect("a code");
    let encoded = qrcode::QrCode::with_error_correction_level(golden_link(), qrcode::EcLevel::M)
        .expect("a code");
    let width = code.width();
    assert_eq!(width, encoded.width());
    for row in 0..width {
        for column in 0..width {
            let dark = encoded[(column, row)] == qrcode::Color::Dark;
            assert_eq!(code.dark(row, column), dark, "row {row}, column {column}");
        }
    }
    assert!(code.dark(width - 8, 8));
}

#[test]
fn a_link_at_the_2048_byte_ceiling_produces_a_code_that_reads_back_and_fits_the_dialog() {
    let text = format!("fermix://pair?s={}", "Ab9".repeat(700))[..2048].to_owned();
    let link: PairingLink = guards::link(&text).expect("the ceiling passes");
    let code = PairingCode::make(&link).expect("a 2048-byte link fits level M");
    assert!(code.width() > 160, "near the largest code");
    assert_eq!(read_back(&code), text);
    assert_eq!(code.module_px(), 2);
    assert!(code.side() <= PairingCode::MAX_SIDE);
    assert_eq!(PairingCode::MAX_SIDE, 370);
}

#[test]
fn the_card_scales_by_whole_pixels_with_a_four_module_quiet_zone() {
    let link = guards::link(&golden_link()).expect("passes");
    let code = PairingCode::make(&link).expect("a code");
    assert_eq!(code.span(), code.width() + 8);
    assert!(code.module_px() >= 2);
    assert_eq!(
        code.module_px(),
        (PairingCode::PREFERRED_SIDE / code.span()).max(2)
    );
    assert_eq!(code.side(), code.span() * code.module_px());
}
