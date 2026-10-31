//! The Always On pane's rules (M75 lean v1 §7): one screen per summary state and problem kind
//! in the engine's words, the few steps the app keeps itself, where each action goes and the
//! calls it makes in order, the notifications raised from what changed between two reads, and
//! how often the summary is read. Everything is fed from the engine's own goldens.

use fermix_client::always_on::actions::{perform, Context, HomeDaemon, Outcome, JOB_OVERDUE};
use fermix_client::always_on::model::{Model, Press};
use fermix_client::always_on::notices::{notice, wait, NoticeKind, Reading};
use fermix_client::always_on::{
    choice_sentence, details, money, own_server_command, page, price, shape, Fact, Local, Page,
    ScreenKind, GUIDE_URL, HETZNER_CONSOLE,
};
use fermix_client::home::{
    passphrase, Action, ActionId, Code, HomeStatus, MoveFrom, MoveStarted, ProblemKind, Readiness,
    ReadinessId, SnapshotRun, SummaryState, PASSPHRASE_MISMATCH,
};
use fermix_client::management::{CallError, Refusal};
use fermix_client::model::{JobList, JobView};
use fermix_client::provision::{Location, Options, Pick, PowerAction, ProvisionView, METHODS};
use fermix_client::view::DaemonProblem;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const CANDIDATE: &str = "cand-5e2a9c1f7b3d4086";
/// 2031-05-04T12:00:00Z, the goldens' "now".
const NOW: i64 = 1_935_662_400;
const DAY: i64 = 86_400;

fn result(name: &str) -> Value {
    SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no golden named {name}"))["response"]["result"]
        .clone()
}

fn golden<T: DeserializeOwned>(name: &str, change: impl FnOnce(&mut Value)) -> T {
    let mut value = result(name);
    change(&mut value);
    serde_json::from_value(value).unwrap_or_else(|e| panic!("{name} decodes: {e}"))
}

fn status(name: &str) -> HomeStatus {
    golden(name, |_| {})
}

/// `home_status_creating` with its summary replaced by `summary`.
fn summarised(summary: Value) -> HomeStatus {
    golden("home_status_creating", |s| s["summary"] = summary)
}

fn problem(kind: &str, action: Value, alternatives: Value) -> HomeStatus {
    summarised(json!({
        "state": "problem", "sentence": format!("A sentence for {kind}."), "action": action,
        "detail": {"kind": kind, "note": null, "seen_at": "2031-05-04T11:50:00Z",
                   "alternatives": alternatives}
    }))
}

fn action(id: &str, label: &str) -> Value {
    json!({"id": id, "label": label})
}

/// Every method the engine serves, as its `hello` lists them.
fn served() -> Vec<String> {
    let hello = result("hello");
    hello["capabilities"]["methods"]
        .as_array()
        .expect("methods")
        .iter()
        .map(|m| m.as_str().expect("a name").to_owned())
        .collect()
}

fn without_hetzner() -> Vec<String> {
    served()
        .into_iter()
        .filter(|m| !METHODS.contains(&m.as_str()))
        .collect()
}

fn screen_of(status: &HomeStatus, local: &Local) -> fermix_client::always_on::Screen {
    match page(Some(status), None, local, &served()) {
        Page::Screen(screen) => *screen,
        other => panic!("a screen, not {other:?}"),
    }
}

fn model_on(status: HomeStatus) -> Model {
    let mut model = Model::default();
    model.served(served());
    assert_eq!(
        model.answered(status, NOW),
        None,
        "a first read raises nothing"
    );
    model
}

fn ids(actions: &[Action]) -> Vec<ActionId> {
    actions.iter().map(|a| a.id).collect()
}

// ---- The screens ----------------------------------------------------------------------------

#[test]
fn an_engine_without_the_summary_or_the_status_is_told_to_update() {
    let older: HomeStatus = golden("home_status", |s| {
        s.as_object_mut().expect("a status").remove("summary");
    });
    assert_eq!(
        page(Some(&older), None, &Local::default(), &served()),
        Page::NeedsUpdate
    );
    let before_home: Vec<String> = served()
        .into_iter()
        .filter(|m| !m.starts_with("home."))
        .collect();
    assert_eq!(
        page(None, None, &Local::default(), &before_home),
        Page::NeedsUpdate
    );
    let refused = DaemonProblem::UpdateNeeded("Fermix is older than this app.".into());
    assert_eq!(
        page(None, Some(&refused), &Local::default(), &served()),
        Page::NeedsUpdate
    );
    assert_eq!(
        page(None, None, &Local::default(), &served()),
        Page::Reading
    );
}

#[test]
fn a_screen_that_needs_hetzner_waits_for_every_provision_method() {
    let creating = status("home_status_creating");
    let choose = summarised(json!({"state": "choose",
        "sentence": "Your token works.", "action": action("create_server", "Create server"),
        "detail": {"billing": "Hetzner bills by the hour until the server is deleted."}}));
    for needs in [&creating, &choose] {
        assert_eq!(
            page(Some(needs), None, &Local::default(), &without_hetzner()),
            Page::NeedsUpdate
        );
        assert!(matches!(
            page(Some(needs), None, &Local::default(), &served()),
            Page::Screen(_)
        ));
    }
    let on = status("home_status_on");
    assert!(matches!(
        page(Some(&on), None, &Local::default(), &without_hetzner()),
        Page::Screen(_)
    ));
}

#[test]
fn off_says_its_lines_and_set_up_opens_the_token_screen_in_offs_own_words() {
    let off = summarised(json!({"state": "off",
        "sentence": "Keep Fermix running when this computer sleeps. It moves to a server you own.\nVoice calls and meetings stop after the move.",
        "action": action("set_up", "Set up"),
        "detail": {"connect": {"sentence": "Paste a Hetzner API token to let Fermix create your server.",
            "steps": ["Turn on two-factor sign-in.", "Make a project for Fermix alone."],
            "alternatives": [action("own_server", "Use my own Linux server")]}}}));
    let shown = screen_of(&off, &Local::default());
    assert_eq!(shown.kind, ScreenKind::Off);
    assert!(shown
        .sentence
        .contains("\nVoice calls and meetings stop after the move."));
    assert_eq!(shown.action.as_ref().map(|a| a.id), Some(ActionId::SetUp));
    assert!(!shown.token_field);
    let mut model = model_on(off);
    assert_eq!(model.press(ActionId::SetUp), Press::Drawn);
    let Page::Screen(token) = model.page() else {
        panic!("a screen")
    };
    assert_eq!(token.kind, ScreenKind::Token);
    assert_eq!(
        token.sentence,
        "Paste a Hetzner API token to let Fermix create your server."
    );
    assert_eq!(token.steps.len(), 2);
    assert_eq!(ids(&token.alternatives), [ActionId::OwnServer]);
    assert!(token.token_field && token.action.is_none());
}

#[test]
fn connect_is_the_token_screen_with_its_steps_and_the_own_server_link() {
    let connect = summarised(json!({"state": "connect",
        "sentence": "Paste a Hetzner API token to let Fermix create your server.",
        "action": null,
        "detail": {"steps": ["Turn on two-factor sign-in."],
                   "alternatives": [action("own_server", "Use my own Linux server")]}}));
    let shown = screen_of(&connect, &Local::default());
    assert_eq!(shown.kind, ScreenKind::Token);
    assert!(shown.token_field);
    assert_eq!(shown.steps, ["Turn on two-factor sign-in."]);
    assert_eq!(ids(&shown.alternatives), [ActionId::OwnServer]);
}

#[test]
fn creating_shows_a_bar_and_asks_for_the_passphrase_only_while_none_is_set() {
    let set = screen_of(&status("home_status_creating"), &Local::default());
    assert_eq!(set.kind, ScreenKind::Creating);
    assert!(set.shows_progress);
    assert_eq!(set.action.as_ref().map(|a| a.id), Some(ActionId::Cancel));
    assert!(!set.asks_passphrase, "the golden has a recipient");
    let unset: HomeStatus = golden("home_status_creating", |s| {
        s["snapshots"]["recipient_set"] = json!(false)
    });
    assert!(screen_of(&unset, &Local::default()).asks_passphrase);
    let replacing: HomeStatus = golden("home_status_creating", |s| {
        s["snapshots"]["recipient_set"] = json!(false);
        s["role"] = json!("device");
    });
    assert!(
        !screen_of(&replacing, &Local::default()).asks_passphrase,
        "a desktop's own snapshots are a retired copy"
    );
}

#[test]
fn ready_lists_its_warnings_and_on_offers_backup_and_replace_in_details() {
    let ready = screen_of(&status("home_status_ready"), &Local::default());
    assert_eq!(ready.kind, ScreenKind::Ready);
    assert_eq!(ready.action.as_ref().map(|a| a.id), Some(ActionId::MoveNow));
    let warned: Vec<ReadinessId> = ready.warnings().into_iter().map(|i| i.id).collect();
    assert_eq!(warned, [ReadinessId::StaysHere]);
    assert!(ready.has_details());
    let on = screen_of(&status("home_status_on"), &Local::default());
    assert_eq!(on.kind, ScreenKind::On);
    assert!(on.action.is_none(), "all is well: no action");
    assert_eq!(
        ids(&on.alternatives),
        [ActionId::BackUpNow, ActionId::ReplaceServer]
    );
    for quiet in ["home_status_candidate", "home_status_server"] {
        let shown = screen_of(&status(quiet), &Local::default());
        assert!(shown.action.is_none() && !shown.has_details(), "{quiet}");
    }
    let undone = screen_of(&status("home_status"), &Local::default());
    assert_eq!(undone.kind, ScreenKind::Undone);
    assert_eq!(
        undone.action.as_ref().map(|a| a.id),
        Some(ActionId::TryAgain)
    );
}

#[test]
fn a_problem_is_its_kind_with_its_one_action_and_unknown_words_draw_nothing() {
    let reach = screen_of(&status("home_status_problem"), &Local::default());
    assert_eq!(reach.kind, ScreenKind::Problem(ProblemKind::CannotReach));
    assert_eq!(
        reach.sentence,
        "Fermix cannot reach your server. Your messages wait until it is back."
    );
    assert_eq!(
        reach.action.as_ref().map(|a| a.id),
        Some(ActionId::RestartServer)
    );
    let newer = problem(
        "tides",
        action("fly_away", "Fly away"),
        json!([action("learn_more", "Learn more"), action("swim", "Swim")]),
    );
    let shown = screen_of(&newer, &Local::default());
    assert_eq!(shown.kind, ScreenKind::Problem(ProblemKind::Unknown));
    assert!(
        shown.action.is_none(),
        "no button for an action this app lacks"
    );
    assert_eq!(ids(&shown.alternatives), [ActionId::LearnMore]);
    let odd = summarised(json!({"state": "levitating", "sentence": "Fermix floats.",
        "action": null, "detail": {}}));
    let shown = screen_of(&odd, &Local::default());
    assert_eq!(
        (shown.kind, shown.sentence.as_str()),
        (ScreenKind::Unknown, "Fermix floats.")
    );
}

/// Only one code is open at a time, so "Use my own Linux server" goes while a server Fermix
/// created stands (Details then offers to delete it): a new code would strand that server (L15).
#[test]
fn get_a_code_is_hidden_while_a_created_server_stands() {
    let stands = problem(
        "not_set_up",
        action("start_again", "Start again"),
        json!([
            action("own_server", "Use my own Linux server"),
            action("open_hetzner", "Open Hetzner"),
            action("delete_server", "Delete your server")
        ]),
    );
    let shown = screen_of(&stands, &Local::default());
    assert_eq!(
        ids(&shown.alternatives),
        [ActionId::OpenHetzner, ActionId::DeleteServer]
    );
    let nothing_stands = problem(
        "account_limit",
        action("open_hetzner", "Open Hetzner"),
        json!([action("own_server", "Use my own Linux server")]),
    );
    let shown = screen_of(&nothing_stands, &Local::default());
    assert_eq!(ids(&shown.alternatives), [ActionId::OwnServer]);
}

#[test]
fn paste_a_new_token_opens_the_field_in_place_of_the_action() {
    let refused = problem(
        "token_refused",
        action("paste_token", "Paste a new token"),
        json!([]),
    );
    let mut model = model_on(refused);
    assert_eq!(model.press(ActionId::PasteToken), Press::Drawn);
    let Page::Screen(shown) = model.page() else {
        panic!("a screen")
    };
    assert!(shown.token_field && shown.action.is_none());
    model.token_checked(Err("Hetzner does not accept this token.".into()));
    assert_eq!(
        model.token_refusal.as_deref(),
        Some("Hetzner does not accept this token.")
    );
    model.token_checked(Ok(golden::<ProvisionView>(
        "home_provision_connect",
        |_| {},
    )));
    assert!(model.token_refusal.is_none() && !model.local.pasting_token);
}

/// A moving screen of its own while the engine restarts into this app's move, until the engine
/// says where the move stands, and never longer than its bound.
#[test]
fn the_apps_own_move_shows_moving_until_the_engine_says_where_it_stands() {
    let mut model = model_on(status("home_status_ready"));
    model.local.move_started = Some("Fermix restarts and moves this home.".into());
    model.unanswered(DaemonProblem::NotRunning);
    let Page::Screen(moving) = model.page() else {
        panic!("a screen")
    };
    assert_eq!(moving.kind, ScreenKind::Moving);
    assert_eq!(moving.sentence, "Fermix restarts and moves this home.");
    assert!(moving.shows_progress && moving.action.is_none());
    model.answered(status("home_status_ready"), NOW + 3);
    assert!(
        model.local.move_started.is_some(),
        "ready says nothing of the move"
    );
    model.answered(status("home_status_on"), NOW + 6);
    assert!(model.local.move_started.is_none());
    assert_eq!(model.page().screen().map(|s| s.kind), Some(ScreenKind::On));
    let mut stuck = model_on(status("home_status_ready"));
    stuck.local.move_started = Some("Moving.".into());
    for read in 0..=200 {
        stuck.answered(status("home_status_ready"), NOW + read);
    }
    assert!(stuck.local.move_started.is_none(), "bounded at 200 reads");
}

#[test]
fn fermix_not_running_here_says_so_and_a_later_failure_keeps_the_screen() {
    assert_eq!(
        page(
            None,
            Some(&DaemonProblem::NotRunning),
            &Local::default(),
            &served()
        ),
        Page::Unreadable(DaemonProblem::NotRunning)
    );
    let mut model = model_on(status("home_status_on"));
    model.unanswered(DaemonProblem::NotRunning);
    assert_eq!(model.page(), Page::Unreadable(DaemonProblem::NotRunning));
    model.unanswered(DaemonProblem::NotResponding);
    assert_eq!(model.page().screen().map(|s| s.kind), Some(ScreenKind::On));
    assert_eq!(
        model.read_note().as_deref(),
        Some("Fermix is not responding.")
    );
    model.answered(status("home_status_on"), NOW + 60);
    assert_eq!(model.read_note(), None);
}

#[test]
fn each_step_the_app_keeps_goes_once_the_engine_moved_past_it() {
    let off = summarised(json!({"state": "off", "sentence": "Keep Fermix running.",
        "action": action("set_up", "Set up"), "detail": {}}));
    let mut model = model_on(off.clone());
    model.press(ActionId::SetUp);
    model.answered(off, NOW + 1);
    assert!(model.local.setting_up, "still off");
    model.answered(status("home_status_creating"), NOW + 2);
    assert!(!model.local.setting_up);
    let mut model = model_on(status("home_status_ready"));
    model.finished(Outcome {
        reviewed: Some(vec![golden::<Readiness>("home_move_readiness", |_| {})
            .items[5]
            .clone()]),
        ..Outcome::default()
    });
    assert_eq!(model.reviewed.len(), 1);
    model.answered(status("home_status_on"), NOW + 1);
    assert!(model.reviewed.is_empty(), "a review belongs to ready");
}

#[test]
fn the_choose_screen_says_the_owners_choice_once_it_differs() {
    let options: Options = golden("home_provision_options", |_| {});
    let choose = summarised(json!({"state": "choose", "sentence": "Your token works.",
        "action": action("create_server", "Create server"), "detail": {}}));
    let mut model = model_on(choose);
    let shown = model.page().screen().expect("a screen");
    assert_eq!(
        model.choose_sentence(&shown),
        "Your token works.",
        "nothing read yet"
    );
    model.options = Some(Ok(options.clone()));
    assert_eq!(model.choose_sentence(&shown), options.sentence);
    assert_eq!(model.pick(), options.recommended);
    let roomy = Pick {
        size: "cpx32".into(),
        location: Location::Hel1,
    };
    model.choose(roomy.clone());
    assert_eq!(model.pick(), Some(roomy));
    assert_eq!(
        model.choose_sentence(&shown),
        "CPX32 in Helsinki, €36.09 a month."
    );
}

#[test]
fn prices_and_shapes_are_hetzners_numbers_as_given() {
    let options: Options = golden("home_provision_options", |_| {});
    let offer = options
        .offer(&options.recommended.clone().expect("a recommendation"))
        .expect("its offer");
    assert_eq!(money("20.09", "EUR"), "€20.09");
    assert_eq!(money("20.09", "USD"), "$20.09");
    assert_eq!(money("20.09", "CHF"), "20.09 CHF");
    assert_eq!(shape(offer), "2 vCPU, 4 GB memory, 80 GB disk");
    assert_eq!(
        price(offer, &options.currency),
        "€19.49 a month, and €0.60 for its IPv4 address. Prices include VAT."
    );
    assert_eq!(
        choice_sentence(offer, &options),
        "CPX22 in Falkenstein, €20.09 a month."
    );
}

#[test]
fn the_own_servers_command_carries_its_code_and_leaves_with_the_pane() {
    let code: Code = golden("home_enroll_open", |_| {});
    let command = own_server_command(&code).expect("a code of the contract's shape");
    assert!(
        command.starts_with("curl -fsSL https://fermix.ai/always-on | sudo FERMIX_ENROLL_CODE=")
    );
    assert!(command.ends_with(&format!("{} sh", code.text())));
    for odd in [
        "fxenroll1.abc;reboot",
        "fxenroll1.",
        "fxenroll2.abc",
        "fxenroll1.a b",
    ] {
        let shaped: Code =
            serde_json::from_value(json!({"code": odd, "expires_at": "2031-05-04T12:30:00Z"}))
                .expect("decodes");
        assert_eq!(
            own_server_command(&shaped),
            None,
            "{odd} goes into no shell line"
        );
    }
    let mut model = model_on(status("home_status_problem"));
    let odd: Code = serde_json::from_value(
        json!({"code": "fxenroll1.abc;reboot", "expires_at": "2031-05-04T12:30:00Z"}),
    )
    .expect("decodes");
    model.code_answered(Ok(odd));
    assert!(
        model.handout.is_none(),
        "a code no command can carry is not shown"
    );
    assert_eq!(
        model.code_refusal.as_deref(),
        Some("Fermix answered in a way this app does not understand.")
    );
    model.code_answered(Ok(code));
    assert!(model.handout.is_some());
    model.pane_hidden();
    assert!(
        model.handout.is_none(),
        "the code leaves memory with the pane"
    );
}

#[test]
fn details_hold_what_an_expert_might_want_and_nothing_else() {
    let on = screen_of(&status("home_status_on"), &Local::default());
    assert_eq!(
        details(&on, None),
        [
            Fact::Value("Name", "fermix-general-a3f9".into()),
            Fact::Value("Location", "Falkenstein".into()),
            Fact::Value("Version", "1.2.3".into()),
            Fact::Time("Last backup in S3".into(), "2031-05-04T10:00:00Z".into()),
            Fact::Bytes("Backups on this computer", 144_633),
            Fact::Share("Server disk in use", 0.42),
            Fact::Line(
                "Your conversations and memories, the secrets, the browser and traces \
                live on your server, and Hetzner can technically read its disk. Backups are \
                encrypted to your recovery passphrase."
                    .into()
            ),
            Fact::Act(Action {
                id: ActionId::BackUpNow,
                label: "Back up now".into()
            }),
            Fact::Act(Action {
                id: ActionId::ReplaceServer,
                label: "Replace the server".into()
            }),
        ]
    );
    let ready = screen_of(&status("home_status_ready"), &Local::default());
    let facts = details(&ready, None);
    assert_eq!(facts.len(), 7, "six facts and the wait: {facts:?}");
    assert_eq!(facts[1], Fact::Value("Size", "CPX22".into()));
    assert_eq!(facts[6], Fact::WaitsUntil("2031-05-05T11:00:00Z".into()));
    let reach = screen_of(&status("home_status_problem"), &Local::default());
    assert_eq!(
        details(&reach, None),
        [Fact::Seen("2031-05-04T11:50:00Z".into())]
    );
    let options: Options = golden("home_provision_options", |_| {});
    let choose = screen_of(
        &summarised(json!({"state": "choose", "sentence": "Your token works.",
            "action": action("create_server", "Create server"),
            "detail": {"billing": "Hetzner bills by the hour until the server is deleted."}})),
        &Local::default(),
    );
    let offer = options
        .offer(&cpx("cpx22", Location::Fsn1))
        .expect("an offer");
    assert_eq!(
        details(&choose, Some((offer, &options.currency))),
        [
            Fact::Line("2 vCPU, 4 GB memory, 80 GB disk".into()),
            Fact::Line(
                "€19.49 a month, and €0.60 for its IPv4 address. Prices include VAT.".into()
            ),
            Fact::Line("Hetzner bills by the hour until the server is deleted.".into()),
        ]
    );
    let creating = screen_of(&status("home_status_creating"), &Local::default());
    assert!(details(&creating, None).is_empty());
}

// ---- Where each action goes ---------------------------------------------------------------

#[test]
fn every_action_the_engine_offers_goes_somewhere() {
    let routes = [
        (ActionId::SetUp, Press::Drawn),
        (ActionId::PasteToken, Press::Drawn),
        (ActionId::ReplaceServer, Press::Drawn),
        (ActionId::LearnMore, Press::Open(GUIDE_URL)),
        (ActionId::OpenHetzner, Press::Open(HETZNER_CONSOLE)),
        (ActionId::Update, Press::Update),
        (ActionId::OwnServer, Press::GetCode),
    ];
    for (id, route) in routes {
        let mut model = model_on(status("home_status_problem"));
        assert_eq!(model.press(id), route, "{id:?}");
    }
    let daemon = [
        ActionId::CreateServer,
        ActionId::Cancel,
        ActionId::MoveNow,
        ActionId::TryAgain,
        ActionId::CheckAgain,
        ActionId::TurnOn,
        ActionId::RestartServer,
        ActionId::StartAgain,
        ActionId::CreateIn,
        ActionId::DeleteServer,
        ActionId::BackUpNow,
    ];
    for id in daemon {
        let mut model = model_on(status("home_status_problem"));
        let Press::Run(context) = model.press(id) else {
            panic!("{id:?} runs on the daemon")
        };
        assert_eq!(context.action, id);
        assert_eq!(model.working, Some(id), "{id:?} holds the buttons");
        assert_eq!(
            model.press(ActionId::BackUpNow),
            Press::Ignored,
            "one at a time"
        );
    }
    let mut model = model_on(status("home_status_problem"));
    assert_eq!(model.press(ActionId::Unknown), Press::Ignored);
}

// ---- The calls each action makes ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Options,
    Create(Pick, bool),
    Power(PowerAction),
    Resources,
    Delete,
    JobList,
    JobCancel(String),
    JobGet(String),
    Readiness(String),
    Start(String, MoveFrom),
    SnapshotsRun,
}

/// A daemon that answers from the goldens and writes down every call, in order. A refusal set
/// for a method answers it instead; jobs answer `job_gets` in turn, then completed.
#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<Call>>,
    pauses: RefCell<u32>,
    refuse: RefCell<Vec<(&'static str, Refusal)>>,
    options: Option<Options>,
    running: Vec<JobView>,
    job_gets: RefCell<VecDeque<JobView>>,
    review: Option<Readiness>,
}

fn job(name: &str, status: &str, failure: Value) -> JobView {
    golden(name, |j| {
        j["status"] = json!(status);
        j["failure"] = failure;
    })
}

fn refusal(code: &str, sentence: &str) -> Refusal {
    Refusal {
        code: code.into(),
        sentence: sentence.into(),
        field: None,
        reason: None,
    }
}

impl Fake {
    fn new() -> Fake {
        Fake {
            options: Some(golden("home_provision_options", |_| {})),
            ..Fake::default()
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    fn refusing(self, method: &'static str, refused: Refusal) -> Fake {
        self.refuse.borrow_mut().push((method, refused));
        self
    }

    fn note<T>(&self, call: Call, method: &str, answer: T) -> Result<T, CallError> {
        self.calls.borrow_mut().push(call);
        let mut refuse = self.refuse.borrow_mut();
        match refuse.iter().position(|(m, _)| *m == method) {
            Some(at) => Err(CallError::Refused(refuse.remove(at).1)),
            None => Ok(answer),
        }
    }
}

impl HomeDaemon for Fake {
    fn provision_options(&self) -> Result<Options, CallError> {
        let options = self.options.clone().expect("options are answered");
        self.note(Call::Options, "options", options)
    }

    fn provision_create(&self, pick: &Pick, replace: bool) -> Result<JobView, CallError> {
        let running = golden("home_provision_create", |_| {});
        self.note(Call::Create(pick.clone(), replace), "create", running)
    }

    fn provision_power(&self, action: PowerAction) -> Result<JobView, CallError> {
        let running = golden("home_provision_power", |_| {});
        self.note(Call::Power(action), "power", running)
    }

    fn provision_resources(&self) -> Result<ProvisionView, CallError> {
        let view = golden("home_provision_resources", |_| {});
        self.note(Call::Resources, "resources", view)
    }

    fn provision_delete(&self) -> Result<JobView, CallError> {
        let running = golden("home_provision_delete", |_| {});
        self.note(Call::Delete, "delete", running)
    }

    fn job_list(&self) -> Result<JobList, CallError> {
        let jobs = JobList {
            jobs: self.running.clone(),
        };
        self.note(Call::JobList, "job.list", jobs)
    }

    fn job_cancel(&self, job_id: &str) -> Result<JobView, CallError> {
        let cancelled = job("home_provision_create", "cancelled", Value::Null);
        self.note(Call::JobCancel(job_id.into()), "job.cancel", cancelled)
    }

    fn job_get(&self, job_id: &str) -> Result<JobView, CallError> {
        let next = self.job_gets.borrow_mut().pop_front();
        let answer = next.unwrap_or_else(|| golden("job_get_home_provision_completed", |_| {}));
        self.note(Call::JobGet(job_id.into()), "job.get", answer)
    }

    fn home_move_readiness(&self, candidate_id: &str) -> Result<Readiness, CallError> {
        let review = self
            .review
            .clone()
            .unwrap_or_else(|| golden("home_move_readiness", |_| {}));
        self.note(Call::Readiness(candidate_id.into()), "readiness", review)
    }

    fn home_move_start(
        &self,
        candidate_id: &str,
        _passphrase: &fermix_client::home::Passphrase,
        from: MoveFrom,
    ) -> Result<MoveStarted, CallError> {
        let started = golden("home_move_start", |_| {});
        self.note(Call::Start(candidate_id.into(), from), "start", started)
    }

    fn home_snapshots_run(&self) -> Result<SnapshotRun, CallError> {
        let run = golden("home_snapshots_run", |_| {});
        self.note(Call::SnapshotsRun, "run", run)
    }

    fn pause(&self, wait: Duration) {
        assert_eq!(
            wait,
            Duration::from_secs(2),
            "jobs are polled every two seconds"
        );
        *self.pauses.borrow_mut() += 1;
    }
}

fn context(action: ActionId, status: HomeStatus) -> Context {
    Context {
        action,
        status: Some(status),
        options: None,
        chosen: None,
        passphrase: None,
        shown: Vec::new(),
    }
}

fn typed() -> fermix_client::home::Passphrase {
    passphrase("invented-harbor-juniper", "invented-harbor-juniper").expect("agree")
}

fn cpx(size: &str, location: Location) -> Pick {
    Pick {
        size: size.into(),
        location,
    }
}

const JOB: &str = "job:Hz3Fx07a";
const DELETE_JOB: &str = "job:Hz5Del9c";

#[test]
fn create_server_reads_the_options_then_creates_the_recommendation_or_the_choice() {
    let fake = Fake::new();
    let creating = context(ActionId::CreateServer, status("home_status_creating"));
    let outcome = perform(&fake, &creating);
    assert_eq!(
        fake.calls(),
        [
            Call::Options,
            Call::Create(cpx("cpx22", Location::Fsn1), false)
        ]
    );
    assert!(
        matches!(outcome.options, Some(Ok(_))),
        "the options it read are kept"
    );
    let fake = Fake::new();
    let chosen = Context {
        chosen: Some(cpx("cpx32", Location::Hel1)),
        options: fake.options.clone(),
        ..creating
    };
    perform(&fake, &chosen);
    assert_eq!(
        fake.calls(),
        [Call::Create(cpx("cpx32", Location::Hel1), false)]
    );
}

#[test]
fn create_server_with_nothing_in_stock_says_so_and_creates_nothing() {
    let mut fake = Fake::new();
    let mut sold_out = fake.options.clone().expect("options");
    sold_out.recommended = None;
    sold_out.sentence = "Hetzner has no server of this kind in stock right now.".into();
    fake.options = Some(sold_out);
    let outcome = perform(
        &fake,
        &context(ActionId::CreateServer, status("home_status_creating")),
    );
    assert_eq!(fake.calls(), [Call::Options]);
    assert_eq!(
        outcome.refusal.as_deref(),
        Some("Hetzner has no server of this kind in stock right now.")
    );
}

#[test]
fn cancel_stops_the_running_job_before_it_deletes_what_was_made() {
    let mut fake = Fake::new();
    fake.running = vec![job("home_provision_create", "running", Value::Null)];
    perform(
        &fake,
        &context(ActionId::Cancel, status("home_status_creating")),
    );
    assert_eq!(
        fake.calls(),
        [
            Call::JobList,
            Call::JobCancel(JOB.into()),
            Call::Delete,
            Call::JobGet(DELETE_JOB.into()),
        ]
    );
    let fake = Fake::new();
    perform(
        &fake,
        &context(ActionId::Cancel, status("home_status_creating")),
    );
    assert_eq!(
        fake.calls(),
        [Call::JobList, Call::Delete, Call::JobGet(DELETE_JOB.into())],
        "nothing running: straight to the delete"
    );
}

#[test]
fn move_now_reviews_then_starts_and_this_computer_stays_a_desktop() {
    let fake = Fake::new();
    let mut moving = context(ActionId::MoveNow, status("home_status_ready"));
    moving.passphrase = Some(typed());
    moving.shown = vec![
        ReadinessId::StaysHere,
        ReadinessId::CutoverGap,
        ReadinessId::ServesNothing,
    ];
    let outcome = perform(&fake, &moving);
    assert_eq!(
        fake.calls(),
        [
            Call::Readiness(CANDIDATE.into()),
            Call::Start(CANDIDATE.into(), MoveFrom::ThisComputer)
        ]
    );
    assert_eq!(
        outcome.move_started.as_deref(),
        Some("Fermix restarts in the device role and moves this home to north-box; keep the recovery passphrase somewhere safe.")
    );
    assert!(
        outcome.passphrase_spent,
        "the passphrase leaves memory once the move starts"
    );
}

#[test]
fn move_now_stops_once_at_a_warning_the_screen_did_not_show() {
    let fake = Fake::new();
    let mut moving = context(ActionId::MoveNow, status("home_status_ready"));
    moving.passphrase = Some(typed());
    moving.shown = vec![ReadinessId::StaysHere];
    let outcome = perform(&fake, &moving);
    assert_eq!(fake.calls(), [Call::Readiness(CANDIDATE.into())]);
    let reviewed: Vec<ReadinessId> = outcome
        .reviewed
        .expect("the review is shown")
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(
        reviewed,
        [
            ReadinessId::StaysHere,
            ReadinessId::CutoverGap,
            ReadinessId::ServesNothing
        ]
    );
    assert!(outcome.move_started.is_none() && !outcome.passphrase_spent);
}

#[test]
fn move_now_asks_for_the_passphrase_where_none_is_held() {
    let fake = Fake::new();
    let outcome = perform(
        &fake,
        &context(ActionId::MoveNow, status("home_status_ready")),
    );
    assert!(fake.calls().is_empty());
    assert!(outcome.asks_passphrase);
}

/// During a replace the move is the server's, to the candidate its code enrolled, and the
/// server being replaced keeps nothing.
#[test]
fn move_now_on_a_desktop_replacing_its_server_moves_the_server_and_keeps_nothing() {
    let replacing: HomeStatus = golden("home_status_on", |s| {
        s["primary_home"] = result("home_status_ready");
        s["summary"] = json!({"state": "ready", "sentence": "Your new server is ready.",
            "action": action("move_now", "Move now"), "detail": {}});
    });
    let fake = Fake::new();
    let mut moving = context(ActionId::MoveNow, replacing);
    moving.passphrase = Some(typed());
    moving.shown = vec![
        ReadinessId::StaysHere,
        ReadinessId::CutoverGap,
        ReadinessId::ServesNothing,
    ];
    perform(&fake, &moving);
    assert_eq!(
        fake.calls(),
        [
            Call::Readiness(CANDIDATE.into()),
            Call::Start(CANDIDATE.into(), MoveFrom::TheServer)
        ]
    );
}

#[test]
fn an_unconfirmed_start_is_a_notice_and_the_move_is_followed() {
    let unconfirmed = "The server fermix-vm did not confirm in time whether the move started.";
    let fake = Fake::new().refusing("start", refusal("unconfirmed", unconfirmed));
    let mut moving = context(ActionId::MoveNow, status("home_status_ready"));
    moving.passphrase = Some(typed());
    moving.shown = vec![
        ReadinessId::StaysHere,
        ReadinessId::CutoverGap,
        ReadinessId::ServesNothing,
    ];
    let outcome = perform(&fake, &moving);
    assert_eq!(outcome.notice.as_deref(), Some(unconfirmed));
    assert_eq!(outcome.move_started.as_deref(), Some(unconfirmed));
    assert!(outcome.refusal.is_none());
}

#[test]
fn check_again_reviews_turn_on_and_restart_power_the_server_then_read_hetzner() {
    let fake = Fake::new();
    perform(
        &fake,
        &context(ActionId::CheckAgain, status("home_status_ready")),
    );
    assert_eq!(fake.calls(), [Call::Readiness(CANDIDATE.into())]);
    for (id, power) in [
        (ActionId::TurnOn, PowerAction::On),
        (ActionId::RestartServer, PowerAction::Reset),
    ] {
        let fake = Fake::new();
        perform(&fake, &context(id, status("home_status_problem")));
        assert_eq!(
            fake.calls(),
            [
                Call::Power(power),
                Call::JobGet("job:Hz9Rst2b".into()),
                Call::Resources
            ],
            "{id:?}"
        );
    }
}

#[test]
fn start_again_deletes_then_creates_the_same_size_in_the_same_place() {
    let fake = Fake::new();
    perform(
        &fake,
        &context(ActionId::StartAgain, status("home_status_creating")),
    );
    assert_eq!(
        fake.calls(),
        [
            Call::Resources,
            Call::Delete,
            Call::JobGet(DELETE_JOB.into()),
            Call::Create(cpx("cpx32", Location::Hel1), false),
        ]
    );
    let fake = Fake::new();
    perform(
        &fake,
        &context(ActionId::StartAgain, status("home_status_problem")),
    );
    assert_eq!(
        fake.calls().last(),
        Some(&Call::Create(cpx("cpx32", Location::Hel1), true)),
        "on a linked desktop the new server replaces the old"
    );
    let fake = Fake::new();
    fake.job_gets.borrow_mut().push_back(job(
        "home_provision_delete",
        "failed",
        json!({"code": "upstream", "sentence": "Hetzner did not delete the server."}),
    ));
    let outcome = perform(
        &fake,
        &context(ActionId::StartAgain, status("home_status_creating")),
    );
    assert_eq!(
        fake.calls(),
        [
            Call::Resources,
            Call::Delete,
            Call::JobGet(DELETE_JOB.into())
        ],
        "nothing is created while the old server stands"
    );
    assert_eq!(
        outcome.refusal.as_deref(),
        Some("Hetzner did not delete the server.")
    );
}

#[test]
fn create_in_creates_the_chosen_size_where_hetzner_has_room() {
    let no_room = summarised(json!({"state": "problem",
        "sentence": "Falkenstein has no CPX22 right now.",
        "action": action("create_in", "Create in Nuremberg"),
        "detail": {"kind": "no_capacity", "next_location": "nbg1", "alternatives": []}}));
    let fake = Fake::new();
    perform(&fake, &context(ActionId::CreateIn, no_room));
    assert_eq!(
        fake.calls(),
        [
            Call::Options,
            Call::Create(cpx("cpx22", Location::Nbg1), false)
        ]
    );
}

#[test]
fn delete_back_up_and_replace_make_their_one_call() {
    let fake = Fake::new();
    perform(
        &fake,
        &context(ActionId::DeleteServer, status("home_status_creating")),
    );
    assert_eq!(
        fake.calls(),
        [Call::Delete, Call::JobGet(DELETE_JOB.into())]
    );
    let fake = Fake::new();
    perform(
        &fake,
        &context(ActionId::BackUpNow, status("home_status_on")),
    );
    assert_eq!(fake.calls(), [Call::SnapshotsRun]);
    let fake = Fake::new();
    let mut replace = context(ActionId::ReplaceServer, status("home_status_on"));
    replace.chosen = Some(cpx("cpx32", Location::Hel1));
    perform(&fake, &replace);
    assert_eq!(
        fake.calls(),
        [Call::Create(cpx("cpx32", Location::Hel1), true)]
    );
}

#[test]
fn a_refusal_or_a_failed_job_is_said_in_the_daemons_words() {
    let owner = "Only the owner can give Fermix a Hetzner token or create, restart or delete its \
                 server: use the Fermix app.";
    let fake = Fake::new().refusing("create", refusal("unavailable", owner));
    let outcome = perform(
        &fake,
        &context(ActionId::CreateServer, status("home_status_creating")),
    );
    assert_eq!(outcome.refusal.as_deref(), Some(owner));
    let fake = Fake::new();
    fake.job_gets.borrow_mut().push_back(job(
        "home_provision_power",
        "failed",
        json!({"code": "upstream", "sentence": "Hetzner did not turn the server on."}),
    ));
    let outcome = perform(
        &fake,
        &context(ActionId::TurnOn, status("home_status_problem")),
    );
    assert_eq!(
        outcome.refusal.as_deref(),
        Some("Hetzner did not turn the server on.")
    );
}

#[test]
fn a_job_is_followed_only_as_long_as_its_budget() {
    let fake = Fake::new();
    let running = job("home_provision_delete", "running", Value::Null);
    for _ in 0..1000 {
        fake.job_gets.borrow_mut().push_back(running.clone());
    }
    let outcome = perform(
        &fake,
        &context(ActionId::DeleteServer, status("home_status_creating")),
    );
    let polls = fake
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::JobGet(_)))
        .count();
    assert_eq!(polls, 450, "fifteen minutes at two seconds");
    assert_eq!(*fake.pauses.borrow(), 450);
    assert_eq!(outcome.refusal.as_deref(), Some(JOB_OVERDUE));
}

// ---- The passphrase, the token and the code -------------------------------------------------

#[test]
fn the_passphrase_is_typed_twice_held_for_the_move_and_never_kept_after() {
    let mut model = model_on(status("home_status_ready"));
    assert_eq!(
        model.passphrase_typed("invented-one-words", "invented-two-words"),
        None
    );
    assert_eq!(
        model.passphrase_refusal.as_deref(),
        Some(PASSPHRASE_MISMATCH)
    );
    let agreed = model
        .passphrase_typed("invented-harbor-juniper", "invented-harbor-juniper")
        .expect("the two agree");
    model.passphrase_set(agreed, Ok(()));
    assert!(model.passphrase_refusal.is_none());
    let Press::Run(moving) = model.press(ActionId::MoveNow) else {
        panic!("Move now runs")
    };
    assert!(moving.passphrase.is_some(), "the move takes it again");
    model.finished(Outcome {
        move_started: Some("Moving.".into()),
        passphrase_spent: true,
        ..Outcome::default()
    });
    let Press::Run(again) = model.press(ActionId::TryAgain) else {
        panic!("Try again runs")
    };
    assert!(again.passphrase.is_none(), "dropped once the move started");
}

#[test]
fn move_now_without_a_held_passphrase_asks_for_it_and_then_moves() {
    let mut model = model_on(status("home_status_ready"));
    let Press::Run(first) = model.press(ActionId::MoveNow) else {
        panic!("runs")
    };
    assert!(first.passphrase.is_none());
    model.finished(Outcome {
        asks_passphrase: true,
        ..Outcome::default()
    });
    assert!(model.asks_move_passphrase && model.working.is_none());
    let moving = model
        .move_with_passphrase("invented-harbor-juniper", "invented-harbor-juniper")
        .expect("the two agree");
    assert!(!model.asks_move_passphrase);
    assert!(moving.passphrase.is_some() && moving.action == ActionId::MoveNow);
}

#[test]
fn replace_creates_from_the_sheets_choice() {
    let mut model = model_on(status("home_status_on"));
    assert_eq!(model.press(ActionId::ReplaceServer), Press::Drawn);
    assert!(model.replacing);
    let replace = model
        .replace_with(cpx("cpx32", Location::Hel1))
        .expect("runs");
    assert_eq!(replace.action, ActionId::ReplaceServer);
    assert_eq!(replace.chosen, Some(cpx("cpx32", Location::Hel1)));
    assert!(!model.replacing);
}

// ---- The notifications ----------------------------------------------------------------------

fn reading(status: HomeStatus, at: i64) -> Reading {
    Reading { status, at }
}

#[test]
fn the_server_ready_and_the_move_done_are_each_told_once() {
    let ready = reading(status("home_status_ready"), NOW);
    let creating = reading(status("home_status_creating"), NOW - 60);
    let told = notice(Some(&creating), &ready, false).expect("ready is told");
    assert_eq!(told.kind, NoticeKind::ServerReady);
    assert_eq!(
        told.body,
        "Your server is ready for the move. There is no way back to this Mac in this version."
    );
    assert_eq!(notice(Some(&ready), &ready, false), None, "once");
    assert_eq!(notice(None, &ready, false), None, "a launch raises nothing");
    let on = reading(status("home_status_on"), NOW + 60);
    assert_eq!(
        notice(Some(&ready), &on, false),
        None,
        "not from ready alone"
    );
    assert_eq!(
        notice(Some(&ready), &on, true).map(|n| n.kind),
        Some(NoticeKind::MoveDone),
        "this app's own move, across the restart"
    );
    let moving: HomeStatus = summarised(json!({"state": "moving", "sentence": "Moving.",
        "action": null, "detail": {}}));
    assert_eq!(
        notice(Some(&reading(moving, NOW)), &on, false).map(|n| n.kind),
        Some(NoticeKind::MoveDone)
    );
    assert_eq!(notice(Some(&on), &on, false), None);
}

#[test]
fn an_unreachable_server_and_one_set_up_again_are_told_as_they_begin() {
    let on = reading(status("home_status_on"), NOW);
    let reach = reading(status("home_status_problem"), NOW + 60);
    let told = notice(Some(&on), &reach, false).expect("told");
    assert_eq!(told.kind, NoticeKind::CannotReach);
    assert_eq!(notice(Some(&reach), &reach, false), None);
    let waited = reading(
        problem(
            "waited_too_long",
            action("start_again", "Start again"),
            json!([]),
        ),
        NOW + 120,
    );
    assert_eq!(
        notice(Some(&on), &waited, false).map(|n| n.kind),
        Some(NoticeKind::SetUpAgain)
    );
    let other = reading(
        problem("turned_off", action("turn_on", "Turn it on"), json!([])),
        NOW + 120,
    );
    assert_eq!(
        notice(Some(&on), &other, false),
        None,
        "not every problem is told"
    );
}

fn old_backup(seen: &str, at: i64) -> Reading {
    let mut status = problem(
        "backup_old",
        action("back_up_now", "Back up now"),
        json!([]),
    );
    status.summary.as_mut().expect("a summary").detail.seen_at = Some(seen.into());
    reading(status, at)
}

#[test]
fn an_old_backup_is_told_once_it_passes_a_week_and_once_for_that_backup() {
    let taken = "2031-05-04T12:00:00Z";
    let six_days = old_backup(taken, NOW + 6 * DAY);
    let eight_days = old_backup(taken, NOW + 8 * DAY);
    assert_eq!(
        notice(
            Some(&six_days),
            &old_backup(taken, NOW + 6 * DAY + 60),
            false
        ),
        None
    );
    assert_eq!(
        notice(Some(&six_days), &eight_days, false).map(|n| n.kind),
        Some(NoticeKind::BackupOld)
    );
    let nine_days = old_backup(taken, NOW + 9 * DAY);
    assert_eq!(
        notice(Some(&eight_days), &nine_days, false),
        None,
        "once for that backup"
    );
    let on = reading(status("home_status_on"), NOW);
    assert_eq!(
        notice(Some(&on), &eight_days, false).map(|n| n.kind),
        Some(NoticeKind::BackupOld),
        "a week old as the problem begins"
    );
    let newer = old_backup("2031-05-06T12:00:00Z", NOW + 10 * DAY);
    assert_eq!(
        notice(Some(&nine_days), &newer, false).map(|n| n.kind),
        Some(NoticeKind::BackupOld),
        "a newer backup that grew old is told again"
    );
}

#[test]
fn each_kind_of_notice_replaces_only_its_own() {
    let kinds = [
        NoticeKind::ServerReady,
        NoticeKind::MoveDone,
        NoticeKind::CannotReach,
        NoticeKind::SetUpAgain,
        NoticeKind::BackupOld,
    ];
    let ids: std::collections::BTreeSet<&str> = kinds.iter().map(|k| k.id()).collect();
    assert_eq!(ids.len(), kinds.len());
    assert!(ids.iter().all(|id| id.starts_with("always-on-")));
}

// ---- How often the summary is read ----------------------------------------------------------

#[test]
fn the_summary_is_read_briskly_while_something_moves_and_rarely_when_nothing_is_set_up() {
    let secs = Duration::from_secs;
    let creating = status("home_status_creating");
    let on = status("home_status_on");
    let off = summarised(json!({"state": "off", "sentence": "Keep Fermix running.",
        "action": action("set_up", "Set up"), "detail": {}}));
    assert_eq!(wait(Some(&creating), false, true, false), secs(3));
    assert_eq!(wait(Some(&creating), false, false, false), secs(15));
    assert_eq!(wait(Some(&on), false, true, false), secs(10));
    assert_eq!(wait(Some(&on), false, false, false), secs(60));
    assert_eq!(wait(Some(&off), false, false, false), secs(300));
    assert_eq!(wait(Some(&on), true, false, false), secs(300));
    assert_eq!(wait(None, true, false, true), secs(3), "the app's own move");
    let mut model = Model::default();
    assert_eq!(model.next_wait(), secs(60), "until hello lists the methods");
    model.served(served());
    model.answered(on, NOW);
    assert_eq!(model.next_wait(), secs(60));
    model.pane_shown();
    assert_eq!(model.next_wait(), secs(10));
}

#[test]
fn opening_the_pane_reads_hetzner_where_the_summary_rests_on_it() {
    let mut model = model_on(status("home_status_problem"));
    assert!(model.needs_reconcile());
    let mut on = model_on(status("home_status_on"));
    assert!(!on.needs_reconcile());
    on.served(without_hetzner());
    let mut creating = model_on(status("home_status_creating"));
    assert!(creating.needs_reconcile());
    creating.served(without_hetzner());
    assert!(
        !creating.needs_reconcile(),
        "not where the engine lacks the provisioner"
    );
    model.pane_shown();
    assert!(model.pane_visible);
}

#[test]
fn the_summary_states_this_app_draws_are_every_one_the_engine_answers() {
    for state in [
        SummaryState::Off,
        SummaryState::Connect,
        SummaryState::Choose,
        SummaryState::Creating,
        SummaryState::Ready,
        SummaryState::Moving,
        SummaryState::On,
        SummaryState::Undone,
        SummaryState::Retired,
        SummaryState::Candidate,
        SummaryState::Primary,
    ] {
        let word = format!("{state:?}").to_lowercase();
        let shown = summarised(json!({"state": word, "sentence": "Where it stands.",
            "action": null, "detail": {}}));
        let screen = screen_of(&shown, &Local::default());
        assert_ne!(screen.kind, ScreenKind::Unknown, "{state:?}");
        assert_eq!(screen.sentence, "Where it stands.");
    }
}
