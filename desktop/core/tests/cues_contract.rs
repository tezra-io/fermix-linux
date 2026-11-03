//! The cue methods against the vendored management contract (M60 cues §9, Contract): every cue
//! golden decodes through the app's own types, each vocabulary is the schema's
//! `x-cue-vocabulary`, and each of the 24 calls is sent by the app's own client to a fake daemon
//! answering with the golden, so the request it writes is held to the request fixture too.

use fermix_client::cues::calls::{FormChoice, SourceChoice};
use fermix_client::cues::wire::{
    ApprovalState, CueState, DecidedFrom, Decision, DoorState, MandateAction, MandateLoad,
    MandateMode, ProposalFilter, ProposalState, ResultStatus, SecretField, SourceAction,
    SourceKind, SourceState, WatcherFilter, WatcherState, Word,
};
use fermix_client::frame::{read_frame, write_frame};
use fermix_client::management::Management;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;

const REQUESTS: &str = include_str!("../contracts/management/fixtures/requests.jsonl");
const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const SCHEMA: &str = include_str!("../contracts/management/protocol.schema.json");
const PROPOSAL: &str = "cpr_9f2c41d07a5e6b38c1d2e4f0";

fn named(lines: &str, name: &str) -> Value {
    let found: Vec<Value> = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .filter(|v| v["name"] == name)
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one fixture named {name}");
    found.into_iter().next().expect("one")
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
/// fixture `request`: the same method, params and version.
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

fn values(pairs: Value) -> Map<String, Value> {
    pairs.as_object().expect("an object").clone()
}

#[test]
fn the_status_and_the_lists_decode_as_published() {
    exchange("cues_status", "cues_status", |m| {
        let status = m.cues_status().expect("decodes");
        assert!(status.state.is(CueState::Running));
        assert!(status.door.state.is(DoorState::NotRunning));
        assert_eq!(status.door.port, 4032);
        assert_eq!(status.counts.proposals_waiting, 1);
    });
    exchange("cues_sources_list", "cues_sources_list", |m| {
        let list = m.cue_sources().expect("decodes");
        let names: Vec<&str> = list.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["eng-blogs", "gh-hooks", "home-alerts"]);
        assert!(list.sources[1].kind.is(SourceKind::Webhook));
        assert!(list.can_add);
    });
    exchange("cues_watchers_list", "cues_watchers_list", |m| {
        let list = m.cue_watchers().expect("decodes");
        assert_eq!(list.watchers[0].name, "release-watch");
        assert_eq!(list.watchers[0].counts.offered, 3);
    });
    exchange("cues_mandates_list", "cues_mandates_list", |m| {
        let list = m.cue_mandates().expect("decodes");
        assert!(list.mandates[0].mode.is(MandateMode::Approve));
    });
    exchange("cues_proposals_list", "cues_proposals_list", |m| {
        let list = m.cue_proposals(ProposalFilter::Waiting).expect("decodes");
        assert_eq!(list.proposals[0].proposal_id, PROPOSAL);
        assert_eq!(list.waiting, 1);
    });
}

#[test]
fn a_source_reads_and_its_form_is_asked_for_by_preset_or_kind() {
    exchange("cues_sources_get", "cues_sources_get", |m| {
        let detail = m.cue_source("gh-hooks").expect("decodes");
        assert_eq!(detail.source.name, "gh-hooks");
        assert!(!detail.values.is_empty());
        assert_eq!(detail.gaps.len(), 1);
        assert_eq!(detail.rejected.len(), 1);
    });
    exchange("cues_sources_form", "cues_sources_form", |m| {
        let choice = FormChoice {
            preset: Some("github".into()),
            ..FormChoice::default()
        };
        let form = m.cue_source_form(&choice).expect("decodes");
        assert_eq!(form.kinds.len(), 4);
        assert_eq!(form.rows[0].key, "ref");
    });
    exchange(
        "cues_sources_form_stream",
        "cues_sources_form_stream",
        |m| {
            let choice = FormChoice {
                kind: Some("stream".into()),
                ..FormChoice::default()
            };
            let form = m.cue_source_form(&choice).expect("decodes");
            let url = form
                .rows
                .iter()
                .find(|r| r.key == "url")
                .expect("a url row");
            assert_eq!(
                url.example.as_deref(),
                Some("wss://news.example.com/stream")
            );
        },
    );
}

#[test]
fn the_source_writes_send_the_contracts_requests() {
    exchange("cues_sources_add", "cues_sources_add", |m| {
        let choice = SourceChoice::Preset("rss".into());
        let fields = values(json!({"url": "https://example.org/releases.atom"}));
        let added = m
            .cue_source_add("eng-blogs", &choice, fields)
            .expect("decodes");
        assert_eq!(added.token, None);
    });
    exchange("cues_sources_update", "cues_sources_update", |m| {
        let answer = m
            .cue_source_update("eng-blogs", values(json!({"daily_budget": 200})))
            .expect("decodes");
        assert_eq!(answer.source.daily_budget, 200);
    });
    exchange("cues_sources_remove", "cues_sources_remove", |m| {
        let removed = m.cue_source_remove("eng-blogs").expect("decodes");
        assert_eq!(removed.watchers, ["release-watch"]);
    });
    exchange("cues_sources_enable", "cues_sources_enable", |m| {
        assert!(
            m.cue_source_switch("home-alerts", true)
                .expect("decodes")
                .source
                .enabled
        );
    });
    exchange("cues_sources_disable", "cues_sources_disable", |m| {
        let answer = m.cue_source_switch("home-alerts", false).expect("decodes");
        assert!(!answer.source.enabled);
    });
}

/// A token and an address are handed out, and never shown by a `Debug` that could reach a log.
#[test]
fn a_handed_out_value_decodes_and_never_shows_in_debug() {
    exchange("cues_sources_add", "cues_sources_add_local_push", |m| {
        let choice = SourceChoice::Preset("rss".into());
        let fields = values(json!({"url": "https://example.org/releases.atom"}));
        let added = m
            .cue_source_add("eng-blogs", &choice, fields)
            .expect("decodes");
        let token = added.token.clone().expect("a first token");
        assert!(!token.reveal().is_empty());
        assert!(!format!("{added:?}").contains(token.reveal()));
    });
    exchange("cues_sources_token_new", "cues_sources_token_new", |m| {
        let fresh = m.cue_source_token_new("home-alerts").expect("decodes");
        assert!(!format!("{fresh:?}").contains(fresh.token.reveal()));
    });
    exchange("cues_sources_url_get", "cues_sources_url_get", |m| {
        let address = m.cue_source_address("gh-hooks").expect("decodes");
        assert!(address.path.reveal().starts_with("/cues/hooks/gh-hooks/"));
        let shown = format!("{address:?}");
        assert!(!shown.contains(address.path.reveal()));
        assert!(!shown.contains(address.url.reveal()));
    });
}

#[test]
fn a_watcher_reads_with_the_owners_own_words() {
    exchange("cues_watchers_get", "cues_watchers_get", |m| {
        let detail = m.cue_watcher("release-watch").expect("decodes").watcher;
        assert_eq!(detail.row.name, "release-watch");
        assert_eq!(
            detail.listen_for,
            ["release", "outage", "security advisory", "CVE"]
        );
        assert_eq!(detail.instructions[0].number, 1);
        assert_eq!(detail.examples.pass, 1);
    });
}

#[test]
fn the_mandate_calls_send_the_contracts_requests() {
    exchange("cues_mandates_choices", "cues_mandates_choices", |m| {
        let choices = m.cue_mandate_choices().expect("decodes");
        assert_eq!(choices.jobs[0].id, "release_watch_2");
        assert!(choices.tools[1].read_only);
    });
    let added = values(json!({
        "approve_within_s": 1800, "job": "release_watch_2", "max_cue_age_s": 900,
        "mode": "approve", "read_tools": ["github_search_issues"],
        "tools": ["github_create_issue"]
    }));
    exchange("cues_mandates_add", "cues_mandates_add", |m| {
        let answer = m
            .cue_mandate_write("release-issues", added, true)
            .expect("decodes");
        assert!(answer.mandate.approval.state == Some(Word::Known(ApprovalState::Unapproved)));
    });
    exchange("cues_mandates_update", "cues_mandates_update", |m| {
        let changed = values(json!({"approve_within_s": 3600}));
        let answer = m
            .cue_mandate_write("release-issues", changed, false)
            .expect("decodes");
        assert_eq!(answer.mandate.approve_within_s, 3600);
    });
    exchange("cues_mandates_remove", "cues_mandates_remove", |m| {
        let removed = m.cue_mandate_remove("release-issues").expect("decodes");
        assert_eq!(removed.name, "release-issues");
    });
    exchange("cues_mandates_enable", "cues_mandates_enable", |m| {
        assert!(
            m.cue_mandate_switch("release-issues", true)
                .expect("ok")
                .mandate
                .enabled
        );
    });
    exchange("cues_mandates_disable", "cues_mandates_disable", |m| {
        assert!(
            !m.cue_mandate_switch("release-issues", false)
                .expect("ok")
                .mandate
                .enabled
        );
    });
    exchange("cues_mandates_approve", "cues_mandates_approve", |m| {
        let answer = m.cue_mandate_approve("release-issues").expect("decodes");
        assert!(answer.mandate.load.state.is(MandateLoad::Active));
    });
}

#[test]
fn every_proposal_reads_with_fermixs_record_and_its_summary() {
    for golden in [
        "cues_proposals_get",
        "cues_proposals_get_confirmed",
        "cues_proposals_get_denied",
        "cues_proposals_get_expired",
        "cues_proposals_get_refused",
    ] {
        exchange("cues_proposals_get", golden, |m| {
            let card = m.cue_proposal(PROPOSAL).expect("decodes").proposal;
            assert_eq!(card.record.proposal_id, PROPOSAL, "{golden}");
            assert!(!card.summary.is_empty(), "{golden}");
        });
    }
    exchange("cues_proposals_decide", "cues_proposals_decide", |m| {
        let answer = m
            .cue_proposal_decide(PROPOSAL, Decision::Confirm)
            .expect("decodes");
        assert!(answer.proposal.state.is(ProposalState::Confirmed));
        assert!(answer.proposal.decided_from == Some(Word::Known(DecidedFrom::Management)));
    });
}

/// Every cue golden is read by one of the tests above, so a golden the engine adds fails here
/// until the app reads it.
#[test]
fn no_cue_golden_goes_unread() {
    let read: BTreeSet<&str> = [
        "cues_status",
        "cues_sources_list",
        "cues_sources_get",
        "cues_sources_form",
        "cues_sources_form_stream",
        "cues_sources_add",
        "cues_sources_add_local_push",
        "cues_sources_add_webhook",
        "cues_sources_update",
        "cues_sources_remove",
        "cues_sources_enable",
        "cues_sources_disable",
        "cues_sources_token_new",
        "cues_sources_url_get",
        "cues_watchers_list",
        "cues_watchers_get",
        "cues_mandates_list",
        "cues_mandates_choices",
        "cues_mandates_add",
        "cues_mandates_update",
        "cues_mandates_remove",
        "cues_mandates_enable",
        "cues_mandates_disable",
        "cues_mandates_approve",
        "cues_proposals_list",
        "cues_proposals_get",
        "cues_proposals_get_confirmed",
        "cues_proposals_get_denied",
        "cues_proposals_get_expired",
        "cues_proposals_get_refused",
        "cues_proposals_decide",
    ]
    .into_iter()
    .collect();
    let published: BTreeSet<String> = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .filter(|name| name.starts_with("cues_"))
        .collect();
    let read: BTreeSet<String> = read.into_iter().map(str::to_owned).collect();
    assert_eq!(published, read);
}

/// The webhook added without its secret waits for it, and its golden decodes as such.
#[test]
fn a_webhook_added_without_its_secret_says_which_one_it_waits_for() {
    exchange("cues_sources_add", "cues_sources_add_webhook", |m| {
        let choice = SourceChoice::Preset("rss".into());
        let fields = values(json!({"url": "https://example.org/releases.atom"}));
        let added = m
            .cue_source_add("eng-blogs", &choice, fields)
            .expect("decodes");
        let source = added.source;
        assert!(source.state.is(SourceState::NeedsSecret));
        assert!(source.primary_action == Some(Word::Known(SourceAction::SetSecret)));
        assert!(source.missing_secret == Some(Word::Known(SecretField::Secret)));
    });
}

fn wire<T: Serialize>(words: &[T]) -> Vec<String> {
    words
        .iter()
        .map(|w| {
            serde_json::to_value(w)
                .expect("a word serializes")
                .as_str()
                .expect("a word is a string")
                .to_owned()
        })
        .collect()
}

fn published(name: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(SCHEMA).expect("the schema is JSON");
    schema["x-cue-vocabulary"][name]
        .as_array()
        .unwrap_or_else(|| panic!("no vocabulary {name}"))
        .iter()
        .map(|w| w.as_str().expect("a word").to_owned())
        .collect()
}

/// Each enum a row's word decodes into, and the schema's own list of that enum, by pointer.
fn enum_at(pointer: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(SCHEMA).expect("the schema is JSON");
    schema
        .pointer(pointer)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no enum at {pointer}"))
        .iter()
        .filter_map(|w| w.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn each_cue_vocabulary_is_the_schemas() {
    use SourceState as S;
    let sources = [
        S::Off,
        S::NeedsSecret,
        S::NotRunning,
        S::Idle,
        S::Polling,
        S::Connected,
        S::Reconnecting,
        S::AuthFailed,
        S::OverBudget,
        S::Stopped,
        S::Unavailable,
    ];
    assert_eq!(wire(&sources), published("source_states"));
    let actions = [
        SourceAction::Enable,
        SourceAction::Disable,
        SourceAction::SetSecret,
        SourceAction::NewToken,
        SourceAction::GetUrl,
    ];
    assert_eq!(wire(&actions), published("source_actions"));
    let watchers = [
        WatcherState::On,
        WatcherState::Paused,
        WatcherState::NeedsUpdate,
        WatcherState::Held,
    ];
    assert_eq!(wire(&watchers), published("watcher_states"));
    let mandate_actions = [
        MandateAction::Approve,
        MandateAction::Enable,
        MandateAction::Disable,
    ];
    assert_eq!(wire(&mandate_actions), published("mandate_actions"));
    let proposals = [
        ProposalState::Waiting,
        ProposalState::Confirmed,
        ProposalState::Denied,
        ProposalState::Expired,
        ProposalState::Refused,
    ];
    assert_eq!(wire(&proposals), published("proposal_states"));
    use DecidedFrom as D;
    let decided = [
        D::Telegram,
        D::Signal,
        D::Whatsapp,
        D::Imessage,
        D::Companion,
        D::Mobile,
        D::Management,
    ];
    assert_eq!(wire(&decided), published("decided_from"));
}

#[test]
fn each_enum_inside_the_cue_rows_is_the_schemas() {
    let defs = "/$defs";
    let kinds = [
        SourceKind::LocalPush,
        SourceKind::Feed,
        SourceKind::Stream,
        SourceKind::Webhook,
    ];
    assert_eq!(
        wire(&kinds),
        enum_at(&format!("{defs}/cueSourceRow/properties/kind/enum"))
    );
    let fields = [
        SecretField::Token,
        SecretField::Key,
        SecretField::Secret,
        SecretField::PathSecret,
    ];
    assert_eq!(
        wire(&fields),
        enum_at(&format!("{defs}/cueSourceSecret/properties/field/enum"))
    );
    assert_eq!(
        wire(&[WatcherFilter::Llm, WatcherFilter::None]),
        enum_at(&format!("{defs}/cueWatcherRow/properties/filter/enum"))
    );
    assert_eq!(
        wire(&[MandateMode::Approve, MandateMode::Auto]),
        enum_at(&format!("{defs}/cueMandateRow/properties/mode/enum"))
    );
    let states = [
        CueState::Off,
        CueState::NeedsMemory,
        CueState::NeedsRestart,
        CueState::Running,
    ];
    let status = format!("{defs}/cues_status_result/properties");
    assert_eq!(wire(&states), enum_at(&format!("{status}/state/enum")));
    let doors = [
        DoorState::Listening,
        DoorState::Unavailable,
        DoorState::NotRunning,
    ];
    assert_eq!(
        wire(&doors),
        enum_at(&format!("{status}/door/properties/state/enum"))
    );
    let results = [ResultStatus::Ok, ResultStatus::Error, ResultStatus::Unknown];
    assert_eq!(
        wire(&results),
        enum_at(&format!(
            "{defs}/cueProposalRecord/properties/result_status/enum"
        ))
    );
}
