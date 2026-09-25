//! The conversation the Chat page draws: what was asked, what streamed back,
//! which tools ran, and whether a reply is still coming.

use fermix_client::acp::{StopReason, ToolStatus, Update};
use fermix_client::chat::{tool_label, Entry, Phase, Transcript};

fn chunk(text: &str) -> Update {
    Update::MessageChunk(text.into())
}

/// A wall-clock time, in seconds since the epoch, `n` seconds into the test.
fn at(n: i64) -> i64 {
    1_790_000_000 + n
}

#[test]
fn a_new_conversation_is_empty_and_ready() {
    let t = Transcript::default();
    assert!(t.entries().is_empty());
    assert_eq!(t.phase(), Phase::Idle);
    assert!(t.can_send());
}

#[test]
fn sending_records_the_question_and_waits_for_the_reply() {
    let mut t = Transcript::default();
    assert!(t.send("  What is on my calendar?  ", at(0)));
    assert_eq!(t.entries(), [Entry::User("What is on my calendar?".into())]);
    assert_eq!(t.phase(), Phase::Waiting);
    assert!(!t.can_send(), "one reply at a time");
    assert!(
        !t.send("again", at(0)),
        "a second question waits for the first reply"
    );
}

#[test]
fn a_blank_message_is_never_sent() {
    let mut t = Transcript::default();
    assert!(!t.send("   \n ", at(0)));
    assert!(t.entries().is_empty());
}

#[test]
fn chunks_join_into_one_reply_until_a_tool_runs_between_them() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&chunk("Hel"));
    assert_eq!(t.phase(), Phase::Streaming);
    t.apply(&chunk("lo."));
    t.apply(&Update::ToolCall {
        id: "t1".into(),
        title: "web_search".into(),
        status: ToolStatus::Running,
    });
    t.apply(&Update::ToolUpdate {
        id: "t1".into(),
        status: ToolStatus::Completed,
    });
    t.apply(&chunk("Found it."));
    assert_eq!(
        t.entries(),
        [
            Entry::User("hi".into()),
            Entry::Assistant("Hello.".into()),
            Entry::Tool {
                id: "t1".into(),
                title: "web_search".into(),
                status: ToolStatus::Completed
            },
            Entry::Assistant("Found it.".into()),
        ]
    );
}

#[test]
fn an_update_for_a_tool_nobody_announced_changes_nothing() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&Update::ToolUpdate {
        id: "ghost".into(),
        status: ToolStatus::Failed,
    });
    assert_eq!(t.entries(), [Entry::User("hi".into())]);
}

#[test]
fn a_finished_reply_frees_the_composer() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&chunk("Hello"));
    t.finish(&StopReason::EndTurn, at(1));
    assert_eq!(t.phase(), Phase::Idle);
    assert!(t.can_send());
    assert_eq!(t.entries().len(), 2);
}

#[test]
fn stopping_marks_the_reply_as_stopped() {
    let mut t = Transcript::default();
    t.send("write an essay", at(0));
    t.apply(&chunk("Once"));
    assert!(t.stop());
    assert_eq!(t.phase(), Phase::Stopping);
    assert!(!t.stop(), "a second stop sends nothing");
    t.finish(&StopReason::Cancelled, at(1));
    assert_eq!(t.phase(), Phase::Idle);
    assert_eq!(t.entries().last(), Some(&Entry::Notice("Stopped".into())));
}

#[test]
fn nothing_to_stop_while_idle() {
    let mut t = Transcript::default();
    assert!(!t.stop());
}

#[test]
fn a_failed_reply_says_why_and_frees_the_composer() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.fail("Fermix could not answer.", at(1));
    assert_eq!(t.phase(), Phase::Idle);
    assert_eq!(
        t.entries().last(),
        Some(&Entry::Failure("Fermix could not answer.".into()))
    );
}

#[test]
fn a_tool_still_running_when_the_reply_ends_is_left_as_it_was_last_seen() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&Update::ToolCall {
        id: "t1".into(),
        title: "shell".into(),
        status: ToolStatus::Running,
    });
    t.finish(&StopReason::Cancelled, at(1));
    assert!(matches!(
        t.entries()[1],
        Entry::Tool {
            status: ToolStatus::Running,
            ..
        }
    ));
}

#[test]
fn tool_names_read_as_words() {
    assert_eq!(tool_label("web_search"), "Web search");
    assert_eq!(tool_label("memory.recall"), "Memory recall");
    assert_eq!(tool_label(""), "Tool");
}

#[test]
fn a_refused_reply_is_explained_in_words_a_person_can_act_on() {
    use fermix_client::chat::{reply_error, ReplyError};
    assert_eq!(
        reply_error(
            -32000,
            "Re-authenticate: the model provider rejected Fermix's credentials."
        ),
        ReplyError::SignInAgain
    );
    assert_eq!(
        ReplyError::SignInAgain.sentence(),
        "Your provider refused Fermix's sign-in. Sign in again under Providers."
    );
    assert_eq!(
        reply_error(
            -32603,
            "the Fermix turn failed; the daemon log has the reason"
        ),
        ReplyError::Failed
    );
    assert_eq!(
        ReplyError::Failed.sentence(),
        "Fermix could not answer. The reason is in its log."
    );
    assert_eq!(
        reply_error(-32602, "prompt must be a list of ACP content blocks"),
        ReplyError::Refused("prompt must be a list of ACP content blocks".into())
    );
}

#[test]
fn a_note_marks_where_the_assistant_stopped_remembering() {
    let mut t = Transcript::default();
    t.note("The connection to Fermix ended, so your next message starts a new conversation.");
    assert!(
        t.entries().is_empty(),
        "nothing to separate in an empty conversation"
    );
    t.send("hi", at(0));
    t.finish(&fermix_client::acp::StopReason::EndTurn, at(1));
    t.note("The connection to Fermix ended, so your next message starts a new conversation.");
    t.note("The connection to Fermix ended, so your next message starts a new conversation.");
    assert_eq!(
        t.entries().last(),
        Some(&Entry::Notice(
            "The connection to Fermix ended, so your next message starts a new conversation."
                .into()
        ))
    );
    assert_eq!(
        t.entries().len(),
        2,
        "the same note twice in a row is shown once"
    );
}

fn picture() -> fermix_client::acp::Image {
    fermix_client::acp::Image {
        mime: "image/png".into(),
        bytes: vec![1, 2, 3].into(),
    }
}

fn running(id: &str) -> Update {
    Update::ToolCall {
        id: id.into(),
        title: "web_search".into(),
        status: ToolStatus::Running,
    }
}

#[test]
fn thoughts_gather_into_one_entry_ahead_of_the_reply() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&Update::ThoughtChunk("Look".into()));
    t.apply(&Update::ThoughtChunk("ing it up.".into()));
    assert_eq!(t.phase(), Phase::Streaming);
    t.apply(&chunk("Found it."));
    assert_eq!(
        t.entries(),
        [
            Entry::User("hi".into()),
            Entry::Thought("Looking it up.".into()),
            Entry::Assistant("Found it.".into()),
        ]
    );
}

#[test]
fn a_picture_sits_between_the_text_around_it() {
    let mut t = Transcript::default();
    t.send("show me", at(0));
    t.apply(&chunk("Here:"));
    t.apply(&Update::Image(picture()));
    t.apply(&chunk("Like it?"));
    assert_eq!(
        t.entries(),
        [
            Entry::User("show me".into()),
            Entry::Assistant("Here:".into()),
            Entry::Image(picture()),
            Entry::Assistant("Like it?".into()),
        ]
    );
}

#[test]
fn a_file_that_could_not_come_through_is_named_in_its_own_entry() {
    let mut t = Transcript::default();
    t.send("screenshot please", at(0));
    t.apply(&Update::Attachment("shot.png".into()));
    t.apply(&chunk("Sent."));
    assert_eq!(t.entries()[1], Entry::Attachment("shot.png".into()));
    assert_eq!(t.entries()[2], Entry::Assistant("Sent.".into()));
}

#[test]
fn the_orb_shows_while_fermix_works_and_nothing_else_on_screen_moves() {
    let mut t = Transcript::default();
    assert!(!t.thinking(), "nothing asked yet");
    t.send("hi", at(0));
    assert!(t.thinking(), "asked, nothing back");
    t.apply(&Update::ThoughtChunk("Hmm".into()));
    assert!(t.thinking(), "thoughts are collapsed, so the orb stays");
    t.apply(&running("t1"));
    assert!(!t.thinking(), "a running tool shows its own progress");
    t.apply(&Update::ToolUpdate {
        id: "t1".into(),
        status: ToolStatus::Completed,
    });
    assert!(t.thinking(), "between a tool and the next text");
    t.apply(&chunk("Hel"));
    assert!(!t.thinking(), "the text streaming in shows the work");
    t.apply(&Update::Image(picture()));
    assert!(t.thinking(), "after a picture, before more text");
    assert!(t.stop());
    assert!(t.thinking(), "stopping still waits on Fermix");
    t.finish(&StopReason::Cancelled, at(1));
    assert!(!t.thinking());
}

#[test]
fn each_question_and_each_finished_reply_carry_a_time() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    assert_eq!(t.time(0), Some(at(0)));
    t.apply(&chunk("Checking."));
    t.apply(&running("t1"));
    t.apply(&chunk("Done."));
    assert_eq!(t.time(1), None, "a reply is timed once it ends");
    assert_eq!(t.time(3), None);
    t.finish(&StopReason::EndTurn, at(9));
    assert_eq!(t.time(1), None, "only the end of the reply");
    assert_eq!(t.time(2), None);
    assert_eq!(t.time(3), Some(at(9)));
    assert_eq!(t.time(4), None, "past the end");
}

#[test]
fn a_stopped_reply_is_timed_above_its_note() {
    let mut t = Transcript::default();
    t.send("essay", at(0));
    t.apply(&chunk("Once"));
    t.stop();
    t.finish(&StopReason::Cancelled, at(5));
    assert_eq!(t.entries()[2], Entry::Notice("Stopped".into()));
    assert_eq!(t.time(1), Some(at(5)));
    assert_eq!(t.time(2), None);
}

#[test]
fn a_reply_stopped_before_anything_came_keeps_only_the_question_time() {
    let mut t = Transcript::default();
    t.send("essay", at(0));
    t.stop();
    t.finish(&StopReason::Cancelled, at(3));
    assert_eq!(t.time(0), Some(at(0)));
    assert_eq!(t.time(1), None);
}

#[test]
fn a_failed_reply_is_timed_and_retrying_asks_the_same_question_again() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&chunk("Hal"));
    t.fail("The chat connection to Fermix broke.", at(4));
    assert_eq!(t.time(2), Some(at(4)));
    assert!(t.can_retry());
    assert_eq!(t.retry(), Some("hi".into()));
    assert_eq!(
        t.entries(),
        [Entry::User("hi".into())],
        "the half reply and the failure make way for the new reply"
    );
    assert_eq!(t.time(0), Some(at(0)));
    assert_eq!(t.phase(), Phase::Waiting);
    assert!(!t.can_retry());
    assert_eq!(t.retry(), None, "one retry at a time");
}

#[test]
fn only_a_failure_that_ends_the_conversation_can_be_retried() {
    let mut t = Transcript::default();
    assert!(!t.can_retry());
    assert_eq!(t.retry(), None);
    t.send("hi", at(0));
    t.fail("Fermix could not answer.", at(1));
    t.send("again", at(2));
    assert!(!t.can_retry(), "a newer question replaced it");
    t.finish(&StopReason::EndTurn, at(3));
    assert!(!t.can_retry());
}

#[test]
fn tool_states_read_as_words() {
    use fermix_client::chat::tool_state;
    assert_eq!(tool_state(ToolStatus::Running), "running");
    assert_eq!(tool_state(ToolStatus::Completed), "done");
    assert_eq!(tool_state(ToolStatus::Failed), "failed");
}

#[test]
fn a_picture_is_drawn_no_wider_than_fits_its_box_and_never_enlarged() {
    use fermix_client::chat::picture_width;
    assert_eq!(picture_width(800, 400, 440), 440, "wide: the box's width");
    assert_eq!(picture_width(400, 800, 440), 220, "tall: the box's height");
    assert_eq!(picture_width(100, 50, 440), 100, "small: its own size");
    assert_eq!(picture_width(1, 10_000, 440), 1, "never nothing");
}

#[test]
#[should_panic(expected = "has no size")]
fn a_picture_without_a_size_is_a_bug() {
    fermix_client::chat::picture_width(0, 10, 440);
}

#[test]
fn a_saved_picture_is_named_for_its_type() {
    use fermix_client::chat::picture_file_name;
    assert_eq!(picture_file_name("image/png"), "Fermix image.png");
    assert_eq!(picture_file_name("image/jpeg"), "Fermix image.jpg");
    assert_eq!(picture_file_name("image/svg+xml"), "Fermix image.svg");
    assert_eq!(picture_file_name("image/webp"), "Fermix image.webp");
    assert_eq!(
        picture_file_name("image/../../x"),
        "Fermix image",
        "a type that is not a plain word names no extension"
    );
}
