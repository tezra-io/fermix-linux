//! The Always On pane as a renderer of the engine's summary (Always On setup §3, §8.4): each
//! screen is the summary as the engine words it, every press is one `home.act` with the key it
//! was drawn with and what the action `needs`, its `confirm` is asked first, the answer is drawn,
//! and the app keeps only how it asks for a secret, how it draws a choice, and that it restarts
//! after its own Move now. The summaries are the engine's own goldens, and the token screens the
//! summaries its handoff generated from the code.

use fermix_client::always_on::model::{Act, Dialog, Model, Press, Read};
use fermix_client::always_on::notices::{notices_file, Posted, POSTED};
use fermix_client::always_on::{
    by_city, destructive, listed, offered, screen, served as served_by, token_target, Fact, Page,
    TokenTarget, LINK_FAILED, NEEDED, RESTARTING,
};
use fermix_client::home::{
    Acted, Action, ActionId, HomeStatus, Input, Line, Notice, Passphrase, Provider, Summary,
    SummaryState, Token, PASSPHRASE_MISMATCH,
};
use fermix_client::management::{CallError, Refusal};
use fermix_client::provision::{Options, Pick};
use fermix_client::view::DaemonProblem;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
/// The one provider the goldens name.
const PROVIDER: &str = "hetzner";
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

fn options() -> Options {
    decoded(result("home_provision_options"))
}

/// The row the engine recommends: the golden's.
fn recommended() -> Pick {
    Pick {
        size: "cpx22".into(),
        location: "fsn1".into(),
    }
}

fn record(id: &str, label: &str, needs: Value, confirm: Value, key: &str) -> Value {
    json!({"id": id, "label": label, "needs": needs, "confirm": confirm, "url": null, "key": key})
}

fn hetzner() -> Value {
    json!({"console_url": "https://console.hetzner.cloud/",
        "docs_url": "https://fermix.ai/docs/always-on/#hetzner", "id": "hetzner",
        "name": "Hetzner", "sentence": "Paste a Hetzner API token to let Fermix create your server.",
        "steps": ["Sign in to Hetzner's console and turn on two-factor sign-in for your account.",
            "Make a new project for Fermix alone, so the token reaches nothing else.",
            "In that project, open Security, then API Tokens, and generate a token with Read & Write permission.",
            "Paste the token here: Fermix checks it at once."],
        "token_label": "Hetzner API token"})
}

/// A second provider, invented: nothing in the app may name it.
fn nimbus() -> Value {
    json!({"console_url": "https://console.nimbus.invalid/",
        "docs_url": "https://fermix.ai/docs/always-on/#nimbus", "id": "nimbus", "name": "Nimbus",
        "sentence": "Paste a Nimbus access key to let Fermix create your server.",
        "steps": ["Make a Nimbus account for Fermix alone."], "token_label": "Nimbus access key"})
}

const SET_UP_KEY: &str = "ypJXbXCwrSQSdVK8_tXftA";
const OWN_KEY: &str = "Mx_U6542oONDPqGb3qliTQ";

/// `off`, as the engine words it (the summary handoff's appendix).
fn off_with(providers: Value) -> Value {
    json!({"action": record("paste_token", "Set up", json!(["token"]), Value::Null, SET_UP_KEY),
        "detail": {"alternatives": [record("own_server", "Use my own Linux server", json!([]),
            Value::Null, OWN_KEY)], "providers": providers},
        "key": "WwwCtpM5-2otfTa1t5s38A", "notice": null,
        "sentence": "Keep Fermix running when this Mac sleeps. It moves to a server you own.",
        "state": "off"})
}

fn off() -> Value {
    off_with(json!([hetzner()]))
}

/// `connect` while a code for a server of the owner's own is open.
fn connect() -> Value {
    json!({"action": record("paste_token", "Set up", json!(["token"]), Value::Null, SET_UP_KEY),
        "detail": {"alternatives": [
            record("own_server", "Use my own Linux server", json!([]), Value::Null, OWN_KEY),
            record("cancel", "Cancel the code", json!([]), Value::Null, "PGFKmgUa3i48MJgwQSKf-g")],
            "providers": [hetzner()]},
        "key": "1beYgzYKYRJoEBvPpLz31A", "notice": null,
        "sentence": "Paste a Hetzner API token to let Fermix create your server.",
        "state": "connect"})
}

fn choose() -> Value {
    result("home_act")["summary"].clone()
}

/// A model that read `summary` with the pane shown, every read answered.
fn model_on(summary: Value) -> Model {
    let mut model = Model::default();
    model.pane_shown();
    assert_eq!(model.next_read(), Some(Read::Status));
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

fn token(text: &str) -> Token {
    Token::pasted(text).expect("a token")
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
    let creating = screen(&summary("home_status_creating"));
    assert!(creating.shows_progress);
    assert_eq!(
        creating.details.first(),
        Some(&Fact::Line(Line {
            label: "Server".into(),
            value: "fermix-general-a3f9".into()
        }))
    );
    assert_eq!(
        creating.details.last(),
        Some(&Fact::Line(Line {
            label: "Costs".into(),
            value: "€8.33 a month, billed by the hour until deleted".into()
        }))
    );
    assert_eq!(creating.details.len(), 6);
    let on = screen(&summary("home_status_on"));
    assert_eq!(on.details.len(), 9, "eight lines and what lives there");
    let Some(Fact::Sentence(data)) = on.details.last() else {
        panic!("what lives on the server, last")
    };
    assert!(data.starts_with("Your conversations and memories"));
    let choose = screen(&decoded(choose()));
    assert_eq!(
        choose.details,
        [Fact::Sentence(
            "Hetzner bills by the hour until the server is deleted.".into()
        )]
    );
    for quiet in ["home_status_candidate", "home_status_server", "home_status"] {
        assert!(screen(&summary(quiet)).details.is_empty(), "{quiet}");
    }
}

#[test]
fn a_running_step_shows_the_bar_and_the_engines_progress() {
    let step = json!({"action": record("cancel", "Cancel", json!([]),
            json!("Cancel and delete your server? Everything Fermix created for it is deleted."),
            "G5z__UAdjaH9J2d38M3Zuw"),
        "detail": {"progress": "Checking that Hetzner has the server in stock."},
        "key": "aVo_EyPSBtR07awKKf2KNA", "notice": null,
        "sentence": "Starting your server…", "state": "creating"});
    let shown = screen(&decoded(step));
    assert!(shown.shows_progress);
    assert_eq!(
        shown.progress.as_deref(),
        Some("Checking that Hetzner has the server in stock.")
    );
    let ready = screen(&summary("home_status_ready"));
    assert!(!ready.shows_progress && ready.progress.is_none());
    let moving = json!({"action": null, "detail": {}, "key": "DXMsAJ5oa8GcFfqMnPOUWQ",
        "notice": null, "state": "moving",
        "sentence": "Moving. This Mac restarts and reconnects to your server. Messages wait until it is done."});
    assert!(screen(&decoded(moving)).shows_progress);
}

#[test]
fn ready_lists_the_reviews_items_and_on_and_off_offer_their_other_actions() {
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
    assert_eq!(
        ids(&ready.others),
        [ActionId::DeleteServer],
        "a server waiting for its move can be deleted"
    );
    let on = screen(&summary("home_status_on"));
    assert!(on.action.is_none(), "all is well: no main action");
    assert_eq!(
        ids(&on.others),
        [ActionId::BackUpNow, ActionId::ReplaceServer]
    );
    let off = screen(&decoded(off()));
    assert_eq!(ids(&off.others), [ActionId::OwnServer]);
    let connect = screen(&decoded(connect()));
    assert_eq!(
        ids(&connect.others),
        [ActionId::OwnServer, ActionId::Cancel]
    );
}

#[test]
fn a_problem_draws_its_action_its_alternatives_and_its_note() {
    let failed = json!({"action": record("turn_on", "Turn it on", json!([]), Value::Null,
            "-2-qM4YNTcvtEyU-7v1z6A"),
        "detail": {"alternatives": [record("delete_server", "Delete your server", json!([]),
                json!("Delete your server? Everything on it is deleted."), "c1yXdvadZiiMCMzJaPBJhg")],
            "kind": "power_failed",
            "lines": [{"label": "Server", "value": "fermix-general-a3f9"}],
            "note": "Your server costs its monthly price until it is deleted.",
            "seen_at": "2031-05-04T11:30:00Z"},
        "key": "LcluSnDHlygipjnerXKhpw", "notice": null,
        "sentence": "Fermix could not reach Hetzner, so check this computer's connection and try again.",
        "state": "problem"});
    let shown = screen(&decoded(failed));
    assert_eq!(shown.state, SummaryState::Problem);
    assert_eq!(shown.action.as_ref().map(|a| a.id), Some(ActionId::TurnOn));
    assert_eq!(ids(&shown.others), [ActionId::DeleteServer]);
    assert_eq!(
        shown.details,
        [
            Fact::Sentence("Your server costs its monthly price until it is deleted.".into()),
            Fact::Line(Line {
                label: "Server".into(),
                value: "fermix-general-a3f9".into()
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
    assert!(!offered(&later("create_server", json!(["fingerprint"]))));
    assert!(!offered(&later(
        "create_server",
        json!(["token", "choice"])
    )));
    assert!(offered(&later("create_server", json!(["choice"]))));
    let mut on = summary("home_status_on");
    on.detail.actions.push(later("fly_away", json!([])));
    on.action = Some(later("fly_away", json!([])));
    let shown = screen(&on);
    assert!(shown.action.is_none());
    assert_eq!(
        ids(&shown.others),
        [ActionId::BackUpNow, ActionId::ReplaceServer]
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
    assert_eq!(states.len(), 12);
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
    let mut model = model_on(choose());
    model.unanswered(DaemonProblem::NotRunning);
    let kept = screen_of(&model);
    assert_eq!(kept.state, SummaryState::Choose);
    assert_eq!(model.read_note().as_deref(), Some("Fermix is not running."));
    model.answered(answering(choose()));
    assert_eq!(model.read_note(), None);
}

// ---- When the pane reads ------------------------------------------------------------------------

#[test]
fn nothing_is_read_until_told_and_each_notice_is_one_read() {
    let mut model = Model::default();
    assert_eq!(model.next_read(), None, "nothing at launch");
    model.home_status_changed();
    model.home_status_changed();
    assert_eq!(model.next_read(), Some(Read::Status));
    assert_eq!(
        model.next_read(),
        None,
        "two notices meanwhile are one read"
    );
    model.engine_restarted();
    assert_eq!(
        model.next_read(),
        None,
        "a new engine is read only while shown"
    );
    model.pane_shown();
    assert_eq!(model.next_read(), Some(Read::Status));
    model.served(served());
    model.answered(answering(off()));
    assert_eq!(model.next_read(), None, "off reads nothing more");
    model.engine_restarted();
    assert_eq!(model.next_read(), Some(Read::Status));
    assert_eq!(model.next_read(), None);
}

#[test]
fn a_choice_reads_the_offers_of_the_provider_in_use_once_while_the_pane_shows() {
    let mut model = model_on(choose());
    assert!(screen_of(&model).choice);
    assert_eq!(
        model.next_read(),
        Some(Read::Options),
        "nothing remembered: the engine answers for the provider in use"
    );
    model.options_read(Ok(options()));
    assert_eq!(model.next_read(), None, "read once while the pane shows");
    assert_eq!(model.loaded_options(), Some(&options()));
    model.pane_hidden();
    assert_eq!(model.loaded_options(), None);
    model.pane_shown();
    assert_eq!(model.next_read(), Some(Read::Status));
    model.answered(answering(choose()));
    assert_eq!(
        model.next_read(),
        Some(Read::Options),
        "read again as it shows again"
    );
    let mut off = model_on(off());
    assert_eq!(off.next_read(), None, "no choice shows, so no offers");
    off.pick_provider("hetzner");
    assert_eq!(off.next_read(), None);
}

// ---- Every press is one home.act --------------------------------------------------------------

/// Each offer is drawn as the engine words it, its label and its price, grouped by the city of
/// its place; the app words no size and no price.
#[test]
fn the_offers_are_the_engines_label_and_price_grouped_by_city() {
    let offers = options();
    assert_eq!(offers.provider.name, "Hetzner");
    let first = &offers.offers[0];
    assert_eq!(first.label, "CPX22: 2 cores, 4 GB memory, 80 GB disk");
    assert_eq!(
        first.price,
        "€20.09 a month, billed by the hour until deleted"
    );
    assert_eq!(offers.city(&first.location), Some("Falkenstein"));
    let grouped = by_city(&offers);
    let cities: Vec<&str> = grouped.iter().map(|(city, _)| *city).collect();
    assert_eq!(cities, ["Falkenstein", "Nuremberg", "Helsinki"]);
    assert!(grouped.iter().all(|(_, rows)| !rows.is_empty()));
    let (_, helsinki) = &grouped[2];
    assert!(helsinki.iter().all(|o| o.location == "hel1"));
    assert_eq!(
        grouped.iter().map(|(_, rows)| rows.len()).sum::<usize>(),
        offers.offers.len()
    );
}

/// A choice is sent as the row it is: before the offers are read a press only reads them, and
/// where the engine recommends none the owner picks one first, told why in the engine's words.
#[test]
fn a_choice_waits_for_the_offers_and_sends_the_row_the_owner_was_shown() {
    let mut model = model_on(choose());
    let create = screen_of(&model).action.expect("Create server");
    assert_eq!(model.press(&create), Press::Read, "the offers first");
    assert_eq!(model.dialog(), None);
    assert_eq!(model.working, None);
    assert_eq!(model.next_read(), Some(Read::Options));
    let mut none_in_stock = options();
    none_in_stock.recommended = None;
    none_in_stock.sentence =
        "Hetzner has no server type with 4 GB in stock in Falkenstein right now.".into();
    model.options_read(Ok(none_in_stock));
    assert_eq!(model.press(&create), Press::Drawn);
    assert_eq!(model.dialog(), None, "nothing to send yet");
    assert_eq!(
        model.refusal.as_deref(),
        Some("Hetzner has no server type with 4 GB in stock in Falkenstein right now.")
    );
    let helsinki = Pick {
        size: "cpx22".into(),
        location: "hel1".into(),
    };
    model.choose(helsinki.clone());
    model.press(&create);
    assert_eq!(
        model.confirmed(),
        act(ActionId::CreateServer, &create.key, Input::choice(helsinki))
    );
}

/// Start again needs a choice where the server it creates again kept no size and place: the
/// same choice as a create, then the row.
#[test]
fn start_again_that_needs_a_choice_draws_it_and_sends_the_row() {
    let stopped = json!({"action": record("start_again", "Start again", json!(["choice"]),
            json!("Delete what Fermix created and start again? Hetzner bills the new server by the hour until it is deleted."),
            "k-start-again"),
        "detail": {"alternatives": [], "kind": "create_failed", "note": null,
            "seen_at": "2031-05-04T11:30:00Z"},
        "key": "k-summary", "notice": null, "outcome": "failed",
        "sentence": "Fermix stopped before your server was created, so start again.",
        "state": "problem"});
    let mut model = model_on(stopped);
    assert!(screen_of(&model).choice);
    assert_eq!(model.next_read(), Some(Read::Options));
    model.options_read(Ok(options()));
    let again = screen_of(&model).action.expect("Start again");
    model.press(&again);
    assert_eq!(model.dialog(), Some(Dialog::Confirm(again.clone())));
    assert_eq!(
        model.confirmed(),
        act(
            ActionId::StartAgain,
            "k-start-again",
            Input::choice(recommended())
        )
    );
}

/// The engine serves Always On where `hello` lists `home.act`: a release build lists none of its
/// methods, and the pane is not listed there.
#[test]
fn always_on_is_listed_only_where_the_engine_serves_home_act() {
    assert!(served_by(&served()));
    let release: Vec<String> = served()
        .into_iter()
        .filter(|m| !["home.status", "home.act", "home.provision.options"].contains(&m.as_str()))
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
    assert_eq!(
        model.next_read(),
        Some(Read::Status),
        "read again after the act"
    );
}

#[test]
fn a_confirm_is_asked_first_and_declining_it_sends_nothing() {
    let mut model = model_on(result("home_status_creating")["summary"].clone());
    let cancel = screen_of(&model).action.expect("Cancel");
    assert_eq!(model.press(&cancel), Press::Drawn);
    assert_eq!(model.dialog(), Some(Dialog::Confirm(cancel.clone())));
    model.dismissed();
    assert_eq!(model.dialog(), None);
    assert_eq!(model.working, None);
    model.press(&cancel);
    assert_eq!(
        model.confirmed(),
        act(ActionId::Cancel, "lW7Sx3Zz1dpg_Diwa6TmhA", Input::none())
    );
    assert_eq!(model.dialog(), None);
}

#[test]
fn deleting_steps_never_take_return_for_yes() {
    for id in [
        ActionId::DeleteServer,
        ActionId::Cancel,
        ActionId::StartAgain,
    ] {
        assert!(destructive(id), "{id:?}");
    }
    for id in [
        ActionId::CreateServer,
        ActionId::RestartServer,
        ActionId::MoveNow,
        ActionId::ReplaceServer,
        ActionId::CreateIn,
    ] {
        assert!(!destructive(id), "{id:?}");
    }
}

#[test]
fn set_up_asks_for_the_token_in_place_of_the_action_and_sends_it() {
    let mut model = model_on(off());
    let set_up = screen_of(&model).action.expect("Set up");
    assert_eq!(model.press(&set_up), Press::Drawn);
    let asking = screen_of(&model);
    assert!(asking.asks_token && asking.action.is_none());
    let Some(TokenTarget::Provider(provider)) = model.token_target() else {
        panic!("the one provider listed")
    };
    assert_eq!(provider.token_label, "Hetzner API token");
    assert_eq!(
        provider.docs_url,
        "https://fermix.ai/docs/always-on/#hetzner"
    );
    assert_eq!(model.token_entered("   \n"), Press::Ignored);
    let sent = model.token_entered(" invented-hetzner-token-6d1f0c9a2b7e4f3a\n");
    let one = Input::token(token("invented-hetzner-token-6d1f0c9a2b7e4f3a"), None);
    assert_eq!(sent, act(ActionId::PasteToken, SET_UP_KEY, one));
    assert_eq!(
        model.token_entered("invented-other-token-0042"),
        Press::Ignored,
        "one check at a time"
    );
    let refused = "Hetzner does not accept this token, so paste a Read & Write API token from the Hetzner project you made for Fermix.";
    model.acted(Err(refusal("invalid_params", refused)));
    assert_eq!(model.token_refusal.as_deref(), Some(refused));
    assert!(
        screen_of(&model).asks_token,
        "the field stays for another paste"
    );
    model.token_entered("invented-hetzner-token-77aa0c9a2b7e4f3a");
    model.acted(Ok(acted(choose(), Value::Null)));
    let chosen = screen_of(&model);
    assert_eq!(chosen.state, SummaryState::Choose);
    assert!(!chosen.asks_token);
    assert_eq!(model.token_refusal, None);
}

#[test]
fn with_several_providers_the_owner_picks_one_and_the_token_names_it() {
    let mut model = model_on(off_with(json!([hetzner(), nimbus()])));
    let set_up = screen_of(&model).action.expect("Set up");
    model.press(&set_up);
    let Some(TokenTarget::Pick(listed)) = model.token_target() else {
        panic!("a pick first")
    };
    let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Hetzner", "Nimbus"], "in the engine's order");
    assert_eq!(
        model.token_entered("invented-nimbus-key-0001"),
        Press::Ignored
    );
    model.pick_provider("nimbus");
    let picked = Input::token(token("invented-nimbus-key-0001"), Some("nimbus"));
    assert_eq!(
        model.token_entered("invented-nimbus-key-0001"),
        act(ActionId::PasteToken, SET_UP_KEY, picked)
    );
    let one: Vec<Provider> = decoded(json!([hetzner()]));
    assert!(matches!(
        token_target(&one, Some("nimbus")),
        Some(TokenTarget::Provider(p)) if p.id == PROVIDER
    ));
    assert_eq!(token_target(&[], None), None);
}

#[test]
fn create_server_sends_the_recommended_row_unless_the_owner_changed_it() {
    let mut model = model_on(choose());
    model.next_read();
    model.options_read(Ok(options()));
    let create = screen_of(&model).action.expect("Create server");
    model.press(&create);
    assert_eq!(model.dialog(), Some(Dialog::Confirm(create.clone())));
    assert_eq!(
        model.confirmed(),
        act(
            ActionId::CreateServer,
            "F09RF8MofBF6hdXN7NWtuA",
            Input::choice(recommended())
        ),
        "the row the owner was shown, never left to the engine"
    );
    model.acted(Err(refusal(
        "unavailable",
        "Fermix is already working on your server, so try again once that is done.",
    )));
    assert_eq!(
        model.refusal.as_deref(),
        Some("Fermix is already working on your server, so try again once that is done.")
    );
    let roomier = Pick {
        size: "cpx32".into(),
        location: "hel1".into(),
    };
    model.choose(roomier.clone());
    assert_eq!(model.chosen(), Some(&roomier));
    model.press(&create);
    assert_eq!(
        model.confirmed(),
        act(
            ActionId::CreateServer,
            "F09RF8MofBF6hdXN7NWtuA",
            Input::choice(roomier)
        )
    );
}

#[test]
fn replace_asks_its_choice_in_a_dialog_that_shows_its_confirm() {
    let mut model = model_on(result("home_status_on")["summary"].clone());
    let replace = screen_of(&model).others[1].clone();
    assert_eq!(model.press(&replace), Press::Read);
    assert_eq!(model.dialog(), None, "the offers are read first");
    assert_eq!(model.next_read(), Some(Read::Options));
    model.options_read(Ok(options()));
    assert_eq!(model.dialog(), Some(Dialog::Choice(replace.clone())));
    assert_eq!(
        model.choice_answered(),
        act(
            ActionId::ReplaceServer,
            "ysD6fueqfNRZpiywBU6HPA",
            Input::choice(recommended())
        ),
        "accepting the dialog accepts its confirm: no second question"
    );
    assert_eq!(model.dialog(), None);
    model.acted(Ok(acted(
        result("home_status_on")["summary"].clone(),
        Value::Null,
    )));
    model.press(&replace);
    let elsewhere = Pick {
        size: "cpx32".into(),
        location: "nbg1".into(),
    };
    model.choose(elsewhere.clone());
    assert_eq!(
        model.choice_answered(),
        act(
            ActionId::ReplaceServer,
            "ysD6fueqfNRZpiywBU6HPA",
            Input::choice(elsewhere)
        )
    );
    model.acted(Ok(acted(
        result("home_status_on")["summary"].clone(),
        Value::Null,
    )));
    model.pane_hidden();
    model.pane_shown();
    model.next_read();
    model.answered(answering(result("home_status_on")["summary"].clone()));
    model.press(&replace);
    model.next_read();
    model.options_read(Err("Paste a Hetzner API token first.".into()));
    assert_eq!(model.dialog(), None, "no dialog on a failed read");
    assert_eq!(
        model.refusal.as_deref(),
        Some("Paste a Hetzner API token first.")
    );
    assert_eq!(
        model.press(&replace),
        Press::Read,
        "a press reads them again"
    );
    assert_eq!(model.next_read(), Some(Read::Options));
    model.options_read(Ok(options()));
    assert_eq!(model.dialog(), Some(Dialog::Choice(replace)));
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
    model.acted(Ok(acted(moving.clone(), json!({"move_id": "mv-replace"}))));
    assert_eq!(screen_of(&model).state, SummaryState::Moving);
    assert!(screen_of(&model).sentence.starts_with("Moving."));
}

#[test]
fn changed_says_its_sentence_and_drops_what_was_asked_for_on_the_older_screen() {
    let mut model = model_on(off());
    let set_up = screen_of(&model).action.expect("Set up");
    model.press(&set_up);
    model.token_entered("invented-hetzner-token-6d1f0c9a2b7e4f3a");
    model.acted(Err(refusal("changed", CHANGED)));
    assert_eq!(model.refusal.as_deref(), Some(CHANGED));
    assert_eq!(model.token_refusal, None);
    assert_eq!(model.next_read(), Some(Read::Status));
    let mut newer = connect();
    newer["action"]["key"] = json!("a-newer-key");
    model.answered(answering(newer));
    assert!(
        !screen_of(&model).asks_token,
        "a field for a key no longer offered goes"
    );
    let mut same = model_on(off());
    same.press(&set_up);
    same.answered(answering(off()));
    assert!(screen_of(&same).asks_token, "the same screen keeps it");
}

#[test]
fn the_own_servers_code_shows_with_the_command_until_the_code_or_the_pane_goes() {
    let mut model = model_on(off());
    let own = screen_of(&model).others[0].clone();
    assert_eq!(
        model.press(&own),
        act(ActionId::OwnServer, OWN_KEY, Input::none())
    );
    model.acted(Ok(acted(
        connect(),
        json!({"code": "fxenroll1.aW52ZW50ZWQtY29kZQ", "expires_at": "2031-05-04T12:30:00Z",
            "command": "curl -fsSL https://fermix.ai/always-on | sudo sh"}),
    )));
    let code = model.handout.clone().expect("the code, once");
    assert_eq!(code.code(), "fxenroll1.aW52ZW50ZWQtY29kZQ");
    assert_eq!(
        code.command,
        "curl -fsSL https://fermix.ai/always-on | sudo sh"
    );
    assert_eq!(code.expires_at, "2031-05-04T12:30:00Z");
    model.answered(answering(connect()));
    assert!(model.handout.is_some(), "while the code is open");
    model.pane_hidden();
    assert_eq!(model.handout, None, "the code goes with the pane");
    model.pane_shown();
    model.next_read();
    model.answered(answering(off()));
    model.press(&own);
    model.acted(Ok(acted(
        connect(),
        json!({"code": "fxenroll1.YW5vdGhlci1jb2Rl", "expires_at": "2031-05-04T12:40:00Z",
            "command": "curl -fsSL https://fermix.ai/always-on | sudo sh"}),
    )));
    assert!(model.handout.is_some());
    model.answered(answering(off()));
    assert_eq!(model.handout, None, "the code was closed: off says so");
}

#[test]
fn learn_more_and_open_console_open_their_page_and_update_is_the_apps_own() {
    let retired = json!({"action": {"confirm": null, "id": "learn_more", "key": "AevTAJ1PC7bJ23Nb6779IQ",
            "label": "Learn more", "needs": [], "url": "https://fermix.ai/docs/always-on/#hetzner"},
        "detail": {}, "key": "pE4etWKyHkFBceL_axOMzw", "notice": null, "state": "retired",
        "sentence": "Fermix moved to your server, and this Mac is no longer connected to it."});
    let mut model = model_on(retired);
    let learn = screen_of(&model).action.expect("Learn more");
    assert_eq!(
        model.press(&learn),
        Press::Open("https://fermix.ai/docs/always-on/#hetzner".into())
    );
    assert_eq!(model.working, None, "nothing is sent");
    let mut console: Action = decoded(record(
        "open_console",
        "Open the console",
        json!([]),
        Value::Null,
        "k-console",
    ));
    assert_eq!(model.press(&console), Press::Drawn, "no page to open");
    assert_eq!(model.refusal.as_deref(), Some(LINK_FAILED));
    console.url = Some("https://console.hetzner.cloud/".into());
    assert_eq!(
        model.press(&console),
        Press::Open("https://console.hetzner.cloud/".into())
    );
    let update: Action = decoded(record(
        "update",
        "Update",
        json!([]),
        Value::Null,
        "k-update",
    ));
    assert_eq!(model.press(&update), Press::Update);
}

// ---- The notifications --------------------------------------------------------------------------

#[test]
fn a_notice_is_raised_once_for_each_id_and_the_last_sixteen_are_kept() {
    let mut model = Model::default();
    model.pane_shown();
    model.next_read();
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
    let answer = acted(result("home_status_ready")["summary"].clone(), Value::Null);
    model.press(&screen_of(&model).action.expect("Restart server"));
    model.confirmed();
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
    model.next_read();
    model.served(served());
    let ready = result("home_status_ready")["summary"]["notice"].clone();
    let mut raised = Posted::default();
    raised.fresh(&decoded(ready));
    raised.save(&file).expect("saved");
    let mut model_after = Model::with_posted(Posted::load(&file).expect("read back"));
    model_after.pane_shown();
    model_after.next_read();
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

const PANE_SOURCES: [(&str, &str); 7] = [
    ("core/src/always_on.rs", include_str!("../src/always_on.rs")),
    (
        "core/src/always_on/model.rs",
        include_str!("../src/always_on/model.rs"),
    ),
    (
        "core/src/always_on/notices.rs",
        include_str!("../src/always_on/notices.rs"),
    ),
    ("core/src/provision.rs", include_str!("../src/provision.rs")),
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

/// Every step is the engine's: the pane sends `home.act` and reads `home.status` and the offers,
/// and calls none of the step methods the engine took over, nor follows a job.
#[test]
fn the_pane_calls_no_step_of_its_own() {
    let own = PANE_SOURCES
        .iter()
        .filter(|(path, _)| *path != "core/src/management.rs");
    for (path, source) in own {
        for gone in [
            "home.move.",
            "home.enroll.",
            "home.snapshots.",
            "home.provision.connect",
            "home.provision.create",
            "home.provision.power",
            "home.provision.resources",
            "home.provision.delete",
            "home.provision.providers",
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
    let (_, pane) = PANE_SOURCES[6];
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
