//! The permission ledger and the platform statements: one source of each string (M38 §6.5, §7.4).

use fermix_client::ledger::{microphone_statement, MICROPHONE_DETAIL, MICROPHONE_HEADLINE, RIGHTS};

#[test]
fn the_ledger_names_seven_rights_each_with_who_how_and_where() {
    assert_eq!(RIGHTS.len(), 7);
    for right in RIGHTS {
        for text in [right.principal, right.revoke, right.artifact] {
            assert!(text.ends_with('.'), "{}: {text}", right.title);
        }
    }
    assert_eq!(RIGHTS[0].title, "Microphone and voice");
}

#[test]
fn the_microphone_statement_says_all_four_things_it_has_to_say() {
    let statement = microphone_statement();
    assert!(statement.contains("Linux has no microphone permission"));
    assert!(statement.contains("nothing to revoke"));
    assert!(statement.contains("so can any other program you run"));
    assert!(statement.contains("mute the microphone"));
    assert!(statement.contains("On macOS the operating system asks first"));
}

#[test]
fn the_folded_statement_is_its_headline_over_the_rest_word_for_word() {
    assert_eq!(MICROPHONE_HEADLINE, "Linux has no microphone permission");
    assert!(MICROPHONE_DETAIL.starts_with("Nothing asked you"));
    assert_eq!(
        microphone_statement(),
        "Linux has no microphone permission. Nothing asked you, nothing appears in your system \
         settings, and there is nothing to revoke. While Fermix is running it can open the \
         microphone at any time, and so can any other program you run. Your real controls are to \
         not run it, to mute the microphone in your sound settings or in PipeWire, or to run it \
         in a sandbox that withholds audio, which also stops it playing sound. On macOS the \
         operating system asks first. On Linux it does not."
    );
}
