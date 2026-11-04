//! The Always On pane as a renderer of the engine's summary (Always On setup §3, §8.4): each
//! screen is the summary as the engine words it, every press is one `home.act` with the key it
//! was drawn with and what the action `needs`, its `confirm` is asked first, the answer is drawn,
//! and the app keeps only how it asks for the recovery passphrase and that it restarts after its
//! own Move now. The summaries are the engine's own goldens, and the screens no golden holds are
//! built as the engine builds them, in its words for a Linux computer.

use fermix_client::always_on::model::{Act, Dialog, Model, Press};
use fermix_client::always_on::notices::{notices_file, Posted, POSTED};
use fermix_client::always_on::{
    destructive, listed, offered, screen, served as served_by, Fact, Page, LINK_FAILED, NEEDED,
    RESTARTING,
};
use fermix_client::home::{
    Acted, Action, ActionId, HomeStatus, Input, Line, Notice, Passphrase, Summary, SummaryState,
    PASSPHRASE_MISMATCH,
};
use fermix_client::management::{CallError, Refusal};
use fermix_client::view::DaemonProblem;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const CHANGED: &str = "Always On changed since this was shown, so look at it again.";

fn result(name: &str) -> Value {
    SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no golden named {name}"))["response"]["result"]
        .clone()
}

fn decoded<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap_or_else(|e| panic!("decodes: {e}"))
}

fn status(name: &str) -> HomeStatus {
    decoded(result(name))
}

fn summary(name: &str) -> Summary {
    status(name).summary
}

/// A status whose summary is `summary`, as `home.status` answers it.
fn answering(summary: Value) -> HomeStatus {
    decoded(json!({"role": "primary", "summary": summary}))
}

/// What `home.act` answers: `summary` after the step, and its result.
fn acted(summary: Value, result: Value) -> Acted {
    decoded(json!({"summary": summary, "result": result}))
}

/// Every method the engine serves, as its `hello` lists them.
fn served() -> Vec<String> {
    result("hello")["capabilities"]["methods"]
        .as_array()
        .expect("methods")
        .iter()
        .map(|m| m.as_str().expect("a name").to_owned())
        .collect()
}

fn record(id: &str, label: &str, needs: Value, confirm: Value, key: &str) -> Value {
    json!({"id": id, "label": label, "needs": needs, "confirm": confirm, "url": null, "key": key})
}

/// The key the engine draws Use my own Linux server with, on `off` and `connect` alike.
const OWN_KEY: &str = "Mx_U6542oONDPqGb3qliTQ";
/// Cancel the code's key, as the request fixture sends it.
const CANCEL_CODE_KEY: &str = "PGFKmgUa3i48MJgwQSKf-g";
/// The command `own_server` answers: the installer at this engine's own version.
const COMMAND: &str = "curl -fsSL https://fermix.ai/always-on | sudo sh -s -- --version 1.2.3";

/// `off`, as the engine answers Cancel the code: its one action a server of the owner's own.
fn off() -> Value {
    result("home_act")["summary"].clone()
}

/// `connect` while a code for a server of the owner's own is open, as the engine builds it:
/// nothing to press while the server uses the code, and a new code or Cancel the code beside it.
fn connect() -> Value {
    json!({"action": null,
        "detail": {"alternatives": [
            record("own_server", "Use my own Linux server", json!([]), Value::Null, OWN_KEY),
            record("cancel", "Cancel the code", json!([]), Value::Null, CANCEL_CODE_KEY)]},
        "key": "1beYgzYKYRJoEBvPpLz31A", "notice": null, "outcome": "waiting",
        "sentence": "Waiting for your server: run the command on it and paste the code when it asks.",
        "state": "connect"})
}

/// A model that read `summary` with the pane shown, every read answered.
fn model_on(summary: Value) -> Model {
    let mut model = Model::default();
    model.pane_shown();
    assert!(model.take_read());
    model.served(served());
    model.answered(answering(summary));
    model
}

fn screen_of(model: &Model) -> fermix_client::always_on::Screen {
    match model.page() {
        Page::Screen(screen) => *screen,
        other => panic!("a screen, not {other:?}"),
    }
}

fn ids(actions: &[Action]) -> Vec<ActionId> {
    actions.iter().map(|a| a.id).collect()
}

fn typed(text: &str) -> Passphrase {
    Passphrase::typed(text).expect("typed")
}

/// The daemon's refusal of an act, in its words.
fn refusal(code: &str, sentence: &str) -> CallError {
    CallError::Refused(Refusal {
        code: code.into(),
        sentence: sentence.into(),
        field: None,
        reason: None,
    })
}

fn act(action: ActionId, key: &str, input: Input) -> Press {
    Press::Run(Act {
        action,
        key: key.into(),
        input,
    })
}

// ---- The screens are the summary --------------------------------------------------------------

#[test]
fn every_golden_draws_its_sentence_and_its_one_action_in_the_engines_words() {
    for name in [
        "home_status",
        "home_status_creating",
        "home_status_ready",
        "home_status_ready_first_move",
        "home_status_candidate",
        "home_status_server",
        "home_status_on",
        "home_status_problem",
    ] {
        let summary = summary(name);
        let shown = screen(&summary);
        assert_eq!(shown.state, summary.state, "{name}");
        assert_eq!(shown.sentence, summary.sentence, "{name}");
        assert_eq!(shown.action, summary.action, "{name}");
    }
    let undone = screen(&summary("home_status"));
    let try_again = undone.action.expect("Try again");
    assert_eq!(
        (try_again.id, try_again.label.as_str()),
        (ActionId::TryAgain, "Try again")
    );
    assert!(try_again
        .confirm
        .expect("asked first")
        .starts_with("Move Fermix to your server now?"));
}

#[test]
fn details_are_the_engines_lines_and_sentences_in_its_order() {
    let ready = screen(&summary("home_status_ready"));
    let labels: Vec<&str> = ready
        .details
        .iter()
        .map(|fact| match fact {
            Fact::Line(line) => line.label.as_str(),
            Fact::Sentence(said) => said.as_str(),
        })
        .collect();
    assert_eq!(labels, ["Server", "System", "Version", "Waits until"]);
    let on = screen(&summary("home_status_on"));
    assert_eq!(on.details.len(), 6, "five lines and what lives there");
    assert_eq!(
        on.details.first(),
        Some(&Fact::Line(Line {
            label: "Server".into(),
            value: "fermix-general-a3f9".into()
        }))
    );
    let Some(Fact::Sentence(data)) = on.details.last() else {
        panic!("what lives on the server, last")
    };
    assert!(data.starts_with("Your conversations and memories"));
    assert!(data.contains("whoever hosts it"), "{data}");
    for quiet in [
        "home_status_creating",
        "home_status_candidate",
        "home_status_server",
        "home_status",
    ] {
        assert!(screen(&summary(quiet)).details.is_empty(), "{quiet}");
    }
}

/// The owner's server connecting and a restore running each show the bar, with the step in hand
/// where the engine names one; nothing is pressed meanwhile.
#[test]
fn a_running_step_shows_the_bar_and_the_engines_progress() {
    let connecting = screen(&summary("home_status_creating"));
    assert_eq!(connecting.sentence, "Connecting your server…");
    assert!(connecting.shows_progress && connecting.action.is_none());
    assert!(connecting.others.is_empty());
    let restoring = screen(&decoded(restore_moving()));
    assert!(restoring.shows_progress && restoring.action.is_none());
    assert_eq!(
        restoring.progress.as_deref(),
        Some("Restoring Fermix from the backup on this computer…")
    );
    let ready = screen(&summary("home_status_ready"));
    assert!(!ready.shows_progress && ready.progress.is_none());
}

/// Always On off offers one thing, a Linux server of the owner's own; while its code is open the
/// screen waits for that server with nothing to press, a new code or Cancel the code beside it.
#[test]
fn off_offers_a_server_of_the_owners_own_and_connect_waits_for_it_with_nothing_to_press() {
    let off = screen(&decoded(off()));
    assert_eq!(off.state, SummaryState::Off);
    assert_eq!(
        off.sentence,
        "Keep Fermix running when this Mac sleeps. It moves to a server you own."
    );
    let own = off.action.expect("Use my own Linux server");
    assert_eq!(
        (own.id, own.label.as_str(), own.key.as_str()),
        (ActionId::OwnServer, "Use my own Linux server", OWN_KEY)
    );
    assert!(own.needs.is_empty() && own.confirm.is_none());
    assert!(off.others.is_empty() && off.details.is_empty());
    let mut model = model_on(connect());
    let waiting = screen_of(&model);
    assert_eq!(waiting.state, SummaryState::Connect);
    assert_eq!(
        waiting.sentence,
        "Waiting for your server: run the command on it and paste the code when it asks."
    );
    assert!(waiting.action.is_none(), "nothing to press while it waits");
    assert!(!waiting.shows_progress);
    assert_eq!(
        ids(&waiting.others),
        [ActionId::OwnServer, ActionId::Cancel]
    );
    let cancel = waiting.others[1].clone();
    assert_eq!(
        model.press(&cancel),
        act(ActionId::Cancel, CANCEL_CODE_KEY, Input::none()),
        "Cancel the code asks nothing first"
    );
}

#[test]
fn ready_lists_the_reviews_items_and_on_offers_its_other_actions() {
    let ready = screen(&summary("home_status_ready"));
    let items: Vec<&str> = ready
        .readiness
        .iter()
        .map(|i| i.sentence.as_str())
        .collect();
    assert_eq!(
        items,
        ["iMessage, voice calls and meetings stop after the move: neither this Mac nor fermix-general-a3f9 runs them."]
    );
    assert!(
        ready.others.is_empty(),
        "a server waiting for its move offers Move now alone"
    );
    let on = screen(&summary("home_status_on"));
    assert!(on.action.is_none(), "all is well: no main action");
    assert_eq!(ids(&on.others), [ActionId::BackUpNow, ActionId::MoveBack]);
}

/// A server out of reach offers Learn more, Restore here beside it and the copy's date; a restore
/// that stopped offers Restore here again and says why in the engine's note.
#[test]
fn a_problem_draws_its_action_its_alternatives_and_its_note() {
    let away = screen(&summary("home_status_problem"));
    assert_eq!(away.state, SummaryState::Problem);
    assert_eq!(
        away.action.as_ref().map(|a| a.id),
        Some(ActionId::LearnMore)
    );
    assert_eq!(ids(&away.others), [ActionId::RestoreHere]);
    assert_eq!(
        away.details,
        [Fact::Line(Line {
            label: "Backup on this Mac".into(),
            value: "Sunday, May 4, 2031 at 12:00 PM CEST".into()
        })]
    );
    let stopped = screen(&decoded(restore_failed()));
    assert_eq!(
        stopped.sentence,
        "Fermix could not restore its backup on this computer."
    );
    assert_eq!(
        stopped.action.as_ref().map(|a| a.id),
        Some(ActionId::RestoreHere)
    );
    assert!(stopped.others.is_empty());
    assert_eq!(
        stopped.details,
        [
            Fact::Sentence(RESTORE_STOPPED.into()),
            Fact::Line(Line {
                label: "Backup on this computer".into(),
                value: "Sunday, May 4, 2031 at 12:00 PM CEST".into()
            }),
        ]
    );
}

/// A button this app cannot ask for would do nothing: an id it does not know, a need it does
/// not know, or more than one need at once.
#[test]
fn an_action_this_app_cannot_ask_for_draws_nothing() {
    let later = |id: &str, needs: Value| {
        decoded::<Action>(record(id, "Later", needs, Value::Null, "k-later"))
    };
    assert!(!offered(&later("fly_away", json!([]))));
    assert!(!offered(&later("move_now", json!(["fingerprint"]))));
    assert!(!offered(&later(
        "move_now",
        json!(["recovery_passphrase", "new_recovery_passphrase"])
    )));
    assert!(offered(&later("move_now", json!(["recovery_passphrase"]))));
    let mut on = summary("home_status_on");
    on.detail.actions.push(later("fly_away", json!([])));
    on.action = Some(later("fly_away", json!([])));
    let shown = screen(&on);
    assert!(shown.action.is_none());
    assert_eq!(
        ids(&shown.others),
        [ActionId::BackUpNow, ActionId::MoveBack]
    );
}

#[test]
fn every_state_the_engine_answers_draws_its_sentence() {
    let schema: Value =
        serde_json::from_str(include_str!("../contracts/management/protocol.schema.json"))
            .expect("the schema is JSON");
    let states = schema["$defs"]["homeSummary"]["properties"]["state"]["enum"]
        .as_array()
        .expect("the states")
        .clone();
    assert_eq!(states.len(), 11);
    for state in states {
        let shown = screen(&decoded(
            json!({"state": state, "sentence": "Where it stands.",
            "action": null, "detail": {}, "key": "k", "notice": null}),
        ));
        assert_ne!(shown.state, SummaryState::Unknown, "{state}");
        assert_eq!(shown.sentence, "Where it stands.");
    }
}

// ---- What the pane shows as a whole -----------------------------------------------------------

#[test]
fn an_engine_without_home_act_is_told_to_update() {
    assert_eq!(NEEDED, ["home.status", "home.act"]);
    let mut model = model_on(off());
    assert!(matches!(model.page(), Page::Screen(_)));
    let older: Vec<String> = served().into_iter().filter(|m| m != "home.act").collect();
    model.served(older);
    assert_eq!(model.page(), Page::NeedsUpdate);
    let mut fresh = Model::default();
    assert_eq!(fresh.page(), Page::Reading);
    fresh.unanswered(DaemonProblem::UpdateNeeded(
        "Fermix is older than this app.".into(),
    ));
    assert_eq!(fresh.page(), Page::NeedsUpdate);
}

/// Fermix down keeps the last screen and adds the sentence; with nothing read yet the pane says
/// why on its own page.
#[test]
fn fermix_down_keeps_the_last_screen_and_adds_its_sentence() {
    let mut fresh = Model::default();
    fresh.unanswered(DaemonProblem::NotRunning);
    assert_eq!(fresh.page(), Page::Unreadable(DaemonProblem::NotRunning));
    assert_eq!(fresh.read_note(), None);
    let ready = result("home_status_ready")["summary"].clone();
    let mut model = model_on(ready.clone());
    model.unanswered(DaemonProblem::NotRunning);
    let kept = screen_of(&model);
    assert_eq!(kept.state, SummaryState::Ready);
    assert_eq!(model.read_note().as_deref(), Some("Fermix is not running."));
    model.answered(answering(ready));
    assert_eq!(model.read_note(), None);
}

// ---- When the pane reads ------------------------------------------------------------------------

#[test]
fn nothing_is_read_until_told_and_each_notice_is_one_read() {
    let mut model = Model::default();
    assert!(!model.take_read(), "nothing at launch");
    model.home_status_changed();
    model.home_status_changed();
    assert!(model.take_read());
    assert!(!model.take_read(), "two notices meanwhile are one read");
    model.engine_restarted();
    assert!(!model.take_read(), "a new engine is read only while shown");
    model.pane_shown();
    assert!(model.take_read());
    model.served(served());
    model.answered(answering(off()));
    assert!(!model.take_read(), "off reads nothing more");
    model.engine_restarted();
    assert!(model.take_read());
    assert!(!model.take_read());
}

// ---- Every press is one home.act --------------------------------------------------------------

/// The engine serves Always On where `hello` lists `home.act`: a release build lists none of its
/// methods, and the pane is not listed there.
#[test]
fn always_on_is_listed_only_where_the_engine_serves_home_act() {
    assert!(served_by(&served()));
    let release: Vec<String> = served()
        .into_iter()
        .filter(|m| !["home.status", "home.act"].contains(&m.as_str()))
        .collect();
    assert!(!served_by(&release));
    let panes = vec!["providers", "always_on", "secrets"];
    assert_eq!(listed(panes.clone(), true), panes);
    assert_eq!(listed(panes, false), ["providers", "secrets"]);
}

#[test]
fn a_press_with_nothing_to_ask_acts_at_once_with_the_key_it_was_drawn_with() {
    let mut model = model_on(result("home_status_on")["summary"].clone());
    let back_up = screen_of(&model).others[0].clone();
    assert_eq!(back_up.id, ActionId::BackUpNow);
    assert_eq!(
        model.press(&back_up),
        act(ActionId::BackUpNow, "HAtkdLOslkPTtZfhMrpeQg", Input::none())
    );
    assert_eq!(model.working, Some(ActionId::BackUpNow));
    assert_eq!(model.press(&back_up), Press::Ignored, "one act at a time");
    let answered = acted(result("home_status_on")["summary"].clone(), Value::Null);
    model.acted(Ok(answered));
    assert_eq!(model.working, None);
    assert!(model.take_read(), "read again after the act");
}

/// The contract lets any action carry a confirm, asked before anything is sent: one that needs
/// nothing is asked in a dialog of its own, and declining it sends nothing.
#[test]
fn a_confirm_is_asked_first_and_declining_it_sends_nothing() {
    let mut asked = connect();
    asked["detail"]["alternatives"][1]["confirm"] =
        json!("Cancel the code? Your server cannot use it after this.");
    let mut model = model_on(asked);
    let cancel = screen_of(&model).others[1].clone();
    assert_eq!(cancel.id, ActionId::Cancel);
    assert_eq!(model.press(&cancel), Press::Drawn);
    assert_eq!(model.dialog(), Some(Dialog::Confirm(cancel.clone())));
    model.dismissed();
    assert_eq!(model.dialog(), None);
    assert_eq!(model.working, None);
    model.press(&cancel);
    assert_eq!(
        model.confirmed(),
        act(ActionId::Cancel, CANCEL_CODE_KEY, Input::none())
    );
    assert_eq!(model.dialog(), None);
}

/// Restore here alone loses something for good, all that came after the copy it puts back, so
/// it alone never takes Return for yes and its answer is drawn as one that deletes; Cancel
/// deletes no server any more.
#[test]
fn restore_here_alone_never_takes_return_for_yes() {
    assert!(destructive(ActionId::RestoreHere));
    for id in [
        ActionId::Cancel,
        ActionId::MoveNow,
        ActionId::TryAgain,
        ActionId::BackUpNow,
        ActionId::CheckAgain,
        ActionId::OwnServer,
        ActionId::MoveBack,
    ] {
        assert!(!destructive(id), "{id:?}");
    }
}

#[test]
fn move_now_asks_the_passphrase_once_where_one_is_set_and_twice_where_none_is() {
    let mut model = model_on(result("home_status_ready")["summary"].clone());
    let move_now = screen_of(&model).action.expect("Move now");
    assert_eq!(model.press(&move_now), Press::Drawn);
    assert_eq!(
        model.dialog(),
        Some(Dialog::Passphrase {
            action: move_now.clone(),
            twice: false
        })
    );
    assert_eq!(model.passphrase_entered("", None), Press::Ignored);
    assert_eq!(
        model.passphrase_entered("invented-harbor-juniper-lantern", None),
        act(
            ActionId::MoveNow,
            "0Unu8V_R8dFwxCLgUqa1fw",
            Input::passphrase(typed("invented-harbor-juniper-lantern"))
        ),
        "the dialog shows the confirm, so its answer accepts it"
    );
    let mut first = model_on(result("home_status_ready_first_move")["summary"].clone());
    let move_now = screen_of(&first).action.expect("Move now");
    first.press(&move_now);
    assert_eq!(
        first.dialog(),
        Some(Dialog::Passphrase {
            action: move_now.clone(),
            twice: true
        })
    );
    assert_eq!(
        first.passphrase_entered("invented-harbor-juniper", Some("invented-harbor-juniper-x")),
        Press::Ignored
    );
    assert_eq!(
        first.passphrase_refusal.as_deref(),
        Some(PASSPHRASE_MISMATCH)
    );
    assert_eq!(
        first.passphrase_entered("invented-harbor-juniper", Some("invented-harbor-juniper")),
        act(
            ActionId::MoveNow,
            "2AqtcHJopcwn24x76cpKEQ",
            Input::passphrase(typed("invented-harbor-juniper"))
        )
    );
    assert_eq!(first.passphrase_refusal, None);
}

/// After its own Move now the engine restarts into the move: the pane says it restarts until
/// the engine has been away and answers again, or says where the move stands.
#[test]
fn the_own_move_shows_restarting_until_the_engine_comes_back() {
    let ready = result("home_status_ready")["summary"].clone();
    let mut model = model_on(ready.clone());
    let move_now = screen_of(&model).action.expect("Move now");
    model.press(&move_now);
    model.passphrase_entered("invented-harbor-juniper-lantern", None);
    model.acted(Ok(acted(ready.clone(), json!({"move_id": "mv-0f1e2d3c"}))));
    let restarting = screen_of(&model);
    assert_eq!(restarting.sentence, RESTARTING);
    assert!(restarting.shows_progress && restarting.action.is_none());
    assert!(restarting.others.is_empty() && restarting.details.is_empty());
    model.answered(answering(ready.clone()));
    assert_eq!(
        screen_of(&model).sentence,
        RESTARTING,
        "the old engine, still"
    );
    model.unanswered(DaemonProblem::NotRunning);
    assert_eq!(screen_of(&model).sentence, RESTARTING);
    assert_eq!(model.read_note().as_deref(), Some("Fermix is not running."));
    model.answered(answering(ready.clone()));
    assert_eq!(
        screen_of(&model).state,
        SummaryState::Ready,
        "back after being away: drawn as it says"
    );
    let mut quick = model_on(ready.clone());
    quick.press(&move_now);
    quick.passphrase_entered("invented-harbor-juniper-lantern", None);
    quick.acted(Ok(acted(ready.clone(), json!({"move_id": "mv-0f1e2d3c"}))));
    quick.engine_restarted();
    quick.answered(answering(ready));
    assert_eq!(
        screen_of(&quick).state,
        SummaryState::Ready,
        "a new engine answered"
    );
}

#[test]
fn a_move_that_says_where_it_stands_is_drawn_as_it_says() {
    let moving = json!({"action": null, "detail": {}, "key": "DXMsAJ5oa8GcFfqMnPOUWQ",
        "notice": null, "state": "moving",
        "sentence": "Moving. This Mac restarts and reconnects to your server. Messages wait until it is done."});
    let mut model = model_on(result("home_status_ready")["summary"].clone());
    let move_now = screen_of(&model).action.expect("Move now");
    model.press(&move_now);
    model.passphrase_entered("invented-harbor-juniper-lantern", None);
    model.acted(Ok(acted(moving.clone(), json!({"move_id": "mv-0f1e2d3c"}))));
    assert_eq!(screen_of(&model).state, SummaryState::Moving);
    assert!(screen_of(&model).sentence.starts_with("Moving."));
}

/// A step refused `changed` says its sentence and reads again; what was asked for on a screen
/// goes once the summary no longer offers its action under that key, and stays while it does.
#[test]
fn changed_says_its_sentence_and_drops_what_was_asked_for_on_the_older_screen() {
    let ready = result("home_status_ready")["summary"].clone();
    let mut model = model_on(ready.clone());
    let move_now = screen_of(&model).action.expect("Move now");
    model.press(&move_now);
    model.passphrase_entered("invented-harbor-juniper-lantern", None);
    model.acted(Err(refusal("changed", CHANGED)));
    assert_eq!(model.refusal.as_deref(), Some(CHANGED));
    assert!(model.take_read());
    model.press(&move_now);
    assert!(model.dialog().is_some());
    let mut newer = ready.clone();
    newer["action"]["key"] = json!("a-newer-key");
    model.answered(answering(newer));
    assert_eq!(
        model.dialog(),
        None,
        "a dialog for a key no longer offered goes"
    );
    let mut same = model_on(ready.clone());
    same.press(&move_now);
    same.answered(answering(ready));
    assert!(same.dialog().is_some(), "the same screen keeps it");
}

/// Use my own Linux server answers the code once, with the command that installs Fermix at this
/// engine's version; they show until the code is closed or the pane goes, and a new code from
/// `connect` shows in its place.
#[test]
fn the_own_servers_code_shows_with_the_command_until_the_code_or_the_pane_goes() {
    let mut model = model_on(off());
    let own = screen_of(&model).action.expect("Use my own Linux server");
    assert_eq!(
        model.press(&own),
        act(ActionId::OwnServer, OWN_KEY, Input::none())
    );
    model.acted(Ok(acted(
        connect(),
        json!({"code": "fxenroll1.aW52ZW50ZWQtY29kZQ", "expires_at": "2031-05-04T12:30:00Z",
            "command": COMMAND}),
    )));
    let code = model.handout.clone().expect("the code, once");
    assert_eq!(code.code(), "fxenroll1.aW52ZW50ZWQtY29kZQ");
    assert_eq!(code.command, COMMAND);
    assert_eq!(code.expires_at, "2031-05-04T12:30:00Z");
    assert!(screen_of(&model).action.is_none(), "connect waits");
    model.answered(answering(connect()));
    assert!(model.handout.is_some(), "while the code is open");
    model.pane_hidden();
    assert_eq!(model.handout, None, "the code goes with the pane");
    model.pane_shown();
    model.take_read();
    model.answered(answering(connect()));
    let again = screen_of(&model).others[0].clone();
    assert_eq!(
        model.press(&again),
        act(ActionId::OwnServer, OWN_KEY, Input::none())
    );
    model.acted(Ok(acted(
        connect(),
        json!({"code": "fxenroll1.YW5vdGhlci1jb2Rl", "expires_at": "2031-05-04T12:40:00Z",
            "command": COMMAND}),
    )));
    assert_eq!(
        model.handout.as_ref().map(|h| h.code()),
        Some("fxenroll1.YW5vdGhlci1jb2Rl")
    );
    model.answered(answering(off()));
    assert_eq!(model.handout, None, "the code was closed: off says so");
}

#[test]
fn learn_more_opens_its_page_and_update_is_the_apps_own() {
    let mut model = model_on(result("home_status_problem")["summary"].clone());
    let learn = screen_of(&model).action.expect("Learn more");
    assert_eq!(
        model.press(&learn),
        Press::Open("https://fermix.ai/docs/always-on/".into())
    );
    assert_eq!(model.working, None, "nothing is sent");
    let mut nowhere = learn.clone();
    nowhere.url = None;
    assert_eq!(model.press(&nowhere), Press::Drawn, "no page to open");
    assert_eq!(model.refusal.as_deref(), Some(LINK_FAILED));
    let update: Action = decoded(record(
        "update",
        "Update",
        json!([]),
        Value::Null,
        "k-update",
    ));
    assert_eq!(model.press(&update), Press::Update);
}

// ---- Moving back --------------------------------------------------------------------------------

/// The move back's and the restore's screens on a Linux computer, as the engine words them (its
/// `Copy`).
const RETURN_CANCEL_KEY: &str = "k-return-cancel";
const RETURN_MOVE_KEY: &str = "k-return-move";
const RESTORE_AGAIN_KEY: &str = "k-restore-again";
/// What the engine notes when a restore stopped before its copy was in place.
const RESTORE_STOPPED: &str =
    "The backup on this computer could not be restored here; Fermix's log says why.";

fn return_cancel() -> Value {
    record(
        "cancel",
        "Cancel",
        json!([]),
        Value::Null,
        RETURN_CANCEL_KEY,
    )
}

fn learn_more() -> Value {
    json!({"confirm": null, "id": "learn_more", "key": "k-learn", "label": "Learn more",
        "needs": [], "url": "https://fermix.ai/docs/always-on/"})
}

fn connecting() -> Value {
    json!({"action": return_cancel(), "detail": {}, "key": "k-connecting", "notice": null,
        "sentence": "Connecting this computer to your server…", "state": "creating"})
}

fn return_ready() -> Value {
    json!({"action": record("move_now", "Move now", json!(["recovery_passphrase"]),
            json!("Move Fermix back to this computer now? Coding agents ask for your approval again. Messages sent to Telegram while it moves wait until it is done. Your server stays until you delete it."),
            RETURN_MOVE_KEY),
        "detail": {"alternatives": [return_cancel()], "readiness": [],
            "lines": [{"label": "Server", "value": "fermix-general-a3f9"},
                {"label": "Waits until", "value": "Monday, May 5, 2031 at 1:00 PM CEST"}]},
        "key": "k-return-ready", "notice": null,
        "sentence": "This computer is ready to take Fermix back.", "state": "ready"})
}

fn return_moving(progress: &str) -> Value {
    json!({"action": null, "detail": {"progress": progress}, "key": format!("k-{progress}"),
        "notice": null, "state": "moving",
        "sentence": "Moving Fermix back to this computer. Messages wait until it is done."})
}

/// Back on this computer as the primary after a move back: Always On is off here, and the
/// server's retired copy is told once.
fn back() -> Value {
    json!({"action": record("own_server", "Use my own Linux server", json!([]), Value::Null,
            OWN_KEY),
        "detail": {}, "key": "k-back", "outcome": "done", "state": "off",
        "notice": {"id": "back:5",
            "sentence": "Fermix is back on this computer. Your own server still runs its retired copy: turn it off or delete it."},
        "sentence": "Keep Fermix running when this computer sleeps. It moves to a server you own."})
}

fn return_failed() -> Value {
    json!({"action": record("move_back", "Move back to this computer", json!([]), Value::Null,
            "k-return-again"),
        "detail": {"alternatives": [learn_more()], "kind": "return_failed", "note": null,
            "seen_at": null},
        "key": "k-return-failed", "notice": null, "outcome": "failed", "state": "problem",
        "sentence": "This computer could not get ready to take Fermix back. Move back to try again."})
}

fn return_stopped() -> Value {
    json!({"action": return_cancel(), "detail": {"kind": "return_failed", "note": null,
            "seen_at": null},
        "key": "k-return-stopped", "notice": null, "outcome": "failed", "state": "problem",
        "sentence": "Moving back stopped part-way, so cancel it and move back again."})
}

fn return_undone() -> Value {
    json!({"action": record("try_again", "Try again", json!(["recovery_passphrase"]),
            json!("Move Fermix back to this computer now? Coding agents ask for your approval again. Messages sent to Telegram while it moves wait until it is done. Your server stays until you delete it."),
            "k-return-try"),
        "detail": {"alternatives": [return_cancel()]},
        "key": "k-return-undone", "notice": null, "state": "undone",
        "sentence": "The move back was undone. Fermix still runs on your server."})
}

fn restore_moving() -> Value {
    json!({"action": null,
        "detail": {"progress": "Restoring Fermix from the backup on this computer…"},
        "key": "k-restoring", "notice": null, "outcome": "running", "state": "moving",
        "sentence": "Restoring Fermix on this computer from its backup. Messages wait until it is done."})
}

fn restore_failed() -> Value {
    json!({"action": record("restore_here", "Restore on this computer",
            json!(["recovery_passphrase"]),
            json!("Restore Fermix on this computer from its backup? Confirm only once you deleted your server, or stopped it for good: if it runs again, two copies of Fermix answer the same channels. Messages, memory and jobs since that backup are gone, and Telegram keeps nothing older than 24 hours."),
            RESTORE_AGAIN_KEY),
        "detail": {"kind": "return_failed", "note": RESTORE_STOPPED, "seen_at": null,
            "lines": [{"label": "Backup on this computer",
                "value": "Sunday, May 4, 2031 at 12:00 PM CEST"}]},
        "key": "k-restore-failed", "notice": null, "outcome": "failed", "state": "problem",
        "sentence": "Fermix could not restore its backup on this computer."})
}

/// `on` offers Move back beside its other actions; its press is one act that asks nothing,
/// and the screen it answers is the engine's: connecting, with Cancel.
#[test]
fn move_back_is_one_press_asked_nothing_and_its_connecting_screen_offers_cancel() {
    let mut model = model_on(result("home_status_on")["summary"].clone());
    let move_back = screen_of(&model).others[1].clone();
    assert_eq!(move_back.id, ActionId::MoveBack);
    assert_eq!(
        model.press(&move_back),
        act(ActionId::MoveBack, "A8TBcMzBwNuJLnGg2E5uAw", Input::none()),
        "nothing moves at this press, so the engine asks nothing first"
    );
    assert_eq!(model.dialog(), None);
    model.acted(Ok(acted(connecting(), Value::Null)));
    let shown = screen_of(&model);
    assert_eq!(shown.state, SummaryState::Creating);
    assert_eq!(shown.sentence, "Connecting this computer to your server…");
    assert!(shown.shows_progress);
    let cancel = shown.action.expect("Cancel");
    assert_eq!(
        model.press(&cancel),
        act(ActionId::Cancel, RETURN_CANCEL_KEY, Input::none())
    );
}

/// Move now on a move back asks the passphrase once and sends it; this computer does not
/// restart at the press, so the engine's moving screen is drawn, never the app's restart line.
/// When it restarts as the primary, the last screen stays with the sentence until the new
/// engine answers, as for any restart, and then it is drawn as it says: Always On off here,
/// with the server's retired copy told once.
#[test]
fn a_move_back_moves_with_the_passphrase_and_its_restart_is_read_as_any_other() {
    let mut model = model_on(return_ready());
    let ready = screen_of(&model);
    assert_eq!(
        ready.sentence,
        "This computer is ready to take Fermix back."
    );
    assert_eq!(ids(&ready.others), [ActionId::Cancel]);
    let move_now = ready.action.expect("Move now");
    model.press(&move_now);
    assert_eq!(
        model.dialog(),
        Some(Dialog::Passphrase {
            action: move_now.clone(),
            twice: false
        })
    );
    assert_eq!(
        model.passphrase_entered("invented-harbor-juniper-lantern", None),
        act(
            ActionId::MoveNow,
            RETURN_MOVE_KEY,
            Input::passphrase(typed("invented-harbor-juniper-lantern"))
        )
    );
    model.acted(Ok(acted(
        return_moving("Receiving your Fermix from the server…"),
        json!({"move_id": "mv-back-7c1d"}),
    )));
    let moving = screen_of(&model);
    assert_ne!(moving.sentence, RESTARTING);
    assert_eq!(
        moving.sentence,
        "Moving Fermix back to this computer. Messages wait until it is done."
    );
    assert_eq!(
        moving.progress.as_deref(),
        Some("Receiving your Fermix from the server…")
    );
    assert!(moving.shows_progress && moving.action.is_none());
    assert!(model.take_read());
    model.answered(answering(return_moving(
        "Putting Fermix in place on this computer…",
    )));
    model.unanswered(DaemonProblem::NotRunning);
    let kept = screen_of(&model);
    assert_eq!(
        kept.progress.as_deref(),
        Some("Putting Fermix in place on this computer…")
    );
    assert_eq!(model.read_note().as_deref(), Some("Fermix is not running."));
    assert!(!model.take_read(), "read on no timer");
    model.engine_restarted();
    assert!(model.take_read());
    let told = model.answered(answering(back()));
    let home = screen_of(&model);
    assert_eq!(home.state, SummaryState::Off);
    assert_eq!(model.read_note(), None);
    assert_eq!(
        home.action.map(|a| a.id),
        Some(ActionId::OwnServer),
        "Always On is off here again"
    );
    assert_eq!(told.map(|n| n.id).as_deref(), Some("back:5"));
}

/// Restore here asks the passphrase once under the engine's confirm, which asks the owner's word
/// that the server is deleted or stopped for good, and sends it with the key it was drawn with,
/// beside the copy's date; a passphrase that opens no copy is said under the action, and the
/// restore runs on the engine's moving screen, never the app's restart line.
#[test]
fn restore_here_asks_the_passphrase_once_under_its_confirm_and_sends_it() {
    let mut model = model_on(result("home_status_problem")["summary"].clone());
    let shown = screen_of(&model);
    assert_eq!(ids(&shown.others), [ActionId::RestoreHere]);
    assert!(shown
        .details
        .iter()
        .any(|f| matches!(f, Fact::Line(l) if l.label == "Backup on this Mac")));
    let restore = shown.others[0].clone();
    assert!(
        restore.confirm.as_deref().is_some_and(|c| c.starts_with(
            "Restore Fermix on this Mac from its backup? Confirm only once you deleted your \
             server, or stopped it for good"
        )),
        "the confirm is the engine's"
    );
    model.press(&restore);
    assert_eq!(
        model.dialog(),
        Some(Dialog::Passphrase {
            action: restore.clone(),
            twice: false
        })
    );
    assert_eq!(
        model.passphrase_entered("invented-harbor-juniper-lantern", None),
        act(
            ActionId::RestoreHere,
            "iOYN-a75Nzr_s5renEJm6g",
            Input::passphrase(typed("invented-harbor-juniper-lantern"))
        ),
        "the dialog shows the confirm, so its answer accepts it"
    );
    let wrong = "That is not the recovery passphrase you set.";
    model.acted(Err(refusal("invalid_params", wrong)));
    assert_eq!(model.refusal.as_deref(), Some(wrong));
    model.press(&restore);
    model.passphrase_entered("invented-harbor-juniper-meadow", None);
    model.acted(Ok(acted(restore_moving(), Value::Null)));
    let shown = screen_of(&model);
    assert_eq!(shown.state, SummaryState::Moving);
    assert_ne!(shown.sentence, RESTARTING);
    assert_eq!(
        shown.sentence,
        "Restoring Fermix on this computer from its backup. Messages wait until it is done."
    );
    assert!(shown.shows_progress && shown.action.is_none());
    assert_eq!(model.refusal, None);
}

/// A move back that stopped, failed or was undone, and a restore that stopped, offer what the
/// engine offers there.
#[test]
fn a_move_back_or_a_restore_that_stopped_offers_what_the_engine_offers() {
    let mut failed = model_on(return_failed());
    let shown = screen_of(&failed);
    assert_eq!(shown.state, SummaryState::Problem);
    assert_eq!(ids(&shown.others), [ActionId::LearnMore]);
    let move_back = shown.action.expect("Move back");
    assert_eq!(
        failed.press(&move_back),
        act(ActionId::MoveBack, "k-return-again", Input::none())
    );
    let stopped = screen(&decoded(return_stopped()));
    assert_eq!(
        stopped.sentence,
        "Moving back stopped part-way, so cancel it and move back again."
    );
    assert_eq!(stopped.action.map(|a| a.id), Some(ActionId::Cancel));
    let mut undone = model_on(return_undone());
    let shown = screen_of(&undone);
    assert_eq!(ids(&shown.others), [ActionId::Cancel]);
    let try_again = shown.action.expect("Try again");
    undone.press(&try_again);
    assert_eq!(
        undone.dialog(),
        Some(Dialog::Passphrase {
            action: try_again,
            twice: false
        })
    );
    let mut restore = model_on(restore_failed());
    let again = screen_of(&restore).action.expect("Restore here again");
    restore.press(&again);
    assert_eq!(
        restore.dialog(),
        Some(Dialog::Passphrase {
            action: again,
            twice: false
        })
    );
    assert_eq!(
        restore.passphrase_entered("invented-harbor-juniper-lantern", None),
        act(
            ActionId::RestoreHere,
            RESTORE_AGAIN_KEY,
            Input::passphrase(typed("invented-harbor-juniper-lantern"))
        )
    );
}

/// The app keys nothing on a problem's kind: every kind the engine publishes draws its sentence
/// and its actions as offered.
#[test]
fn every_problem_kind_draws_its_sentence_and_its_actions() {
    let schema: Value =
        serde_json::from_str(include_str!("../contracts/management/protocol.schema.json"))
            .expect("the schema is JSON");
    let kinds = schema["$defs"]["homeSummaryDetail"]["properties"]["kind"]["enum"]
        .as_array()
        .expect("the kinds")
        .clone();
    assert_eq!(kinds.len(), 8);
    for drawn in ["cannot_reach", "return_failed"] {
        assert!(kinds.contains(&json!(drawn)), "{drawn}");
    }
    for kind in kinds {
        let mut problem = result("home_status_problem")["summary"].clone();
        problem["detail"]["kind"] = kind.clone();
        let shown = screen(&decoded(problem));
        assert_eq!(shown.state, SummaryState::Problem, "{kind}");
        assert!(
            shown
                .sentence
                .starts_with("Fermix cannot reach your server"),
            "{kind}"
        );
        assert_eq!(
            shown.action.map(|a| a.id),
            Some(ActionId::LearnMore),
            "{kind}"
        );
        assert_eq!(ids(&shown.others), [ActionId::RestoreHere], "{kind}");
    }
}

// ---- The notifications --------------------------------------------------------------------------

#[test]
fn a_notice_is_raised_once_for_each_id_and_the_last_sixteen_are_kept() {
    let mut model = Model::default();
    model.pane_shown();
    model.take_read();
    model.served(served());
    let ready = model.answered(status("home_status_ready"));
    assert_eq!(
        ready,
        Some(Notice {
            id: "ready:cand-5e2a9c1f7b3d4086".into(),
            sentence: "Your server is ready for the move.".into()
        })
    );
    assert_eq!(
        model.answered(status("home_status_ready")),
        None,
        "posted once"
    );
    assert_eq!(model.answered(status("home_status_creating")), None);
    let reach = model
        .answered(status("home_status_problem"))
        .expect("a new id");
    assert_eq!(reach.id, "cannot_reach:2031-05-04T11:49:00Z");
    let answer = acted(
        result("home_status_problem")["summary"].clone(),
        Value::Null,
    );
    model.press(&screen_of(&model).others[0]);
    model.passphrase_entered("invented-harbor-juniper-lantern", None);
    assert_eq!(model.acted(Ok(answer)), None, "an act's answer is seen too");
    assert_eq!(POSTED, 16);
    let mut posted = Posted::default();
    let notice = |id: String| Notice {
        id,
        sentence: "Your server is ready for the move.".into(),
    };
    assert!(posted.fresh(&notice("ready:cand-0".into())));
    for n in 1..POSTED {
        assert!(posted.fresh(&notice(format!("ready:cand-{n}"))));
    }
    assert!(!posted.fresh(&notice("ready:cand-0".into())), "still kept");
    assert!(posted.fresh(&notice("ready:cand-16".into())));
    assert!(
        posted.fresh(&notice("ready:cand-0".into())),
        "the oldest of sixteen is let go: an id can come back after an outage clears"
    );
}

/// Fermix back on this computer from a server of the owner's own is one notice like the others:
/// raised once, by its id.
#[test]
fn fermix_back_from_a_server_of_the_owners_own_is_raised_once() {
    let schema: Value =
        serde_json::from_str(include_str!("../contracts/management/protocol.schema.json"))
            .expect("the schema is JSON");
    let pattern = schema["$defs"]["homeSummary"]["properties"]["notice"]["oneOf"][0]["properties"]
        ["id"]["pattern"]
        .as_str()
        .expect("the notice ids' pattern");
    assert!(pattern.contains("|back)"), "{pattern}");
    let mut model = Model::default();
    model.pane_shown();
    model.take_read();
    model.served(served());
    let raised = model.answered(answering(back())).expect("raised");
    assert_eq!(raised.id, "back:5");
    assert!(raised
        .sentence
        .starts_with("Fermix is back on this computer."));
    assert_eq!(model.answered(answering(back())), None, "raised once");
    assert_eq!(screen_of(&model).state, SummaryState::Off);
}

/// The ids raised outlive the app, bounded at sixteen, so a standing notice is not raised again
/// at every launch.
#[test]
fn the_raised_ids_outlive_the_app_and_stay_sixteen() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir
        .path()
        .join("state/fermix-desktop/always-on-notices.json");
    let mut posted = Posted::load(&file).expect("nothing saved yet is none raised");
    let notice = |n: usize| Notice {
        id: format!("ready:cand-{n}"),
        sentence: "Your server is ready for the move.".into(),
    };
    for n in 0..20 {
        assert!(posted.fresh(&notice(n)));
    }
    posted.save(&file).expect("saved");
    let mut again = Posted::load(&file).expect("read back");
    assert_eq!(again.ids().len(), POSTED);
    assert!(!again.fresh(&notice(19)), "raised in the last run");
    assert!(again.fresh(&notice(0)), "let go of before the save");
    let mut model = Model::with_posted(Posted::load(&file).expect("read back"));
    model.pane_shown();
    model.take_read();
    model.served(served());
    let ready = result("home_status_ready")["summary"]["notice"].clone();
    let mut raised = Posted::default();
    raised.fresh(&decoded(ready));
    raised.save(&file).expect("saved");
    let mut model_after = Model::with_posted(Posted::load(&file).expect("read back"));
    model_after.pane_shown();
    model_after.take_read();
    model_after.served(served());
    assert_eq!(
        model_after.answered(status("home_status_ready")),
        None,
        "raised before the app started"
    );
    assert!(model.answered(status("home_status_ready")).is_some());
    assert_eq!(
        model.posted().ids().last().map(String::as_str),
        Some("ready:cand-5e2a9c1f7b3d4086")
    );
}

#[test]
fn a_damaged_or_oversized_notices_file_is_an_error_and_a_long_one_keeps_its_last_sixteen() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir.path().join("always-on-notices.json");
    std::fs::write(&file, "{not json").expect("written");
    assert!(Posted::load(&file).is_err());
    std::fs::write(&file, "x".repeat(64 * 1024)).expect("written");
    let too_big = Posted::load(&file).expect_err("over the bound");
    assert!(too_big.contains("larger"), "{too_big}");
    let ids: Vec<String> = (0..40).map(|n| format!("moved:{n}")).collect();
    std::fs::write(&file, json!({ "posted": ids }).to_string()).expect("written");
    let kept = Posted::load(&file).expect("read");
    let expected: Vec<String> = (24..40).map(|n| format!("moved:{n}")).collect();
    assert_eq!(kept.ids(), expected, "the newest sixteen");
    assert_eq!(
        notices_file(
            Some(std::ffi::OsStr::new("/state")),
            std::path::Path::new("/home/ada")
        ),
        std::path::PathBuf::from("/state/fermix-desktop/always-on-notices.json")
    );
    assert_eq!(
        notices_file(
            Some(std::ffi::OsStr::new("relative")),
            std::path::Path::new("/home/ada")
        ),
        std::path::PathBuf::from("/home/ada/.local/state/fermix-desktop/always-on-notices.json")
    );
}

// ---- What the pane's source holds -------------------------------------------------------------

const PANE_SOURCES: [(&str, &str); 6] = [
    ("core/src/always_on.rs", include_str!("../src/always_on.rs")),
    (
        "core/src/always_on/model.rs",
        include_str!("../src/always_on/model.rs"),
    ),
    (
        "core/src/always_on/notices.rs",
        include_str!("../src/always_on/notices.rs"),
    ),
    ("core/src/home.rs", include_str!("../src/home.rs")),
    (
        "core/src/management.rs",
        include_str!("../src/management.rs"),
    ),
    (
        "app/src/always_on.rs",
        include_str!("../../app/src/always_on.rs"),
    ),
];

/// The provider is the engine's data: no word, name or comment of the app's own names one.
#[test]
fn no_provider_is_named_in_the_apps_own_words() {
    for (path, source) in PANE_SOURCES {
        assert!(
            !source.to_lowercase().contains("hetzner"),
            "{path} names a provider"
        );
    }
}

/// Every step is the engine's: the pane sends `home.act` and reads `home.status`, and calls none
/// of the step methods the engine took over, nor follows a job; and nothing reads what a
/// provider sells, since Fermix creates no server.
#[test]
fn the_pane_calls_no_step_of_its_own() {
    for (path, source) in PANE_SOURCES {
        assert!(
            !source.contains("home.provision."),
            "{path} reads a provider"
        );
    }
    let own = PANE_SOURCES
        .iter()
        .filter(|(path, _)| *path != "core/src/management.rs");
    for (path, source) in own {
        for gone in [
            "home.move.",
            "home.enroll.",
            "home.snapshots.",
            "job_get",
            "job_cancel",
            "job_list",
        ] {
            assert!(!source.contains(gone), "{path} calls {gone}");
        }
    }
}

/// The pane reads on no timer: the window's only timer is the bar that claims no percentage.
#[test]
fn the_pane_reads_on_no_timer() {
    let (_, pane) = PANE_SOURCES[5];
    let timers: Vec<&str> = pane
        .lines()
        .filter(|line| line.contains("timeout_add") || line.contains("timeout_future"))
        .collect();
    assert_eq!(
        timers,
        ["    glib::timeout_add_local(PULSE, move || match weak.upgrade() {"],
        "only the bar's pulse"
    );
    for (path, source) in PANE_SOURCES {
        assert!(!source.contains("Duration::from_secs(600)"), "{path}");
        assert!(!source.contains("OWN_MOVE"), "{path} times the own move");
    }
}
