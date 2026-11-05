//! The Cues flows end to end (M60 cues §4.4, §4.5): the app's own client against a fake daemon
//! on a real socket, answering from the vendored goldens or refusing as the daemon does, and
//! the core state the app keeps between the calls. What is drawn is what that state then holds.

use fermix_client::cues::calls::FormChoice;
use fermix_client::cues::form::{AddFlow, Arrival, SourceFormState, NAME_KEY};
use fermix_client::cues::view::{failure, read_failure, Asking, Failure};
use fermix_client::cues::wire::SecretField;
use fermix_client::cues::words;
use fermix_client::frame::{read_frame, write_frame};
use fermix_client::management::{CallError, Management};
use serde_json::{json, Value};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");

fn golden(name: &str) -> Value {
    SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"))["response"]
        .clone()
}

/// A refusal as the daemon writes it, naming `field`.
fn refusal(code: &str, field: &str, sentence: &str) -> Value {
    json!({"error": {"code": code, "message": "Request parameters are invalid.",
        "details": {"field": field, "sentence": sentence}}})
}

/// A daemon that answers one connection per envelope, in order, each with the request's own id,
/// and hands back every request it read.
struct FakeDaemon {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    handle: JoinHandle<Vec<Value>>,
}

impl FakeDaemon {
    fn start(answers: Vec<Value>) -> FakeDaemon {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let socket = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).expect("the fake daemon binds");
        let handle = thread::spawn(move || {
            answers
                .into_iter()
                .map(|answer| serve_one(&listener, answer))
                .collect()
        });
        FakeDaemon {
            _dir: dir,
            socket,
            handle,
        }
    }

    fn client(&self) -> Management {
        Management::new(self.socket.clone(), Duration::from_secs(2))
    }

    fn requests(self) -> Vec<Value> {
        self.handle.join().expect("the fake daemon ends")
    }
}

fn serve_one(listener: &UnixListener, mut answer: Value) -> Value {
    let (mut stream, _) = listener.accept().expect("the client connects");
    let request: Value = serde_json::from_slice(&read_frame(&mut stream).expect("a request"))
        .expect("the request is JSON");
    answer["request_id"] = request["request_id"].clone();
    let bytes = serde_json::to_vec(&answer).expect("the answer serializes");
    write_frame(&mut stream, &bytes).expect("the answer is written");
    request
}

/// The sentence the app draws for a refused write.
fn sentence_of(e: &CallError) -> String {
    match failure(e) {
        Failure::Sentence(sentence) => sentence,
        Failure::Missing => panic!("the daemon has the method"),
    }
}

/// The add golden's answer as the daemon gives it for a source added under `name`.
fn added_as(name: &str) -> Value {
    let mut answer = golden("cues_sources_add");
    answer["result"]["source"]["ref"] = json!(name);
    answer
}

/// A stream source named `news` with both its secrets typed, the secret first.
fn typed_stream() -> SourceFormState {
    let form = serde_json::from_value(golden("cues_sources_form_stream")["result"].clone())
        .expect("the stream form decodes");
    let mut state = SourceFormState::default();
    state.take(&form, &FormChoice::default());
    state.choose_kind("stream");
    state.take(&form, &state.request());
    state.set(NAME_KEY, json!("news"));
    state.hold_secret("secret", "the-api-secret");
    state.hold_secret("key", "the-api-key");
    state
}

/// What the app does on Add: the source, then each typed secret as the flow gives it out.
fn add(daemon: &FakeDaemon, state: &mut SourceFormState) -> Arrival {
    let (spec, secrets) = state.prepare_add().expect("ready to add");
    let client = daemon.client();
    let added = client
        .cue_source_add(spec.name.as_deref(), &spec.choice, spec.values)
        .expect("the source is added");
    let mut flow = AddFlow::added(added, secrets);
    for _ in 0..8 {
        let Some((id, value)) = flow.next_secret() else {
            return flow.arrival();
        };
        let answer = client.secret_set(&id, value.reveal());
        flow.stored(answer.err().map(|e| sentence_of(&e)));
    }
    panic!("a form has fewer than eight secrets");
}

#[test]
fn an_add_sends_the_source_then_stores_each_typed_secret_in_row_order() {
    let daemon = FakeDaemon::start(vec![
        added_as("news"),
        golden("secret_set"),
        golden("secret_set"),
    ]);
    let arrival = add(&daemon, &mut typed_stream());
    assert_eq!(arrival.at(), None, "the page opens at its top");
    assert!(arrival.unstored.is_none() && arrival.token.is_none());
    let requests = daemon.requests();
    let methods: Vec<&str> = requests
        .iter()
        .map(|r| r["method"].as_str().unwrap())
        .collect();
    assert_eq!(methods, ["cues.sources.add", "secret.set", "secret.set"]);
    let add = requests[0]["params"].to_string();
    assert!(
        !add.contains("the-api-key") && !add.contains("the-api-secret"),
        "no typed secret goes with the source"
    );
    assert_eq!(
        requests[1]["params"],
        json!({"id": "cue_source:news:key", "value": "the-api-key"}),
        "the key's row comes first"
    );
    assert_eq!(
        requests[2]["params"],
        json!({"id": "cue_source:news:secret", "value": "the-api-secret"})
    );
}

#[test]
fn the_first_secret_refused_ends_the_add_and_the_page_opens_at_it() {
    let locked = "Your keyring is locked. Unlock it and try again.";
    let daemon = FakeDaemon::start(vec![
        added_as("news"),
        refusal("secret_store_unavailable", "id", locked),
    ]);
    let arrival = add(&daemon, &mut typed_stream());
    assert_eq!(
        arrival.at(),
        Some(SecretField::Key),
        "opens at the refused secret"
    );
    assert_eq!(
        arrival.unstored,
        Some((SecretField::Key, locked.to_owned())),
        "its sentence is the daemon's"
    );
    let requests = daemon.requests();
    assert_eq!(requests.len(), 2, "the secret after it is never sent");
    assert_eq!(requests[1]["params"]["id"], "cue_source:news:key");
}

/// With no name typed, no `ref` goes with the source, and each typed secret is stored under the
/// name the daemon gave it.
#[test]
fn an_unnamed_add_stores_its_secrets_under_the_name_the_daemon_gave() {
    let daemon = FakeDaemon::start(vec![
        added_as("ycombinator"),
        golden("secret_set"),
        golden("secret_set"),
    ]);
    let mut state = typed_stream();
    state.set(NAME_KEY, json!(""));
    let arrival = add(&daemon, &mut state);
    assert!(arrival.unstored.is_none());
    let requests = daemon.requests();
    assert_eq!(requests[0]["params"].get("ref"), None, "no name is sent");
    let ids: Vec<&str> = requests[1..]
        .iter()
        .map(|r| r["params"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "cue_source:ycombinator:key",
            "cue_source:ycombinator:secret"
        ]
    );
}

/// A name of only spaces is no name: no `ref` goes with the source.
#[test]
fn a_name_of_only_spaces_sends_no_ref() {
    let daemon = FakeDaemon::start(vec![
        added_as("ycombinator"),
        golden("secret_set"),
        golden("secret_set"),
    ]);
    let mut state = typed_stream();
    state.set(NAME_KEY, json!("   "));
    add(&daemon, &mut state);
    let requests = daemon.requests();
    assert_eq!(requests[0]["method"], "cues.sources.add");
    assert_eq!(requests[0]["params"].get("ref"), None, "no name is sent");
    assert_eq!(requests[1]["params"]["id"], "cue_source:ycombinator:key");
}

/// A GitHub source added with no name hands the secret Fermix made to its page, once.
#[test]
fn a_github_add_hands_the_secret_fermix_made_to_the_page() {
    let daemon = FakeDaemon::start(vec![golden("cues_sources_add_webhook")]);
    let mut state = SourceFormState::default();
    let form = serde_json::from_value(golden("cues_sources_form")["result"].clone())
        .expect("the form decodes");
    state.take(&form, &FormChoice::default());
    state.choose_kind("webhook");
    state.take(&form, &state.request());
    let arrival = add(&daemon, &mut state);
    assert!(arrival.secret.is_some(), "shown once on the page");
    assert!(arrival.token.is_none());
    let requests = daemon.requests();
    assert_eq!(requests.len(), 1, "no secret was typed");
    assert_eq!(
        requests[0]["params"],
        json!({"preset": "github", "values": {}})
    );
}

#[test]
fn a_local_push_add_hands_its_token_to_the_page() {
    let daemon = FakeDaemon::start(vec![golden("cues_sources_add_local_push")]);
    let mut state = SourceFormState::default();
    let form = serde_json::from_value(golden("cues_sources_form")["result"].clone())
        .expect("the form decodes");
    state.take(&form, &FormChoice::default());
    state.choose_kind("local_push");
    state.take(&form, &state.request());
    state.set(NAME_KEY, json!("home-alerts"));
    let arrival = add(&daemon, &mut state);
    assert!(arrival.token.is_some(), "shown once on the page");
    let requests = daemon.requests();
    assert_eq!(requests.len(), 1, "no secret was typed");
    assert_eq!(requests[0]["params"]["kind"], "local_push");
}

#[test]
fn a_refused_switch_keeps_its_sentence_on_its_row_until_the_next_answer() {
    let refused = "This source is in use by a watcher that is running.";
    let daemon = FakeDaemon::start(vec![
        refusal("invalid_params", "ref", refused),
        golden("cues_sources_disable"),
    ]);
    let client = daemon.client();
    let key = "source:home-alerts";
    let mut asking = Asking::default();
    asking.note_refusal(key, "An older refusal.");
    let answer = client.cue_source_switch("home-alerts", false);
    asking.answered(key, answer.err().map(|e| sentence_of(&e)).as_deref());
    assert_eq!(
        asking.refusal(key),
        Some(refused),
        "the row draws the new refusal"
    );
    let answer = client.cue_source_switch("home-alerts", false);
    asking.answered(key, answer.err().map(|e| sentence_of(&e)).as_deref());
    assert_eq!(asking.refusal(key), None, "an answer takes it away");
    let methods: Vec<Value> = daemon
        .requests()
        .iter()
        .map(|r| r["method"].clone())
        .collect();
    assert_eq!(methods, ["cues.sources.disable", "cues.sources.disable"]);
}

#[test]
fn a_page_whose_row_went_says_so_and_an_older_daemon_says_it_is_older() {
    let gone = "There is no watcher named release-watch.";
    let daemon = FakeDaemon::start(vec![
        refusal("invalid_params", "name", gone),
        json!({"error": {"code": "method_not_found", "message": "No such method.", "details": {}}}),
        json!({"error": {"code": "unavailable", "message": "Unavailable.",
            "details": {"sentence": "Memory is off, so cues are off."}}}),
    ]);
    let client = daemon.client();
    let said: Vec<String> = (0..3)
        .map(|_| {
            let e = client.cue_watcher("release-watch").expect_err("refused");
            read_failure(&e, "name", words::WATCHER_GONE)
        })
        .collect();
    assert_eq!(said[0], words::WATCHER_GONE);
    assert_eq!(
        said[1],
        "Fermix is older than this app. Update the fermix package."
    );
    assert_eq!(
        said[2], "Memory is off, so cues are off.",
        "the daemon's own sentence"
    );
    let e = CallError::DaemonDown(std::io::ErrorKind::NotFound);
    assert_eq!(
        read_failure(&e, "ref", words::SOURCE_GONE),
        "Fermix is not running."
    );
    daemon.requests();
}
