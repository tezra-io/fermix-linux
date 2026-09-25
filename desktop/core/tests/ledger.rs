//! The permission ledger and the platform statements: one source of each string (M38 §6.5, §7.4).

use fermix_client::ledger::{MICROPHONE_STATEMENT, RIGHTS};

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
    assert!(MICROPHONE_STATEMENT.contains("Linux has no microphone permission"));
    assert!(MICROPHONE_STATEMENT.contains("nothing to revoke"));
    assert!(MICROPHONE_STATEMENT.contains("so can any other program you run"));
    assert!(MICROPHONE_STATEMENT.contains("mute the microphone"));
    assert!(MICROPHONE_STATEMENT.contains("On macOS the operating system asks first"));
}
