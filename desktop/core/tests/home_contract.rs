//! The home half of the vendored management contract (Always On setup §3): every `home.status`
//! golden decodes its summary through the app's own types, every word the contract publishes is
//! one the app knows, and each call the Always On pane makes, `home.status`, `home.act` and
//! `home.provision.options`, is sent by the app's own client to a fake daemon answering with the
//! golden, so the request it writes is held to the contract's request fixture too. A drift in the
//! engine fails here rather than on a call.

use fermix_client::frame::{read_frame, write_frame};
use fermix_client::home::{
    passphrase, ActResult, Acted, ActionId, HomeStatus, Input, Need, Passphrase, SummaryState,
    Token, PASSPHRASE_MISMATCH,
};
use fermix_client::management::{CallError, Management};
use fermix_client::provision::Pick;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;

const SCHEMA: &str = include_str!("../contracts/management/protocol.schema.json");
const REQUESTS: &str = include_str!("../contracts/management/fixtures/requests.jsonl");
const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const ERRORS: &str = include_str!("../contracts/management/fixtures/errors.jsonl");
/// The one provider the goldens name.
const PROVIDER: &str = "hetzner";

/// The one line of `lines` named `name`.
fn named(lines: &str, name: &str) -> Value {
    let found: Vec<Value> = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .filter(|v| v["name"] == name)
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one fixture named {name}");
    found.into_iter().next().expect("one")
}

fn result(name: &str) -> Value {
    named(SUCCESS, name)["response"]["result"].clone()
}

fn decoded<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    serde_json::from_value(result(name)).unwrap_or_else(|e| panic!("{name} does not decode: {e}"))
}

/// A fake daemon that reads one request and answers with `response`, the golden's envelope with
/// the request's own id; it hands the request back.
fn serve(response: Value, call: impl FnOnce(&Management)) -> Value {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).expect("the fake daemon binds");
    let daemon = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the client connects");
        let request: Value =
            serde_json::from_slice(&read_frame(&mut stream).expect("a request frame"))
                .expect("the request is JSON");
        let mut answer = response;
        answer["request_id"] = request["request_id"].clone();
        let bytes = serde_json::to_vec(&answer).expect("the answer serializes");
        write_frame(&mut stream, &bytes).expect("the answer is written");
        request
    });
    call(&Management::new(socket, Duration::from_secs(2)));
    daemon.join().expect("the fake daemon ends")
}

/// Sends one call against the golden `answer` and holds what the client wrote to the request
/// fixture `request`: the same method and the same params.
fn exchange(request: &str, answer: &str, call: impl FnOnce(&Management)) {
    let golden = named(SUCCESS, answer)["response"].clone();
    let sent = serve(golden, call);
    let expected = &named(REQUESTS, request)["frame"];
    assert_eq!(sent["method"], expected["method"], "{request}");
    assert_eq!(sent["params"], expected["params"], "{request}");
    assert_eq!(
        sent["protocol_version"], expected["protocol_version"],
        "{request}"
    );
}

/// The refusal a call met when the fake daemon answered with the error golden `name`.
fn refused_with(name: &str, call: impl FnOnce(&Management) -> CallError) -> CallError {
    let mut seen = None;
    serve(named(ERRORS, name)["response"].clone(), |m| {
        seen = Some(call(m))
    });
    let Some(CallError::Refused(refusal)) = &seen else {
        panic!("{name} is a refusal")
    };
    assert_eq!(refusal.code, named(ERRORS, name)["code"], "{name}");
    seen.expect("answered")
}

fn refused_sentence(error: CallError) -> String {
    match error {
        CallError::Refused(refusal) => refusal.sentence,
        other => panic!("a refusal, not {other:?}"),
    }
}

/// The request fixture's params for one `home.act`.
fn act_params(request: &str) -> Value {
    named(REQUESTS, request)["frame"]["params"].clone()
}

fn act_id(params: &Value) -> ActionId {
    serde_json::from_value(params["action"].clone()).expect("an action id")
}

#[test]
fn every_status_golden_decodes_its_summary_with_each_action_one_record() {
    let states = [
        (
            "home_status",
            SummaryState::Undone,
            Some(ActionId::TryAgain),
        ),
        (
            "home_status_creating",
            SummaryState::Creating,
            Some(ActionId::Cancel),
        ),
        (
            "home_status_ready",
            SummaryState::Ready,
            Some(ActionId::MoveNow),
        ),
        (
            "home_status_ready_first_move",
            SummaryState::Ready,
            Some(ActionId::MoveNow),
        ),
        ("home_status_candidate", SummaryState::Candidate, None),
        ("home_status_server", SummaryState::Primary, None),
        ("home_status_on", SummaryState::On, None),
        (
            "home_status_problem",
            SummaryState::Problem,
            Some(ActionId::RestartServer),
        ),
    ];
    for (name, state, action) in states {
        let summary = decoded::<HomeStatus>(name).summary;
        assert_eq!(summary.state, state, "{name}");
        assert_eq!(summary.action.as_ref().map(|a| a.id), action, "{name}");
        assert!(
            !summary.sentence.is_empty() && !summary.key.is_empty(),
            "{name}"
        );
        let every = summary
            .action
            .iter()
            .chain(&summary.detail.alternatives)
            .chain(&summary.detail.actions);
        for record in every {
            assert!(!record.key.is_empty() && !record.label.is_empty(), "{name}");
        }
    }
    let ready = decoded::<HomeStatus>("home_status_ready").summary;
    let move_now = ready.action.expect("Move now");
    assert_eq!(move_now.needs, [Need::RecoveryPassphrase]);
    assert!(move_now.confirm.is_some() && move_now.url.is_none());
    let notice = ready.notice.expect("the server is ready");
    assert_eq!(notice.id, "ready:cand-5e2a9c1f7b3d4086");
    let first = decoded::<HomeStatus>("home_status_ready_first_move").summary;
    assert_eq!(
        first.action.expect("Move now").needs,
        [Need::NewRecoveryPassphrase]
    );
    let on = decoded::<HomeStatus>("home_status_on").summary;
    assert_eq!(on.detail.actions[1].needs, [Need::Choice]);
    assert_eq!(on.detail.lines.len(), 8);
}

/// Every state, action id and need the contract publishes decodes as one this app knows, so a
/// word the engine adds fails here until the app has a place for it; and `home.act` takes every
/// id but the three a surface opens itself.
#[test]
fn every_published_summary_word_is_one_this_app_knows() {
    let schema: Value = serde_json::from_str(SCHEMA).expect("the schema is JSON");
    let defs = &schema["$defs"];
    let words = |pointer: &str| -> Vec<Value> {
        defs.pointer(pointer)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{pointer} lists its words"))
            .clone()
    };
    for state in words("/homeSummary/properties/state/enum") {
        let known: SummaryState = serde_json::from_value(state.clone()).expect("decodes");
        assert_ne!(known, SummaryState::Unknown, "{state}");
    }
    let mut drawn = BTreeSet::new();
    for id in words("/homeSummaryAction/properties/id/enum") {
        let known: ActionId = serde_json::from_value(id.clone()).expect("decodes");
        assert_ne!(known, ActionId::Unknown, "{id}");
        drawn.insert(id.as_str().expect("a word").to_owned());
    }
    for need in words("/homeSummaryAction/properties/needs/items/enum") {
        let known: Need = serde_json::from_value(need.clone()).expect("decodes");
        assert_ne!(known, Need::Unknown, "{need}");
    }
    let sent: BTreeSet<String> = words("/home_act_params/properties/action/enum")
        .iter()
        .map(|id| id.as_str().expect("a word").to_owned())
        .collect();
    let opened_here: BTreeSet<String> = drawn.difference(&sent).cloned().collect();
    let expected: BTreeSet<String> = ["learn_more", "open_console", "update"]
        .map(str::to_owned)
        .into();
    assert_eq!(opened_here, expected);
    for id in opened_here {
        let id: ActionId = serde_json::from_value(json!(id)).expect("decodes");
        assert!(id.opened_here(), "{id:?}");
    }
    for id in &sent {
        let id: ActionId = serde_json::from_value(json!(id)).expect("decodes");
        assert!(!id.opened_here(), "{id:?}");
    }
    let later: ActionId = serde_json::from_value(json!("fly_away")).expect("decodes");
    assert_eq!(
        later,
        ActionId::Unknown,
        "a newer engine's action is no error"
    );
    let later: Need = serde_json::from_value(json!("fingerprint")).expect("decodes");
    assert_eq!(later, Need::Unknown);
}

#[test]
fn the_status_is_asked_with_no_params() {
    exchange("home_status", "home_status", |m| {
        let status = m.home_status().expect("the golden decodes");
        assert_eq!(status.summary.state, SummaryState::Undone);
    });
}

/// Each step is one `home.act` with the action's id, the key it was drawn with, and only what
/// its `needs` asked for; the answer is the summary after the step.
#[test]
fn each_step_is_one_home_act_with_the_key_and_the_input_its_needs_asked_for() {
    let token = Token::pasted("invented-hetzner-token-6d1f0c9a2b7e4f3a").expect("pasted");
    let params = act_params("home_act");
    exchange("home_act", "home_act", |m| {
        let answer = m
            .home_act(
                act_id(&params),
                params["key"].as_str().expect("a key"),
                &Input::token(token, None),
            )
            .expect("decodes");
        assert_eq!(answer.summary.state, SummaryState::Choose);
        assert_eq!(answer.result, None);
    });
    let params = act_params("home_act_create_server");
    let pick = Pick {
        size: "cpx22".into(),
        location: "fsn1".into(),
    };
    exchange("home_act_create_server", "home_act", |m| {
        m.home_act(
            act_id(&params),
            params["key"].as_str().expect("a key"),
            &Input::choice(Some(pick)),
        )
        .expect("decodes");
    });
    let params = act_params("home_act_move_now");
    let typed = Passphrase::typed("invented-harbor-juniper-lantern").expect("typed");
    exchange("home_act_move_now", "home_act", |m| {
        m.home_act(
            act_id(&params),
            params["key"].as_str().expect("a key"),
            &Input::passphrase(typed),
        )
        .expect("decodes");
    });
    let params = act_params("home_act_cancel");
    exchange("home_act_cancel", "home_act", |m| {
        m.home_act(
            act_id(&params),
            params["key"].as_str().expect("a key"),
            &Input::none(),
        )
        .expect("decodes");
    });
}

#[test]
fn a_provider_is_named_only_where_the_engine_lists_several_and_a_choice_sends_both_halves() {
    let token = Token::pasted("invented-nimbus-key-0001").expect("pasted");
    let sent = serve(named(SUCCESS, "home_act")["response"].clone(), |m| {
        m.home_act(
            ActionId::PasteToken,
            "k-token",
            &Input::token(token, Some("nimbus")),
        )
        .expect("decodes");
    });
    assert_eq!(
        sent["params"],
        json!({"action": "paste_token", "key": "k-token",
            "input": {"token": "invented-nimbus-key-0001", "provider": "nimbus"}})
    );
    let sent = serve(named(SUCCESS, "home_act")["response"].clone(), |m| {
        m.home_act(ActionId::CreateServer, "k-create", &Input::choice(None))
            .expect("decodes");
    });
    assert_eq!(
        sent["params"],
        json!({"action": "create_server", "key": "k-create"}),
        "the recommendation sends neither half"
    );
}

/// `own_server` answers its code once, which no print of it spells, with the command that
/// carries none; Move now answers the move's id.
#[test]
fn the_act_results_are_the_code_withheld_from_print_and_the_moves_id() {
    let summary = result("home_act")["summary"].clone();
    let code: Acted = serde_json::from_value(json!({"summary": summary,
        "result": {"code": "fxenroll1.aW52ZW50ZWQtY29kZQ", "expires_at": "2031-05-04T12:30:00Z",
            "command": "curl -fsSL https://fermix.ai/always-on | sudo sh"}}))
    .expect("decodes");
    let Some(ActResult::OwnServer(own)) = &code.result else {
        panic!("the code")
    };
    assert_eq!(own.code(), "fxenroll1.aW52ZW50ZWQtY29kZQ");
    assert!(
        !own.command.contains("fxenroll1."),
        "the command carries no code"
    );
    assert!(!format!("{code:?}").contains("fxenroll1."));
    let moved: Acted = serde_json::from_value(json!({"summary": summary,
        "result": {"move_id": "mv-0f1e2d3c"}}))
    .expect("decodes");
    assert_eq!(
        moved.result,
        Some(ActResult::Moved {
            move_id: "mv-0f1e2d3c".into()
        })
    );
}

/// An answer that does not decode is never quoted in the error: it can hold the code.
#[test]
fn an_odd_answer_to_an_act_is_never_quoted() {
    let odd = json!({"request_id": "x", "result": {"summary": "fxenroll1.c2VjcmV0",
        "result": null}});
    let mut seen = None;
    serve(odd, |m| {
        seen = Some(m.home_act(ActionId::OwnServer, "k-own", &Input::none()))
    });
    let Some(Err(CallError::Protocol(detail))) = seen else {
        panic!("a broken answer")
    };
    assert!(!detail.contains("fxenroll1."), "{detail}");
}

#[test]
fn the_offers_are_asked_of_one_provider() {
    exchange("home_provision_options", "home_provision_options", |m| {
        let options = m.provision_options(PROVIDER).expect("decodes");
        assert_eq!(options.currency, "EUR");
        let recommended = options.recommended.clone().expect("a recommendation");
        assert_eq!(
            recommended,
            Pick {
                size: "cpx22".into(),
                location: "fsn1".into()
            }
        );
        let offer = options.offer(&recommended).expect("its offer");
        assert_eq!((offer.cores, offer.memory_gb, offer.disk_gb), (2, 4.0, 80));
        assert_eq!(offer.total_monthly, "20.09");
        assert_eq!(options.city("hel1"), Some("Helsinki"));
        assert_eq!(options.city("invented-ash2"), None);
        assert_eq!(
            options.sentence,
            "Recommended: CPX22 in Falkenstein, €20 a month."
        );
    });
}

/// The refusals of a step come back in the daemon's words, which the pane shows: `changed`
/// with its sentence alone, the owner's decision, a lost start, an unreachable server, and a
/// step this socket no longer takes.
#[test]
fn the_refusals_of_a_step_are_said_in_the_daemons_words() {
    let changed = refused_with("changed", |m| {
        m.home_act(ActionId::CreateServer, "k-old", &Input::none())
            .expect_err("refused")
    });
    assert_eq!(
        refused_sentence(changed),
        "Always On changed since this was shown, so look at it again."
    );
    let owner = refused_with("unavailable_home_owner_decision", |m| {
        m.home_act(ActionId::CreateServer, "k-create", &Input::none())
            .expect_err("refused")
    });
    assert_eq!(
        refused_sentence(owner),
        "Only the owner can set up Always On, move Fermix or set its recovery passphrase: use \
         the Fermix app, the setup page or your own terminal."
    );
    let typed = Passphrase::typed("invented-harbor-juniper").expect("typed");
    let lost = refused_with("unconfirmed", |m| {
        m.home_act(ActionId::MoveNow, "k-move", &Input::passphrase(typed))
            .expect_err("unconfirmed")
    });
    assert!(refused_sentence(lost).starts_with("The server fermix-vm did not confirm in time"));
    let away = refused_with("primary_unreachable", |m| {
        m.home_status().expect_err("refused")
    });
    assert_eq!(
        refused_sentence(away),
        "The server fermix-vm is unreachable. Fermix keeps your message until it is back."
    );
    let rail = named(ERRORS, "unavailable_rail_only");
    assert_eq!(
        rail["response"]["error"]["details"]["sentence"],
        "Always On takes this step itself: send home.act with the action the summary offers."
    );
}

/// Every home golden is decoded by a call this app makes, or named as one it no longer makes:
/// the engine runs those steps itself, behind `home.act`.
#[test]
fn no_home_golden_goes_unread() {
    let read: BTreeSet<&str> = [
        "home_status",
        "home_status_creating",
        "home_status_ready",
        "home_status_ready_first_move",
        "home_status_candidate",
        "home_status_server",
        "home_status_on",
        "home_status_problem",
        "home_provision_options",
        "home_act",
    ]
    .into_iter()
    .collect();
    for name in &read {
        let value = result(name);
        assert!(value.is_object(), "{name}");
    }
    // Rail-only, or a step `home.act` takes now: no call of this app's makes them.
    let uncalled = [
        "home_snapshots_status",
        "home_snapshots_run",
        "home_snapshots_passphrase",
        "home_enroll_open",
        "home_enroll_withdraw",
        "home_candidates_list",
        "home_move_readiness",
        "home_move_start",
    ];
    let published: Vec<String> = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .filter(|name| name.starts_with("home_"))
        .collect();
    for name in &published {
        assert!(
            read.contains(name.as_str()) || uncalled.contains(&name.as_str()),
            "{name} is not decoded"
        );
    }
    assert_eq!(published.len(), read.len() + uncalled.len());
}

/// A step may check a token with its provider or review the move on the server first: the
/// client gives it the 90 s the contract asks; the offers read the provider live; every other
/// call keeps the client's own deadline, never a shorter one.
#[test]
fn the_long_calls_get_their_own_deadlines() {
    let client = Management::new("/nowhere/daemon.sock".into(), Duration::from_secs(10));
    assert_eq!(client.deadline_for("home.act"), Duration::from_secs(90));
    assert_eq!(
        client.deadline_for("home.provision.options"),
        Duration::from_secs(35)
    );
    for method in ["home.status", "job.get", "home.move.start"] {
        assert_eq!(
            client.deadline_for(method),
            Duration::from_secs(10),
            "{method}"
        );
    }
    let patient = Management::new("/nowhere/daemon.sock".into(), Duration::from_secs(120));
    assert_eq!(
        patient.deadline_for("home.act"),
        Duration::from_secs(120),
        "never shorter than the client's own"
    );
}

/// The token and the passphrase go out on the wire and nowhere else: no print spells them.
#[test]
fn the_secrets_are_never_printed() {
    let token = Token::pasted(" invented-hetzner-token-6d1f0c9a2b7e4f3a\n").expect("pasted");
    assert!(!format!("{token:?}").contains("invented-hetzner"));
    assert!(Token::pasted("  \n").is_none());
    let typed = passphrase(
        "invented-lighthouse-meridian",
        "invented-lighthouse-meridian",
    )
    .expect("the two agree");
    assert!(!format!("{typed:?}").contains("lighthouse"));
    let input = Input::token(token, Some("hetzner"));
    let shown = format!("{input:?}");
    assert!(!shown.contains("invented-hetzner"), "{shown}");
    assert!(
        shown.contains("hetzner"),
        "the provider is no secret: {shown}"
    );
    let input = Input::passphrase(typed);
    assert!(!format!("{input:?}").contains("lighthouse"));
    assert_eq!(
        passphrase("one-thing-entirely", "another-thing"),
        Err(Some(PASSPHRASE_MISMATCH))
    );
    assert_eq!(passphrase("", "typed-only-once"), Err(None));
    assert_eq!(Passphrase::typed(""), None);
}
