//! The conversation the Chat page draws: what was asked, what streamed back,
//! which tools ran, and whether a reply is still coming.

use fermix_client::acp::{StopReason, ToolKind, ToolStatus, Update};
use fermix_client::chat::{Entry, Phase, RunState, ToolRun, Transcript};

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
    t.apply(&chunk("Hel"), at(1));
    assert_eq!(t.phase(), Phase::Streaming);
    t.apply(&chunk("lo."), at(1));
    t.apply(&call("t1", "web_search", ToolKind::Fetch), at(2));
    t.apply(&ended("t1", ToolStatus::Completed), at(3));
    t.apply(&chunk("Found it."), at(4));
    assert_eq!(
        t.entries(),
        [
            Entry::User("hi".into()),
            Entry::Assistant("Hello.".into()),
            Entry::Tools(vec![run(
                "t1",
                "web_search",
                ToolKind::Fetch,
                RunState::Done,
                2,
                Some(3)
            )]),
            Entry::Assistant("Found it.".into()),
        ]
    );
}

#[test]
fn tools_in_a_row_fold_into_one_group_until_text_comes() {
    let mut t = Transcript::default();
    t.send("plan my day", at(0));
    t.apply(&call("t1", "web_search", ToolKind::Fetch), at(1));
    t.apply(&call("t2", "shell", ToolKind::Execute), at(2));
    t.apply(&ended("t1", ToolStatus::Completed), at(3));
    assert!(
        t.apply(&ended("t2", ToolStatus::Failed), at(5)),
        "an update finds its run inside the group"
    );
    t.apply(&chunk("Here is your day."), at(6));
    t.apply(&call("t3", "file_read", ToolKind::Read), at(7));
    assert_eq!(
        t.entries(),
        [
            Entry::User("plan my day".into()),
            Entry::Tools(vec![
                run(
                    "t1",
                    "web_search",
                    ToolKind::Fetch,
                    RunState::Done,
                    1,
                    Some(3)
                ),
                run(
                    "t2",
                    "shell",
                    ToolKind::Execute,
                    RunState::Failed,
                    2,
                    Some(5)
                ),
            ]),
            Entry::Assistant("Here is your day.".into()),
            Entry::Tools(vec![run(
                "t3",
                "file_read",
                ToolKind::Read,
                RunState::Running,
                7,
                None
            )]),
        ]
    );
}

#[test]
fn an_update_for_a_tool_nobody_announced_changes_nothing() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    let shown = t.apply(&ended("ghost", ToolStatus::Failed), at(1));
    assert!(!shown, "the page logs what it could not place");
    assert_eq!(t.entries(), [Entry::User("hi".into())]);
}

#[test]
fn an_update_this_app_does_not_know_is_reported_as_not_shown() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    assert!(!t.apply(&Update::Other("plan".into()), at(1)));
    assert!(
        t.apply(&chunk(""), at(1)),
        "an empty chunk is understood, only empty"
    );
    assert!(t.apply(&chunk("Hello"), at(1)));
    assert_eq!(t.entries().len(), 2);
}

#[test]
fn a_reply_cut_short_says_why_in_a_note() {
    for (reason, note) in [
        ("max_tokens", "The reply ran out of room."),
        ("refusal", "Fermix declined to answer this."),
        ("max_turn_requests", "The reply ended early."),
    ] {
        let mut t = Transcript::default();
        t.send("essay", at(0));
        t.apply(&chunk("Once"), at(1));
        t.finish(&StopReason::Other(reason.into()), at(1));
        assert_eq!(
            t.entries().last(),
            Some(&Entry::Notice(note.into())),
            "{reason}"
        );
    }
}

#[test]
fn a_finished_reply_frees_the_composer() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&chunk("Hello"), at(1));
    t.finish(&StopReason::EndTurn, at(1));
    assert_eq!(t.phase(), Phase::Idle);
    assert!(t.can_send());
    assert_eq!(t.entries().len(), 2);
}

#[test]
fn stopping_marks_the_reply_as_stopped() {
    let mut t = Transcript::default();
    t.send("write an essay", at(0));
    t.apply(&chunk("Once"), at(1));
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
fn a_tool_still_running_when_the_reply_ends_says_how_it_ended() {
    let cases = [
        (Some(StopReason::Cancelled), RunState::Stopped),
        (Some(StopReason::EndTurn), RunState::Unfinished),
        (None, RunState::Unfinished),
    ];
    for (reason, state) in cases {
        let mut t = Transcript::default();
        t.send("hi", at(0));
        t.apply(&call("t1", "shell", ToolKind::Execute), at(1));
        t.apply(&call("t2", "web_search", ToolKind::Fetch), at(2));
        t.apply(&ended("t2", ToolStatus::Completed), at(3));
        match &reason {
            Some(reason) => t.finish(reason, at(4)),
            None => t.fail("The chat connection to Fermix broke.", at(4)),
        }
        let Entry::Tools(runs) = &t.entries()[1] else {
            panic!("the tools are one group: {:?}", t.entries());
        };
        assert_eq!(
            (runs[0].state, runs[0].ended),
            (state, Some(at(4))),
            "{reason:?}"
        );
        assert_eq!(
            runs[1].state,
            RunState::Done,
            "a finished run keeps its end"
        );
    }
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
        "Fermix could not answer. The reason is in Logs."
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

fn call(id: &str, name: &str, kind: ToolKind) -> Update {
    Update::ToolCall {
        id: id.into(),
        title: name.into(),
        kind,
        status: ToolStatus::Running,
    }
}

fn running(id: &str) -> Update {
    call(id, "web_search", ToolKind::Fetch)
}

fn ended(id: &str, status: ToolStatus) -> Update {
    Update::ToolUpdate {
        id: id.into(),
        status,
    }
}

fn run(
    id: &str,
    name: &str,
    kind: ToolKind,
    state: RunState,
    started: i64,
    ended: Option<i64>,
) -> ToolRun {
    ToolRun {
        id: id.into(),
        name: name.into(),
        kind,
        state,
        started: at(started),
        ended: ended.map(at),
    }
}

#[test]
fn thoughts_gather_into_one_entry_ahead_of_the_reply() {
    let mut t = Transcript::default();
    t.send("hi", at(0));
    t.apply(&Update::ThoughtChunk("Look".into()), at(1));
    t.apply(&Update::ThoughtChunk("ing it up.".into()), at(1));
    assert_eq!(t.phase(), Phase::Streaming);
    t.apply(&chunk("Found it."), at(1));
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
    t.apply(&chunk("Here:"), at(1));
    t.apply(&Update::Image(picture()), at(1));
    t.apply(&chunk("Like it?"), at(1));
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
    t.apply(&Update::Attachment("shot.png".into()), at(1));
    t.apply(&chunk("Sent."), at(1));
    assert_eq!(t.entries()[1], Entry::Attachment("shot.png".into()));
    assert_eq!(t.entries()[2], Entry::Assistant("Sent.".into()));
}

#[test]
fn the_orb_shows_while_fermix_works_and_nothing_else_on_screen_moves() {
    let mut t = Transcript::default();
    assert!(!t.thinking(), "nothing asked yet");
    t.send("hi", at(0));
    assert!(t.thinking(), "asked, nothing back");
    t.apply(&Update::ThoughtChunk("Hmm".into()), at(1));
    assert!(
        !t.thinking(),
        "a live thought carries the orb in its own title, so there is one signal"
    );
    t.apply(&running("t1"), at(1));
    assert!(
        !t.thinking(),
        "a running tool's group carries the live line"
    );
    t.apply(&ended("t1", ToolStatus::Completed), at(1));
    assert!(
        !t.thinking(),
        "so does a group whose tools are done, until text comes"
    );
    t.apply(&chunk("Hel"), at(1));
    assert!(!t.thinking(), "the text streaming in shows the work");
    t.apply(&Update::Image(picture()), at(1));
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
    t.apply(&chunk("Checking."), at(1));
    t.apply(&running("t1"), at(1));
    t.apply(&chunk("Done."), at(1));
    assert_eq!(t.time(1), None, "a reply is timed once it ends");
    assert_eq!(t.time(3), None);
    t.finish(&StopReason::EndTurn, at(70));
    assert_eq!(t.time(1), None, "only the end of the reply");
    assert_eq!(t.time(2), None);
    assert_eq!(t.time(3), Some(at(70)));
    assert_eq!(t.time(4), None, "past the end");
}

#[test]
fn a_reply_in_the_same_minute_as_its_question_repeats_no_time() {
    let mut t = Transcript::default();
    // at(0) is 20 s into its minute: at(39) is the same minute, at(40) the next.
    t.send("hi", at(0));
    t.apply(&chunk("Hello."), at(1));
    t.finish(&StopReason::EndTurn, at(39));
    assert_eq!(t.time(1), None, "the question's time already says it");
    t.send("and now?", at(40));
    t.apply(&chunk("Still here."), at(41));
    t.finish(&StopReason::EndTurn, at(100));
    assert_eq!(t.time(3), Some(at(100)), "a minute on, the reply says when");
}

#[test]
fn a_turn_counts_from_its_question_while_it_runs() {
    let mut t = Transcript::default();
    assert_eq!(t.turn_started(), None);
    t.send("hi", at(5));
    assert_eq!(t.turn_started(), Some(at(5)));
    t.apply(&chunk("Hel"), at(6));
    assert_eq!(t.turn_started(), Some(at(5)));
    t.fail("Fermix could not answer.", at(7));
    assert_eq!(t.turn_started(), None, "nothing runs");
    t.retry();
    assert_eq!(
        t.turn_started(),
        Some(at(5)),
        "a retry is the same question"
    );
}

#[test]
fn a_stopped_reply_is_timed_above_its_note() {
    let mut t = Transcript::default();
    t.send("essay", at(0));
    t.apply(&chunk("Once"), at(1));
    t.stop();
    t.finish(&StopReason::Cancelled, at(65));
    assert_eq!(t.entries()[2], Entry::Notice("Stopped".into()));
    assert_eq!(t.time(1), Some(at(65)));
    assert_eq!(t.time(2), None);
}

#[test]
fn only_a_bubble_carries_the_time_a_reply_ended() {
    let mut t = Transcript::default();
    t.send("think", at(0));
    t.apply(&Update::ThoughtChunk("Hmm".into()), at(1));
    t.stop();
    t.finish(&StopReason::Cancelled, at(4));
    assert_eq!(t.time(1), None, "a thought is a caption, not a bubble");
    let mut t = Transcript::default();
    t.send("search", at(0));
    t.apply(&running("t1"), at(1));
    t.finish(&StopReason::EndTurn, at(4));
    assert_eq!(t.time(1), None, "a tool row is a caption too");
    let mut t = Transcript::default();
    t.send("draw", at(0));
    t.apply(&Update::Image(picture()), at(1));
    t.finish(&StopReason::EndTurn, at(64));
    assert_eq!(t.time(1), Some(at(64)), "a picture is a bubble");
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
    t.apply(&chunk("Hal"), at(1));
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
