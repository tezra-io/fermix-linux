//! The vendored contract is the pin, and this is what makes it load-bearing.
//!
//! Every checksum is recomputed here rather than trusted, so an edit made in
//! this repository to make a client compile is a red build and not a silent
//! divergence from the engine. Every golden the engine publishes is decoded
//! through the same structs the application uses, so a shape that moved
//! upstream fails here rather than in front of a person.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fermix_desktop::management::contract;
use fermix_desktop::management::types::*;
use fermix_desktop::service::types::{
    ActionResult, DiagnosticsExport, InstallOutcome, Restart as CliRestart, ServiceCode,
    ServiceStatus,
};
use serde_json::Value;

mod sha256;

fn contracts_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts")
}

fn read(relative: &str) -> String {
    let path = contracts_directory().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()))
}

fn records(relative: &str) -> Vec<Value> {
    read(relative)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("a fixture record parses"))
        .collect()
}

// ---------------------------------------------------------------------------
// The pin
// ---------------------------------------------------------------------------

#[test]
fn every_vendored_file_hashes_to_its_recorded_digest() {
    let checksums = read("CHECKSUMS.txt");
    let mut checked = 0;

    for line in checksums.lines().filter(|line| !line.trim().is_empty()) {
        let (digest, relative) = line.split_once("  ").expect("a shasum line has two fields");
        let bytes = std::fs::read(contracts_directory().join(relative))
            .unwrap_or_else(|error| panic!("{relative} could not be read: {error}"));

        assert_eq!(
            sha256::hex(&bytes),
            digest,
            "{relative} does not hash to its pinned digest; re-vendor rather than re-hashing"
        );
        checked += 1;
    }

    assert!(checked > 0, "CHECKSUMS.txt lists nothing");
}

#[test]
fn the_manifest_covers_the_tree_exactly() {
    let mut present: Vec<String> = Vec::new();
    walk(&contracts_directory(), &contracts_directory(), &mut present);
    present.retain(|path| path != "CHECKSUMS.txt" && path != "SOURCE.json");
    present.sort();

    let mut pinned: Vec<String> = read("CHECKSUMS.txt")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split_once("  ").expect("a shasum line").1.to_string())
        .collect();
    pinned.sort();

    assert_eq!(
        present, pinned,
        "the tree and CHECKSUMS.txt list different files"
    );
}

#[test]
fn the_two_pin_records_agree_file_for_file() {
    let provenance: Value = serde_json::from_str(&read("SOURCE.json")).expect("SOURCE.json parses");

    let mut recorded = BTreeMap::new();
    for contract in provenance["contracts"]
        .as_array()
        .expect("contracts is a list")
    {
        assert_ne!(
            contract["draft"],
            Value::Bool(true),
            "a draft contract has no upstream to compare against and is not shippable"
        );
        for file in contract["files"].as_array().expect("files is a list") {
            recorded.insert(
                file["path"].as_str().expect("a path").to_string(),
                file["sha256"].as_str().expect("a digest").to_string(),
            );
        }
    }

    let pinned: BTreeMap<String, String> = read("CHECKSUMS.txt")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (digest, path) = line.split_once("  ").expect("a shasum line");
            (path.to_string(), digest.to_string())
        })
        .collect();

    assert_eq!(recorded, pinned, "SOURCE.json and CHECKSUMS.txt disagree");
}

fn walk(root: &Path, directory: &Path, into: &mut Vec<String>) {
    let entries = std::fs::read_dir(directory).expect("the contract tree can be walked");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, into);
        } else {
            let relative = path.strip_prefix(root).expect("inside the tree");
            into.push(relative.to_string_lossy().into_owned());
        }
    }
}

// ---------------------------------------------------------------------------
// The version window
// ---------------------------------------------------------------------------

#[test]
fn the_window_comes_from_the_schema_and_there_is_no_second_key() {
    let schema: Value = serde_json::from_str(contract::SCHEMA_JSON).expect("the schema parses");
    let object = schema.as_object().expect("the schema is an object");

    let mut version_keys: Vec<&str> = object
        .keys()
        .filter(|key| key.contains("version"))
        .map(String::as_str)
        .collect();
    version_keys.sort_unstable();

    assert_eq!(
        version_keys,
        vec![
            "x-method-minimum-versions",
            "x-protocol-version",
            "x-supported-version-range"
        ],
        "a second speakable-versions key drifts from the first and the drift is invisible \
         until a negotiation fails in the field"
    );

    let range = contract::supported_range();
    assert_eq!(
        range.min,
        schema["x-supported-version-range"]["min"].as_u64().unwrap() as u32
    );
    assert_eq!(
        range.max,
        schema["x-supported-version-range"]["max"].as_u64().unwrap() as u32
    );
}

#[test]
fn the_limits_come_from_the_schema() {
    let schema: Value = serde_json::from_str(contract::SCHEMA_JSON).expect("the schema parses");
    let limits = contract::limits();

    assert_eq!(
        limits.max_frame_bytes,
        schema["x-limits"]["max_frame_bytes"].as_u64().unwrap() as usize
    );
    assert_eq!(
        limits.max_params_bytes,
        schema["x-limits"]["max_params_bytes"].as_u64().unwrap() as usize
    );
}

#[test]
fn every_method_the_schema_publishes_has_a_minimum_this_client_can_read() {
    let schema: Value = serde_json::from_str(contract::SCHEMA_JSON).expect("the schema parses");
    let published = schema["x-method-minimum-versions"]
        .as_object()
        .expect("an object");

    for (method, minimum) in published {
        assert_eq!(
            contract::method_minimum(method),
            Some(minimum.as_u64().unwrap() as u32),
            "{method} reads back a different minimum"
        );
    }
}

// ---------------------------------------------------------------------------
// Every golden decodes
// ---------------------------------------------------------------------------

#[test]
fn every_success_golden_decodes_into_its_typed_result() {
    let goldens = records("management/fixtures/success.jsonl");
    assert!(
        goldens.len() > 40,
        "the success goldens are unexpectedly few"
    );

    for golden in &goldens {
        let method = golden["method"].as_str().expect("a method");
        let name = golden["name"].as_str().expect("a name");
        let result = &golden["response"]["result"];

        decode_result(method, result)
            .unwrap_or_else(|reason| panic!("{name} ({method}) did not decode: {reason}"));
    }
}

#[test]
fn every_error_golden_decodes_with_its_code_and_message() {
    for golden in records("management/fixtures/errors.jsonl") {
        let name = golden["name"].as_str().expect("a name");
        let error = &golden["response"]["error"];

        assert!(error["code"].is_string(), "{name} carries no code");
        assert!(error["message"].is_string(), "{name} carries no message");
        assert!(
            error["details"].is_object(),
            "{name} carries no details object"
        );
    }
}

#[test]
fn the_plugin_row_with_every_optional_field_absent_decodes() {
    let golden = compatibility_golden("plugin_row_without_optional_fields");
    let result: PluginsListResult =
        serde_json::from_value(golden["response"]["result"].clone()).expect("decodes");

    let row = result.plugins.first().expect("a row");
    assert!(
        row.verbs.is_empty(),
        "no leading verb means no buttons at all"
    );
    assert!(row.primary_verb.is_none());
    assert!(row.primary_action.is_none());
    assert!(row.settings.is_empty());
    assert!(row.access_profiles.is_empty());
    assert!(row.workspaces.is_empty());
    assert!(row.account_label.is_none());
    assert!(row.remote_disclosure.is_none());
    assert!(
        !row.consent_sentence.is_empty(),
        "the consent sentence is never absent"
    );
}

#[test]
fn the_sign_in_client_with_every_optional_field_absent_decodes() {
    let golden = compatibility_golden("oauth_client_without_optional_fields");
    let result: PluginsListResult =
        serde_json::from_value(golden["response"]["result"].clone()).expect("decodes");

    let client = result.oauth_clients.first().expect("a client row");
    assert!(client.client_id.is_none());
    assert!(!client.secret_present);
    assert!(client.region.is_none());
    assert!(
        client.regions.is_empty(),
        "an absent list reads as one region, never as a picker that failed to arrive"
    );
}

fn compatibility_golden(name: &str) -> Value {
    records("management/fixtures/compatibility.jsonl")
        .into_iter()
        .find(|record| record["name"] == name)
        .unwrap_or_else(|| panic!("no compatibility golden named {name}"))
}

// ---------------------------------------------------------------------------
// The declared absent renderings
// ---------------------------------------------------------------------------

#[test]
fn an_n_minus_one_overview_without_restart_reasons_decodes_and_says_so() {
    let mut golden = success_golden("overview_get");
    golden["response"]["result"]["health"]
        .as_object_mut()
        .expect("health is an object")
        .remove("restart_reasons");

    let overview: OverviewResult =
        serde_json::from_value(golden["response"]["result"].clone()).expect("decodes");

    assert!(overview.health.restart_required, "the fact survives");
    assert!(
        overview.health.restart_reasons.is_empty(),
        "the absent rendering is restart to apply with no reason list"
    );
}

#[test]
fn a_doctor_check_without_a_remediation_decodes_and_offers_no_action() {
    let mut golden = success_golden("doctor_get_in_progress");
    for check in golden["response"]["result"]["checks"]
        .as_array_mut()
        .expect("checks")
    {
        check
            .as_object_mut()
            .expect("a check")
            .remove("remediation");
    }

    let session: DoctorSession =
        serde_json::from_value(golden["response"]["result"].clone()).expect("decodes");

    assert!(!session.checks.is_empty());
    for check in &session.checks {
        assert!(
            check.remediation.is_none(),
            "the absent rendering is the summary with no action button"
        );
        assert!(
            !check.summary.is_empty(),
            "the daemon's own sentence still renders"
        );
    }
}

#[test]
fn a_realtime_object_without_an_engine_decodes() {
    let mut golden = success_golden("overview_get");
    golden["response"]["result"]["realtime"]
        .as_object_mut()
        .expect("realtime is an object")
        .remove("engine");

    let overview: OverviewResult =
        serde_json::from_value(golden["response"]["result"].clone()).expect("decodes");

    assert!(overview.realtime.engine.is_none());
}

fn success_golden(name: &str) -> Value {
    records("management/fixtures/success.jsonl")
        .into_iter()
        .find(|record| record["name"] == name)
        .unwrap_or_else(|| panic!("no success golden named {name}"))
}

// ---------------------------------------------------------------------------
// One decode per published method
// ---------------------------------------------------------------------------

fn decode_result(method: &str, result: &Value) -> Result<(), String> {
    fn check<T: serde::de::DeserializeOwned>(result: &Value) -> Result<(), String> {
        serde_json::from_value::<T>(result.clone())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    match method {
        "hello" => check::<HelloResult>(result),
        "overview.get" => check::<OverviewResult>(result),
        "setup.session.create" => check::<SetupSessionResult>(result),
        "doctor.start" | "doctor.get" | "doctor.cancel" => check::<DoctorSession>(result),
        "logs.query" => check::<LogsQueryResult>(result),
        "lifecycle.prepare" => check::<LifecyclePrepareResult>(result),
        "lifecycle.commit" | "lifecycle.cancel" => check::<LifecycleLeaseResult>(result),
        "diagnostics.build" => check::<DiagnosticsBuildResult>(result),
        "setup.state.get" => check::<SetupStateResult>(result),
        "setup.detect" => check::<SetupDetectResult>(result),
        "settings.sections" => check::<SettingsSectionsResult>(result),
        "settings.get" => check::<SettingsGetResult>(result),
        "settings.apply" => check::<SettingsApplyResult>(result),
        "settings.reload" => check::<SettingsReloadResult>(result),
        "secret.set" | "secret.clear" => check::<SecretResult>(result),
        "secret.migrate_to_keyring" => check::<SecretMigrateResult>(result),
        "providers.set_primary" => check::<ProvidersSetPrimaryResult>(result),
        "providers.models.list" => check::<ProvidersModelsListResult>(result),
        "providers.probe.start"
        | "job.get"
        | "job.cancel"
        | "auth.import.start"
        | "plugins.install.start"
        | "plugins.check.start"
        | "plugins.workspaces.discover.start"
        | "plugins.workspace.select.start"
        | "capabilities.install.start"
        | "meetings.signin.start"
        | "computer_use.grant.start" => check::<JobView>(result),
        "job.list" => check::<JobListResult>(result),
        "auth.start" => check::<AuthStartResult>(result),
        "auth.logout" => check::<AuthLogoutResult>(result),
        "plugins.list" => check::<PluginsListResult>(result),
        "plugins.enable" | "plugins.disable" | "plugins.disconnect" | "plugins.setting.set" => {
            check::<PluginRowResult>(result)
        }
        "plugins.oauth_client.set" => check::<PluginOAuthClientResult>(result),
        "computer_use.permissions.get" => check::<ComputerUsePermissions>(result),
        other => Err(format!("no typed result is declared for {other}")),
    }
}

#[test]
fn every_published_method_has_a_typed_result_in_this_client() {
    // A method the contract publishes and this client cannot decode is a pane
    // that would render empty, so it fails here instead.
    let undecodable: Vec<&str> = contract::methods()
        .map(|(method, _)| method)
        .filter(|method| {
            decode_result(method, &Value::Null)
                .err()
                .is_some_and(is_undeclared)
        })
        .collect();

    assert!(
        undecodable.is_empty(),
        "no typed result for: {undecodable:?}"
    );
}

fn is_undeclared(reason: String) -> bool {
    reason.starts_with("no typed result is declared")
}

// ---------------------------------------------------------------------------
// The typed command line
// ---------------------------------------------------------------------------
//
// The second wire. `contracts/cli/` is the engine's own export of the `--json`
// verbs, and everything below holds this crate to it: every golden decodes into
// the type the runner asks for, nothing published is dropped on the way in,
// every code reads back as itself, and the fake `fermix` the tests and the
// captures drive answers golden bytes and nothing else.

/// One golden envelope, parsed.
fn cli_golden(relative: &str) -> Value {
    let path = contracts_directory().join("cli/fixtures").join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));
    serde_json::from_slice(&bytes).expect("a golden parses")
}

/// Every golden under `cli/fixtures/`, by its path inside that directory.
fn cli_goldens() -> Vec<String> {
    let root = contracts_directory().join("cli/fixtures");
    let mut present: Vec<String> = Vec::new();
    walk(&root, &root, &mut present);
    present.retain(|path| path.ends_with(".json"));
    present.sort();
    present
}

#[test]
fn every_cli_golden_decodes_into_the_type_its_verb_answers_with() {
    let mut decoded = 0;

    for relative in cli_goldens() {
        let golden = cli_golden(&relative);
        let result = golden["result"].clone();

        let outcome: Result<(), String> = match relative.split('/').next().expect("a directory") {
            "service_status" => decode_cli::<ServiceStatus>(&result),
            "service_install" => decode_cli::<InstallOutcome>(&result),
            "service_uninstall" => decode_cli::<ActionResult>(&result),
            "restart" => decode_cli::<CliRestart>(&result),
            "diagnostics_export" => decode_cli::<DiagnosticsExport>(&result),
            "errors" => {
                let error = &golden["error"];
                let code = error["code"].as_str().expect("a code");
                assert_ne!(
                    ServiceCode::of(code),
                    ServiceCode::Unrecognized,
                    "{relative} publishes a code this client reads as unrecognized, so its \
                     refusal would render as a sentence with no state behind it"
                );
                assert!(
                    !error["sentence"].as_str().expect("a sentence").is_empty(),
                    "{relative} carries no sentence"
                );
                Ok(())
            }
            other => Err(format!("no typed result is declared for {other}")),
        };

        outcome.unwrap_or_else(|reason| panic!("{relative} did not decode: {reason}"));
        decoded += 1;
    }

    assert_eq!(
        decoded, 35,
        "the export publishes thirty-five goldens; it now publishes {decoded}"
    );
}

#[test]
fn no_field_a_cli_golden_publishes_is_dropped_on_the_way_in() {
    // Decoding permissively is right on a wire that may add fields, and it is
    // also how a field goes missing without anybody noticing. So each golden is
    // decoded and re-encoded, and every key and every scalar the engine printed
    // has to still be there. A field the engine adds fails here, in the one
    // place that can say which one it was.
    for relative in cli_goldens() {
        if relative.starts_with("errors/") {
            continue;
        }

        let golden = cli_golden(&relative);
        let published = &golden["result"];
        let directory = relative.split('/').next().expect("a directory");

        let decoded = match directory {
            "service_status" => reencode::<ServiceStatus>(published),
            "service_install" => reencode::<InstallOutcome>(published),
            "service_uninstall" => reencode::<ActionResult>(published),
            "restart" => reencode::<CliRestart>(published),
            "diagnostics_export" => reencode::<DiagnosticsExport>(published),
            other => panic!("no typed result is declared for {other}"),
        };

        let mut missing: Vec<String> = Vec::new();
        compare(published, &decoded, "", &mut missing);
        assert!(
            missing.is_empty(),
            "{relative} publishes fields this client does not keep: {missing:?}"
        );
    }
}

#[test]
fn the_fake_command_line_answers_golden_bytes_and_every_golden_is_answered() {
    // Two directions, because each one alone leaves a hole: a state answering a
    // shape the engine never prints is a test passing against fiction, and a
    // golden no state answers is a published result nothing in this repository
    // can be driven into.
    let goldens = contracts_directory().join("cli/fixtures");
    let mut bytes: BTreeMap<Vec<u8>, String> = BTreeMap::new();
    for relative in cli_goldens() {
        let body = std::fs::read(goldens.join(&relative)).expect("a golden is readable");
        bytes.insert(body, relative);
    }

    let states = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/states");
    let mut answered: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut invented: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&states).expect("the states are readable") {
        let state = entry.expect("a state directory").path();
        for file in std::fs::read_dir(&state).expect("a state is readable") {
            let file = file.expect("a state file").path();
            if file.extension().and_then(|name| name.to_str()) != Some("json") {
                continue;
            }

            let name = format!(
                "{}/{}",
                state.file_name().expect("named").to_string_lossy(),
                file.file_name().expect("named").to_string_lossy()
            );
            let body = std::fs::read(&file).expect("a state file is readable");
            match bytes.get(&body) {
                Some(golden) => answered.entry(golden.clone()).or_default().push(name),
                None => invented.push(name),
            }
        }
    }

    assert!(
        invented.is_empty(),
        "the fake command line answers shapes the engine never prints: {invented:?}. \
         Copy the golden the state stands for rather than writing one"
    );

    let unanswered: Vec<&String> = bytes
        .values()
        .filter(|golden| !answered.contains_key(*golden))
        .collect();
    assert!(
        unanswered.is_empty(),
        "these published results have no state of the fake command line behind them: \
         {unanswered:?}"
    );
}

fn decode_cli<T: serde::de::DeserializeOwned>(result: &Value) -> Result<(), String> {
    serde_json::from_value::<T>(result.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn reencode<T>(result: &Value) -> Value
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let decoded: T = serde_json::from_value(result.clone()).expect("the golden decodes");
    serde_json::to_value(&decoded).expect("the decoded result re-encodes")
}

/// Every key and every scalar of `published`, looked for in `kept`.
fn compare(published: &Value, kept: &Value, at: &str, missing: &mut Vec<String>) {
    match published {
        Value::Object(fields) => {
            for (name, value) in fields {
                let path = if at.is_empty() {
                    name.clone()
                } else {
                    format!("{at}.{name}")
                };
                match kept.get(name) {
                    Some(held) => compare(value, held, &path, missing),
                    None => missing.push(path),
                }
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                let path = format!("{at}[{index}]");
                match kept.get(index) {
                    Some(held) => compare(value, held, &path, missing),
                    None => missing.push(path),
                }
            }
        }
        scalar => {
            if scalar != kept {
                missing.push(format!("{at} (published {scalar}, kept {kept})"));
            }
        }
    }
}
