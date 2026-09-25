//! The conversation the Chat page draws: what was asked, what streamed back,
//! which tools ran, and whether a reply is still coming.

use fermix_client::acp::{StopReason, ToolStatus, Update};
use fermix_client::chat::{tool_label, Entry, Phase, Transcript};

fn chunk(text: &str) -> Update {
    Update::MessageChunk(text.into())
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
    assert!(t.send("  What is on my calendar?  "));
    assert_eq!(t.entries(), [Entry::User("What is on my calendar?".into())]);
    assert_eq!(t.phase(), Phase::Waiting);
    assert!(!t.can_send(), "one reply at a time");
    assert!(
        !t.send("again"),
        "a second question waits for the first reply"
    );
}

#[test]
fn a_blank_message_is_never_sent() {
    let mut t = Transcript::default();
    assert!(!t.send("   \n "));
    assert!(t.entries().is_empty());
}

#[test]
fn chunks_join_into_one_reply_until_a_tool_runs_between_them() {
    let mut t = Transcript::default();
    t.send("hi");
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
    t.send("hi");
    t.apply(&Update::ToolUpdate {
        id: "ghost".into(),
        status: ToolStatus::Failed,
    });
    assert_eq!(t.entries(), [Entry::User("hi".into())]);
}

#[test]
fn a_finished_reply_frees_the_composer() {
    let mut t = Transcript::default();
    t.send("hi");
    t.apply(&chunk("Hello"));
    t.finish(&StopReason::EndTurn);
    assert_eq!(t.phase(), Phase::Idle);
    assert!(t.can_send());
    assert_eq!(t.entries().len(), 2);
}

#[test]
fn stopping_marks_the_reply_as_stopped() {
    let mut t = Transcript::default();
    t.send("write an essay");
    t.apply(&chunk("Once"));
    assert!(t.stop());
    assert_eq!(t.phase(), Phase::Stopping);
    assert!(!t.stop(), "a second stop sends nothing");
    t.finish(&StopReason::Cancelled);
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
    t.send("hi");
    t.fail("Fermix could not answer.");
    assert_eq!(t.phase(), Phase::Idle);
    assert_eq!(
        t.entries().last(),
        Some(&Entry::Failure("Fermix could not answer.".into()))
    );
}

#[test]
fn a_tool_still_running_when_the_reply_ends_is_left_as_it_was_last_seen() {
    let mut t = Transcript::default();
    t.send("hi");
    t.apply(&Update::ToolCall {
        id: "t1".into(),
        title: "shell".into(),
        status: ToolStatus::Running,
    });
    t.finish(&StopReason::Cancelled);
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
    t.send("hi");
    t.finish(&fermix_client::acp::StopReason::EndTurn);
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
