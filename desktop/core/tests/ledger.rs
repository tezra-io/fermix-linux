//! The permission ledger and the platform statements: one source of each string (M38 §6.5, §7.4).

use fermix_client::ledger::{MICROPHONE_DETAIL, MICROPHONE_HEADLINE, RIGHTS};

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

/// The owner's plain wording (2026-09-26): what Linux does not do, what this app
/// does with the microphone, and the one control that works.
#[test]
fn the_microphone_statement_says_three_things_plainly() {
    assert_eq!(MICROPHONE_HEADLINE, "How Fermix uses your microphone");
    assert!(MICROPHONE_DETAIL.contains("Linux does not ask before an app uses the microphone"));
    assert!(MICROPHONE_DETAIL.contains("only during a voice call"));
    assert!(MICROPHONE_DETAIL.contains("mute the microphone in your sound settings"));
    assert_eq!(
        MICROPHONE_DETAIL,
        "Linux does not ask before an app uses the microphone, so you will not see a permission \
         prompt. This app turns the microphone on only during a voice call and off when the call \
         ends. To make sure nothing hears you, mute the microphone in your sound settings."
    );
}
