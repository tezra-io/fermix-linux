//! The home half of the vendored management contract (M75 lean v1): every `home.*` golden decodes
//! through the app's own types, and each call the Always On pane makes is sent by the app's own
//! client to a fake daemon answering with the golden, so the request it writes is held to the
//! contract's request fixture too. A drift in the engine fails here rather than on a call.

use fermix_client::frame::{read_frame, write_frame};
use fermix_client::home::{
    passphrase, unconfirmed, ActionId, Candidate, EnrollmentKind, EnrollmentState, ItemStatus,
    LinkStatus, MoveFrom, ProblemKind, ReadinessId, Role, Snapshots, SummaryState,
};
use fermix_client::management::{CallError, Management};
use fermix_client::model::{JobStatus, JobView};
use fermix_client::provision::{
    Holder, Location, Pick, PowerAction, ProvisionState, ReplaceState, ResourceKind, TokenState,
};
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
const CANDIDATE: &str = "cand-5e2a9c1f7b3d4086";
/// The other-project refusal, in the engine's words (`Home.Provision.error_sentence/1`).
const OTHER_PROJECT: &str = "This token is for another Hetzner project than the server Fermix \
    created, so paste a token from that project.";

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
    seen.expect("answered")
}

#[test]
fn every_status_golden_says_where_always_on_stands() {
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
        let status: fermix_client::home::HomeStatus = decoded(name);
        let summary = status.summary.expect("every golden carries a summary");
        assert_eq!(summary.state, state, "{name}");
        assert_eq!(summary.action.map(|a| a.id), action, "{name}");
        assert!(!summary.sentence.is_empty(), "{name}");
        assert_eq!(status.version.as_deref(), Some("1.2.3"), "{name}");
    }
}

#[test]
fn the_status_is_asked_with_no_params_and_answers_this_homes_picture() {
    exchange("home_status", "home_status", |m| {
        let status = m.home_status().expect("the golden decodes");
        assert_eq!(status.role, Role::Primary);
        assert_eq!(status.epoch, Some(4));
        let window = status.enrollment.expect("a window");
        assert_eq!(window.state, EnrollmentState::Used);
        assert_eq!(window.kind, Some(EnrollmentKind::Manual));
        assert_eq!(window.candidate_id.as_deref(), Some(CANDIDATE));
        assert_eq!(status.candidates[0].candidate_id, CANDIDATE);
        let disk = status.disk.expect("a primary's disk");
        assert!(!disk.nearly_full);
        assert_eq!(status.moving.expect("a move").phase, "aborted");
    });
}

#[test]
fn a_created_servers_window_is_provisioned_and_names_the_candidate_it_enrolled() {
    let creating: fermix_client::home::HomeStatus = decoded("home_status_creating");
    let window = creating.enrollment.expect("a window");
    assert_eq!(window.kind, Some(EnrollmentKind::Provisioned));
    assert_eq!(window.state, EnrollmentState::Open);
    assert_eq!(window.candidate_id, None, "nothing enrolled yet");
    let ready: fermix_client::home::HomeStatus = decoded("home_status_ready");
    let window = ready.enrollment.expect("a window");
    assert_eq!(window.state, EnrollmentState::Used);
    assert_eq!(window.candidate_id.as_deref(), Some(CANDIDATE));
}

#[test]
fn the_ready_detail_holds_the_servers_facts_its_warnings_and_its_wait() {
    let ready: fermix_client::home::HomeStatus = decoded("home_status_ready");
    let detail = ready.summary.expect("a summary").detail;
    let server = detail.server.expect("the server's facts");
    assert_eq!(server.name.as_deref(), Some("fermix-general-a3f9"));
    assert_eq!(server.size.as_deref(), Some("CPX22"));
    assert_eq!(server.location.as_deref(), Some("Falkenstein"));
    assert_eq!(server.ipv4.as_deref(), Some("203.0.113.7"));
    assert_eq!(server.system.as_deref(), Some("linux x86_64"));
    let readiness = detail.readiness.expect("the review's items");
    assert_eq!(readiness[0].id, ReadinessId::StaysHere);
    assert_eq!(readiness[0].status, ItemStatus::Warn);
    assert_eq!(detail.waits_until.as_deref(), Some("2031-05-05T11:00:00Z"));
}

#[test]
fn the_on_detail_holds_the_server_its_backups_the_disk_and_two_actions() {
    let on: fermix_client::home::HomeStatus = decoded("home_status_on");
    assert_eq!(on.role, Role::Device);
    let primary = on
        .primary
        .as_ref()
        .expect("the server this desktop reaches");
    assert_eq!(primary.status, LinkStatus::Connected);
    let home = on
        .primary_home
        .as_deref()
        .expect("the server's own picture");
    assert_eq!(home.role, Role::Primary);
    let detail = on.summary.expect("a summary").detail;
    let ids: Vec<ActionId> = detail
        .actions
        .expect("Details offers actions")
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(ids, [ActionId::BackUpNow, ActionId::ReplaceServer]);
    let backups = detail.backups.expect("backups");
    assert_eq!(
        (backups[0].place.as_str(), backups[0].at.as_str()),
        ("S3", "2031-05-04T10:00:00Z")
    );
    assert_eq!(detail.copies_bytes, Some(144_633));
    assert_eq!(detail.disk_used, Some(0.42));
    assert!(detail.data.expect("what lives there").contains("Hetzner"));
}

#[test]
fn a_problem_names_its_kind_when_it_was_seen_and_its_alternatives() {
    let problem: fermix_client::home::HomeStatus = decoded("home_status_problem");
    assert_eq!(
        problem.primary_missing.as_deref(),
        Some("The server fermix-general-a3f9 is unreachable.")
    );
    let summary = problem.summary.expect("a summary");
    assert_eq!(summary.problem(), Some(ProblemKind::CannotReach));
    assert_eq!(
        summary.detail.seen_at.as_deref(),
        Some("2031-05-04T11:50:00Z")
    );
    assert_eq!(summary.detail.alternatives.as_deref(), Some(&[][..]));
    assert_eq!(summary.detail.note, None);
}

/// Every id, state, kind and location the contract publishes decodes as one this app knows, so
/// a word the engine adds fails here until the app has a place for it.
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
    for id in words("/homeSummaryAction/properties/id/enum") {
        let known: ActionId = serde_json::from_value(id.clone()).expect("decodes");
        assert_ne!(known, ActionId::Unknown, "{id}");
    }
    for kind in words("/homeSummaryDetail/properties/kind/enum") {
        let known: ProblemKind = serde_json::from_value(kind.clone()).expect("decodes");
        assert_ne!(known, ProblemKind::Unknown, "{kind}");
    }
    for place in words("/homeSummaryDetail/properties/next_location/enum") {
        let known: Location = serde_json::from_value(place.clone()).expect("decodes");
        assert!(known.wire().is_some(), "{place}");
    }
    let unknown: ActionId = serde_json::from_value(json!("fly_away")).expect("decodes");
    assert_eq!(
        unknown,
        ActionId::Unknown,
        "a newer engine's action is no error"
    );
}

/// The readiness ids the contract names, each one this app knows; an id it does not know is
/// kept as it came, so its sentence is still shown as it is.
#[test]
fn every_readiness_id_the_contract_names_is_known_and_a_new_one_keeps_its_word() {
    let schema: Value = serde_json::from_str(SCHEMA).expect("the schema is JSON");
    let description = schema["$defs"]["homeReadinessItem"]["description"]
        .as_str()
        .expect("the item says its ids");
    let listed = description
        .split_once("id is ")
        .and_then(|(_, rest)| rest.split_once(';'))
        .map(|(ids, _)| ids)
        .expect("the description lists the ids");
    let ids: Vec<&str> = listed
        .split([',', ' '])
        .filter(|w| !w.is_empty() && *w != "or")
        .collect();
    assert_eq!(ids.len(), 11, "{listed}");
    for id in ids {
        let known = ReadinessId::from(id.to_owned());
        assert!(!matches!(known, ReadinessId::Other(_)), "{id} is not known");
    }
    let later = ReadinessId::from("tide_tables".to_owned());
    assert_eq!(later, ReadinessId::Other("tide_tables".into()));
}

#[test]
fn the_review_is_asked_for_one_candidate_and_answers_every_item_by_its_id() {
    exchange("home_move_readiness", "home_move_readiness", |m| {
        let review = m.home_move_readiness(CANDIDATE).expect("decodes");
        assert!(review.ready);
        let ids: Vec<ReadinessId> = review.items.iter().map(|i| i.id.clone()).collect();
        assert_eq!(
            ids,
            [
                ReadinessId::KeychainSecrets,
                ReadinessId::EngineVersion,
                ReadinessId::Disk,
                ReadinessId::S3Size,
                ReadinessId::Plugins,
                ReadinessId::StaysHere,
                ReadinessId::Whatsapp,
                ReadinessId::Slack,
                ReadinessId::CodingAgents,
                ReadinessId::CutoverGap,
                ReadinessId::ServesNothing,
            ]
        );
    });
}

#[test]
fn a_move_from_this_computer_keeps_it_and_a_replace_keeps_nothing() {
    let typed = passphrase(
        "invented-harbor-juniper-lantern",
        "invented-harbor-juniper-lantern",
    )
    .expect("the two agree");
    exchange("home_move_start", "home_move_start", |m| {
        let started = m
            .home_move_start(CANDIDATE, &typed, MoveFrom::ThisComputer)
            .expect("decodes");
        assert!(started.keep_as_desktop);
        assert_eq!(started.candidate.name, "north-box");
    });
    exchange(
        "home_move_start_replacing_the_server",
        "home_move_start",
        |m| {
            m.home_move_start(CANDIDATE, &typed, MoveFrom::TheServer)
                .expect("decodes");
        },
    );
}

#[test]
fn the_passphrase_and_the_code_are_never_printed() {
    let typed = passphrase(
        "invented-lighthouse-meridian",
        "invented-lighthouse-meridian",
    )
    .expect("the two agree");
    assert!(!format!("{typed:?}").contains("lighthouse"));
    exchange("home_enroll_open", "home_enroll_open", |m| {
        let code = m.home_enroll_open().expect("decodes");
        assert!(code.text().starts_with("fxenroll1."));
        assert_eq!(code.expires_at, "2026-10-27T09:40:00Z");
        assert!(!format!("{code:?}").contains("fxenroll1."));
    });
    exchange(
        "home_snapshots_passphrase",
        "home_snapshots_passphrase",
        |m| {
            let set = m.home_snapshots_passphrase(&typed).expect("decodes");
            assert_eq!(set.key_id, "08c15ca36dd935c4");
            assert!(set.snapshots.recipient_set);
        },
    );
    assert_eq!(
        passphrase("one-thing-entirely", "another-thing"),
        Err(Some(fermix_client::home::PASSPHRASE_MISMATCH))
    );
    assert_eq!(passphrase("", "typed-only-once"), Err(None));
}

#[test]
fn back_up_now_starts_a_snapshot() {
    exchange("home_snapshots_run", "home_snapshots_run", |m| {
        let run = m.home_snapshots_run().expect("decodes");
        assert!(run.started && run.snapshots.running);
    });
}

/// The two home reads no call of this app makes still decode through its types, and the one
/// method no surface calls is named, so every home golden is read here.
#[test]
fn no_home_golden_goes_unread() {
    let snapshots: Snapshots = decoded("home_snapshots_status");
    assert!(snapshots.configured);
    #[derive(Deserialize)]
    struct Held {
        candidates: Vec<Candidate>,
    }
    let held: Held = decoded("home_candidates_list");
    assert_eq!(held.candidates[0].candidate_id, CANDIDATE);
    let job: JobView = decoded("job_get_home_provision_completed");
    assert_eq!(
        (job.kind.as_str(), job.status),
        ("home_provision", JobStatus::Completed)
    );
    let read: BTreeSet<&str> = [
        "home_status",
        "home_status_creating",
        "home_status_ready",
        "home_status_candidate",
        "home_status_server",
        "home_status_on",
        "home_status_problem",
        "home_snapshots_status",
        "home_snapshots_run",
        "home_snapshots_passphrase",
        "home_enroll_open",
        "home_candidates_list",
        "home_move_readiness",
        "home_move_start",
        "home_provision_connect",
        "home_provision_options",
        "home_provision_create",
        "home_provision_power",
        "home_provision_resources",
        "home_provision_delete",
    ]
    .into_iter()
    .collect();
    // A desktop's engine forwards it to the server it replaces; no surface calls it.
    let uncalled = ["home_enroll_withdraw"];
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

#[test]
fn a_pasted_token_goes_to_hetzner_and_answers_the_provisioners_view() {
    let token =
        fermix_client::provision::Token::pasted(" invented-hetzner-token-6d1f0c9a2b7e4f3a\n")
            .expect("a token was pasted");
    assert!(!format!("{token:?}").contains("invented-hetzner"));
    exchange("home_provision_connect", "home_provision_connect", |m| {
        let view = m.provision_connect(&token).expect("decodes");
        assert_eq!(view.state, ProvisionState::Connected);
        assert_eq!(view.token, TokenState::Accepted);
        assert_eq!(view.other_servers, Some(false));
        assert!(view.replace.is_none() && view.resources.is_empty());
    });
    assert!(fermix_client::provision::Token::pasted("  \n").is_none());
}

#[test]
fn the_options_name_every_offer_the_recommendation_and_more_room() {
    exchange("home_provision_options", "home_provision_options", |m| {
        let options = m.provision_options().expect("decodes");
        assert_eq!(options.currency, "EUR");
        let recommended = options.recommended.clone().expect("a recommendation");
        assert_eq!(
            recommended,
            Pick {
                size: "cpx22".into(),
                location: Location::Fsn1
            }
        );
        let offer = options.offer(&recommended).expect("its offer");
        assert_eq!((offer.cores, offer.memory_gb, offer.disk_gb), (2, 4.0, 80));
        assert_eq!(
            (offer.monthly.as_str(), offer.ipv4_monthly.as_str()),
            ("19.49", "0.60")
        );
        assert_eq!(offer.total_monthly, "20.09");
        assert_eq!(options.city(Location::Hel1), Some("Helsinki"));
        assert_eq!(options.more_room.map(|p| p.size), Some("cpx32".into()));
        assert_eq!(
            options.sentence,
            "Recommended: CPX22 in Falkenstein, €20 a month."
        );
    });
}

#[test]
fn a_create_sends_its_type_and_location_and_a_replace_says_so() {
    let pick = Pick {
        size: "cpx22".into(),
        location: Location::Fsn1,
    };
    exchange("home_provision_create", "home_provision_create", |m| {
        let job = m.provision_create(&pick, false).expect("a job");
        assert_eq!(job.kind, "home_provision");
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.budget_ms, 900_000);
    });
    let replacement = Pick {
        size: "cpx32".into(),
        location: Location::Hel1,
    };
    exchange(
        "home_provision_create_replace",
        "home_provision_create",
        |m| {
            m.provision_create(&replacement, true).expect("a job");
        },
    );
}

#[test]
fn power_resources_and_delete_answer_as_the_contract_says() {
    exchange("home_provision_power", "home_provision_power", |m| {
        let job = m.provision_power(PowerAction::Reset).expect("a job");
        assert_eq!(job.kind, "home_provision");
    });
    exchange(
        "home_provision_resources",
        "home_provision_resources",
        |m| {
            let view = m.provision_resources().expect("decodes");
            assert_eq!(view.state, ProvisionState::Created);
            assert_eq!(view.checked, Some(true));
            let server = view.server.expect("the new server");
            assert_eq!(server.name, "fermix-general-b7c2");
            assert_eq!(server.location, Location::Hel1);
            assert_eq!(server.power.as_deref(), Some("running"));
            let replace = view.replace.expect("a replace waits");
            assert_eq!(replace.state, ReplaceState::Waiting);
            assert_eq!(replace.current, Holder::Old);
            assert_eq!(
                replace.old.expect("the old server").name,
                "fermix-general-a3f9"
            );
            assert_eq!(view.resources.len(), 8);
            let replaced = view.resources.iter().filter(|r| r.replaced).count();
            assert_eq!(replaced, 4);
            assert_eq!(view.resources[2].kind, ResourceKind::PrimaryIp);
        },
    );
    exchange("home_provision_delete", "home_provision_delete", |m| {
        let job = m.provision_delete().expect("a job");
        assert_eq!(job.status, JobStatus::Running);
    });
}

/// The owner's decisions and the other-project token come back as the daemon's own sentence,
/// which the pane shows under what was asked.
#[test]
fn the_owners_decisions_and_another_projects_token_are_said_in_the_daemons_words() {
    let pick = Pick {
        size: "cpx32".into(),
        location: Location::Hel1,
    };
    let err = refused_with("unavailable_provision_owner_decision", |m| {
        m.provision_create(&pick, true).expect_err("refused")
    });
    let CallError::Refused(refusal) = err else {
        panic!("a refusal")
    };
    assert_eq!(refusal.code, "unavailable");
    assert_eq!(
        refusal.sentence,
        "Only the owner can give Fermix a Hetzner token or create, restart or delete its \
         server: use the Fermix app."
    );
    let other = json!({"request_id": "x", "error": {"code": "invalid_params",
        "message": "Request parameters are invalid.",
        "details": {"field": "token", "sentence": OTHER_PROJECT}}});
    let token = fermix_client::provision::Token::pasted("invented-other-project-token-0042")
        .expect("pasted");
    let mut seen = None;
    serve(other, |m| seen = Some(m.provision_connect(&token)));
    let Some(Err(CallError::Refused(refusal))) = seen else {
        panic!("a refusal")
    };
    assert_eq!(refusal.field.as_deref(), Some("token"));
    assert_eq!(refusal.sentence, OTHER_PROJECT);
    let typed = passphrase("invented-harbor-juniper", "invented-harbor-juniper").expect("agree");
    let err = refused_with("unavailable_home_owner_decision", |m| {
        m.home_move_start(CANDIDATE, &typed, MoveFrom::TheServer)
            .expect_err("refused")
    });
    let CallError::Refused(refusal) = err else {
        panic!("a refusal")
    };
    assert!(refusal
        .sentence
        .starts_with("Only the owner can move Fermix"));
}

/// A start the server had but did not confirm is no failure: its sentence is the notice, and
/// nothing else answers as unconfirmed.
#[test]
fn an_unconfirmed_start_is_told_apart_from_a_refusal() {
    let typed = passphrase("invented-harbor-juniper", "invented-harbor-juniper").expect("agree");
    let err = refused_with("unconfirmed", |m| {
        m.home_move_start(CANDIDATE, &typed, MoveFrom::TheServer)
            .expect_err("unconfirmed")
    });
    assert_eq!(
        unconfirmed(&err),
        Some(
            "The server fermix-vm did not confirm in time whether the move started. Its state \
             shows once the server answers again, and starting it again never starts a second \
             move."
        )
    );
    let err = refused_with("primary_unreachable", |m| {
        m.home_status().expect_err("refused")
    });
    assert_eq!(unconfirmed(&err), None);
    let CallError::Refused(refusal) = err else {
        panic!("a refusal")
    };
    assert_eq!(
        refusal.sentence,
        "The server fermix-vm is unreachable. Fermix keeps your message until it is back."
    );
}

/// The two move calls walk the whole home and the three Hetzner reads make up to three calls of
/// ten seconds: each gets the deadline the contract asks of a client, and every other call
/// keeps the client's own.
#[test]
fn the_long_calls_get_their_own_deadlines() {
    let client = Management::new("/nowhere/daemon.sock".into(), Duration::from_secs(10));
    for method in ["home.move.readiness", "home.move.start"] {
        assert_eq!(
            client.deadline_for(method),
            Duration::from_secs(65),
            "{method}"
        );
    }
    for method in [
        "home.provision.connect",
        "home.provision.options",
        "home.provision.resources",
    ] {
        assert_eq!(
            client.deadline_for(method),
            Duration::from_secs(35),
            "{method}"
        );
    }
    for method in ["home.status", "home.provision.create", "job.get"] {
        assert_eq!(
            client.deadline_for(method),
            Duration::from_secs(10),
            "{method}"
        );
    }
    let patient = Management::new("/nowhere/daemon.sock".into(), Duration::from_secs(90));
    assert_eq!(
        patient.deadline_for("home.move.start"),
        Duration::from_secs(90),
        "never shorter than the client's own"
    );
}
