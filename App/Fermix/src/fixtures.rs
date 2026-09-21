//! The vendored goldens, loaded once and answered from.
//!
//! Two callers share this: the fixture daemon, which serves them over a real
//! socket, and the in-memory peer the model tests run against. One loader, so
//! the peer a test drives and the peer a capture drives cannot answer
//! differently.
//!
//! Nothing here composes a response. Every answer is a record the engine
//! published, optionally replaced by a scenario's own record.

use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::management::contract;

/// The baseline: one golden response per method, and one per section for
/// `settings.get`.
const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");

/// One golden envelope per published error code. Nothing composes a refusal: a
/// refusal this peer has to answer with is the one the engine published for
/// that code.
const ERRORS: &str = include_str!("../contracts/management/fixtures/errors.jsonl");

/// The scenarios, compiled in so the same bytes answer wherever this runs from.
pub const SCENARIOS: &[(&str, &str)] = &[
    (
        "default",
        include_str!("../tests/fixtures/scenarios/default/overrides.jsonl"),
    ),
    (
        "setup_required",
        include_str!("../tests/fixtures/scenarios/setup_required/overrides.jsonl"),
    ),
    (
        "not_running",
        include_str!("../tests/fixtures/scenarios/not_running/overrides.jsonl"),
    ),
    (
        "restart_pending",
        include_str!("../tests/fixtures/scenarios/restart_pending/overrides.jsonl"),
    ),
    (
        "external_change",
        include_str!("../tests/fixtures/scenarios/external_change/overrides.jsonl"),
    ),
    (
        "unreadable",
        include_str!("../tests/fixtures/scenarios/unreadable/overrides.jsonl"),
    ),
    // Three scenarios the reference captures need and the five above cannot
    // show: a Doctor run that finished clean, one that finished with a failure
    // to act on, and a log with nothing in it.
    (
        "doctor_healthy",
        include_str!("../tests/fixtures/scenarios/doctor_healthy/overrides.jsonl"),
    ),
    (
        "doctor_failed",
        include_str!("../tests/fixtures/scenarios/doctor_failed/overrides.jsonl"),
    ),
    (
        "logs_empty",
        include_str!("../tests/fixtures/scenarios/logs_empty/overrides.jsonl"),
    ),
    // The states LA3's surfaces need and the goldens alone cannot show: a
    // plugin installed and waiting for a sign-in, the notetaker in each of the
    // three answers that are not "signed in", and a machine the computer-use
    // helper is not installed on.
    (
        "integrations_states",
        include_str!("../tests/fixtures/scenarios/integrations_states/overrides.jsonl"),
    ),
    (
        "meetings_signed_out",
        include_str!("../tests/fixtures/scenarios/meetings_signed_out/overrides.jsonl"),
    ),
    (
        "meetings_absent",
        include_str!("../tests/fixtures/scenarios/meetings_absent/overrides.jsonl"),
    ),
    (
        "meetings_refused",
        include_str!("../tests/fixtures/scenarios/meetings_refused/overrides.jsonl"),
    ),
    (
        "computer_states",
        include_str!("../tests/fixtures/scenarios/computer_states/overrides.jsonl"),
    ),
    (
        "computer_wayland",
        include_str!("../tests/fixtures/scenarios/computer_wayland/overrides.jsonl"),
    ),
    // The Setup assistant's own states. Each is named for the screen it shows,
    // because the assistant's screen is decided by what the daemon reports
    // rather than by anything this application holds.
    (
        "onboarding_welcome",
        include_str!("../tests/fixtures/scenarios/onboarding_welcome/overrides.jsonl"),
    ),
    (
        "onboarding_starting",
        include_str!("../tests/fixtures/scenarios/onboarding_starting/overrides.jsonl"),
    ),
    (
        "onboarding_linger_denied",
        include_str!("../tests/fixtures/scenarios/onboarding_linger_denied/overrides.jsonl"),
    ),
    (
        "onboarding_boot_failed",
        include_str!("../tests/fixtures/scenarios/onboarding_boot_failed/overrides.jsonl"),
    ),
    (
        "onboarding_connect_ai",
        include_str!("../tests/fixtures/scenarios/onboarding_connect_ai/overrides.jsonl"),
    ),
    (
        "onboarding_about_you",
        include_str!("../tests/fixtures/scenarios/onboarding_about_you/overrides.jsonl"),
    ),
    (
        "onboarding_refused_personalization",
        include_str!(
            "../tests/fixtures/scenarios/onboarding_refused_personalization/overrides.jsonl"
        ),
    ),
    (
        "onboarding_no_restart",
        include_str!("../tests/fixtures/scenarios/onboarding_no_restart/overrides.jsonl"),
    ),
    (
        "onboarding_restart_needed",
        include_str!("../tests/fixtures/scenarios/onboarding_restart_needed/overrides.jsonl"),
    ),
    (
        "onboarding_ready",
        include_str!("../tests/fixtures/scenarios/onboarding_ready/overrides.jsonl"),
    ),
    // The same finished home, read against a command line that reports a newer
    // engine installed than the one running: Home's only attention row is then
    // the one comparing builds.
    (
        "onboarding_skew",
        include_str!("../tests/fixtures/scenarios/onboarding_skew/overrides.jsonl"),
    ),
    // Secrets kept in the private file store, which is the one state that
    // offers a way back to the keyring.
    (
        "secret_store_file",
        include_str!("../tests/fixtures/scenarios/secret_store_file/overrides.jsonl"),
    ),
];

/// The scenarios that are a property of the socket rather than of a response:
/// nothing listens at all. One is Home's daemon-unavailable state and the other
/// is the fresh account the Setup assistant opens on.
pub const SILENT_SCENARIOS: &[&str] = &["not_running", "onboarding_welcome"];

/// Whether this scenario answers at all.
pub fn is_silent(scenario: &str) -> bool {
    SILENT_SCENARIOS.contains(&scenario)
}

/// The goldens for one scenario.
pub struct Goldens {
    responses: BTreeMap<String, serde_json::Value>,
    refusals: BTreeMap<String, serde_json::Value>,
    /// The methods a scenario answers with a sequence rather than with one
    /// record, and how far through each one is. A job that finishes on the
    /// third read is a property of the run rather than of the response, and it
    /// is still a record from a file rather than a response composed here.
    sequences: BTreeMap<String, Vec<serde_json::Value>>,
    cursors: RefCell<BTreeMap<String, usize>>,
}

impl Goldens {
    /// Load the baseline and lay one scenario's records over it.
    pub fn load(scenario: &str) -> Result<Self, String> {
        let overrides = SCENARIOS
            .iter()
            .find(|(name, _)| *name == scenario)
            .map(|(_, body)| *body)
            .ok_or_else(|| format!("no scenario named {scenario}"))?;

        let mut responses = parse(SUCCESS)?;
        let (replaced, sequences) = parse_with_sequences(overrides)?;
        responses.extend(replaced);

        Ok(Self {
            responses,
            refusals: refusals()?,
            sequences,
            cursors: RefCell::new(BTreeMap::new()),
        })
    }

    /// The whole response envelope for one request, with the request's own id
    /// stamped on it.
    ///
    /// The version window is enforced here for the same reason the daemon
    /// enforces it: a client that negotiates against this peer has to meet the
    /// same two refusals it meets in the field.
    pub fn answer(&self, request: &serde_json::Value) -> serde_json::Value {
        let request_id = request
            .get("request_id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let method = request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let version = request
            .get("protocol_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;

        let window = contract::supported_range();
        if version < window.min {
            return self.refusal(request_id, "client_too_old");
        }
        if version > window.max {
            return self.refusal(request_id, "daemon_too_old");
        }

        let section = request
            .get("params")
            .and_then(|params| params.get("section"))
            .and_then(serde_json::Value::as_str);

        if let Some(answer) = self.next_in_sequence(method) {
            return stamped(answer, request_id);
        }

        match self.responses.get(&key(method, section)) {
            Some(response) => stamped(response.clone(), request_id),
            None => self.refusal(request_id, "method_not_found"),
        }
    }

    /// The next record of a sequenced method, holding on the last one.
    fn next_in_sequence(&self, method: &str) -> Option<serde_json::Value> {
        let sequence = self.sequences.get(method)?;
        let mut cursors = self.cursors.borrow_mut();
        let at = cursors.entry(method.to_string()).or_insert(0);
        let answer = sequence.get(*at).or_else(|| sequence.last())?.clone();
        *at = (*at + 1).min(sequence.len());
        Some(answer)
    }

    /// The published envelope for one code, stamped with the request it
    /// answers. A code with no golden is a contract this build was compiled
    /// against and cannot serve, which is loud rather than invented.
    pub fn refusal(&self, request_id: serde_json::Value, code: &str) -> serde_json::Value {
        match self.refusals.get(code) {
            Some(response) => stamped(response.clone(), request_id),
            None => panic!("the vendored goldens publish no envelope for {code}"),
        }
    }

    /// Whether this scenario answers one method at all.
    pub fn publishes(&self, method: &str, section: Option<&str>) -> bool {
        self.responses.contains_key(&key(method, section))
    }

    /// Every `settings.get` section these goldens carry, in order.
    pub fn sections(&self) -> Vec<String> {
        self.responses
            .keys()
            .filter_map(|key| key.strip_prefix("settings.get:"))
            .map(str::to_string)
            .collect()
    }
}

/// One golden per lookup key: the method, or the method and the section for
/// `settings.get`.
type Responses = BTreeMap<String, serde_json::Value>;

/// One scenario's records: the ones that replace a golden, and the ones that
/// are answered in order.
type Sequences = BTreeMap<String, Vec<serde_json::Value>>;

fn parse_with_sequences(body: &str) -> Result<(Responses, Sequences), String> {
    let mut responses = Responses::new();
    let mut sequences = Sequences::new();

    for (number, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        let record: serde_json::Value = serde_json::from_str(line)
            .map_err(|error| format!("record {} does not parse: {error}", number + 1))?;
        let method = record
            .get("method")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("record {} carries no method", number + 1))?;

        match record.get("responses") {
            Some(serde_json::Value::Array(answers)) => {
                sequences.insert(method.to_string(), answers.clone());
            }
            Some(_) => {
                return Err(format!(
                    "record {} carries a responses that is not a list",
                    number + 1
                ))
            }
            None => {
                let response = record
                    .get("response")
                    .ok_or_else(|| format!("record {} carries no response", number + 1))?;
                responses.insert(key(method, section_of(response)), response.clone());
            }
        }
    }

    Ok((responses, sequences))
}

fn parse(body: &str) -> Result<Responses, String> {
    let mut responses = Responses::new();

    for (number, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        let record: serde_json::Value = serde_json::from_str(line)
            .map_err(|error| format!("record {} does not parse: {error}", number + 1))?;

        let method = record
            .get("method")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("record {} carries no method", number + 1))?;

        let response = record
            .get("response")
            .ok_or_else(|| format!("record {} carries no response", number + 1))?;

        responses.insert(key(method, section_of(response)), response.clone());
    }

    Ok(responses)
}

/// A `settings.get` golden is keyed by the section it answers for, which is the
/// `id` its own result carries.
fn section_of(response: &serde_json::Value) -> Option<&str> {
    response.get("result")?.get("id")?.as_str()
}

fn key(method: &str, section: Option<&str>) -> String {
    match section {
        Some(section) if method == "settings.get" => format!("{method}:{section}"),
        _ => method.to_string(),
    }
}

/// The published error envelopes, keyed by code. `method_not_found` appears
/// twice in the goldens and the first record wins, which is the bare refusal
/// rather than the one carrying `requires`.
fn refusals() -> Result<Responses, String> {
    let mut refusals = Responses::new();

    for (number, line) in ERRORS.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: serde_json::Value = serde_json::from_str(line)
            .map_err(|error| format!("error record {} does not parse: {error}", number + 1))?;
        let code = record
            .get("code")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("error record {} carries no code", number + 1))?;
        let response = record
            .get("response")
            .ok_or_else(|| format!("error record {} carries no response", number + 1))?;

        refusals
            .entry(code.to_string())
            .or_insert_with(|| response.clone());
    }

    Ok(refusals)
}

fn stamped(mut response: serde_json::Value, request_id: serde_json::Value) -> serde_json::Value {
    if let Some(object) = response.as_object_mut() {
        object.insert("request_id".to_string(), request_id);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "request_id": "fx-1",
            "protocol_version": contract::supported_range().max,
            "method": method,
            "params": params,
        })
    }

    #[test]
    fn every_scenario_loads() {
        for (name, _) in SCENARIOS {
            Goldens::load(name).unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }

    #[test]
    fn a_scenario_that_does_not_exist_is_refused_rather_than_defaulted() {
        assert!(Goldens::load("no_such_scenario").is_err());
    }

    #[test]
    fn an_answer_carries_the_request_id_it_is_answering() {
        let goldens = Goldens::load("default").expect("loads");
        let answer = goldens.answer(&request("overview.get", serde_json::json!({})));

        assert_eq!(answer["request_id"], "fx-1");
        assert!(answer["result"]["daemon"].is_object());
    }

    #[test]
    fn a_scenario_record_replaces_the_baseline_one() {
        let baseline = Goldens::load("default").expect("loads");
        let overridden = Goldens::load("setup_required").expect("loads");
        let ask = request("overview.get", serde_json::json!({}));

        assert_eq!(
            baseline.answer(&ask)["result"]["readiness"]["status"],
            "ready"
        );
        assert_eq!(
            overridden.answer(&ask)["result"]["readiness"]["status"],
            "setup_required"
        );
    }

    #[test]
    fn a_settings_section_is_answered_by_its_own_golden() {
        let goldens = Goldens::load("default").expect("loads");
        let answer = goldens.answer(&request(
            "settings.get",
            serde_json::json!({"section": "memory"}),
        ));

        assert_eq!(answer["result"]["id"], "memory");
    }

    #[test]
    fn a_version_outside_the_window_meets_the_published_refusal() {
        let goldens = Goldens::load("default").expect("loads");
        let mut ask = request("hello", serde_json::json!({}));
        ask["protocol_version"] = serde_json::json!(contract::supported_range().max + 1);

        assert_eq!(goldens.answer(&ask)["error"]["code"], "daemon_too_old");
    }

    #[test]
    fn a_method_with_no_golden_is_refused_rather_than_invented() {
        let goldens = Goldens::load("default").expect("loads");
        let answer = goldens.answer(&request("settings.invent", serde_json::json!({})));

        assert_eq!(answer["error"]["code"], "method_not_found");
    }

    #[test]
    fn the_sections_are_the_ones_the_goldens_carry() {
        let goldens = Goldens::load("default").expect("loads");
        let sections = goldens.sections();

        assert!(sections.contains(&"memory".to_string()));
        assert!(sections.contains(&"sandbox".to_string()));
        assert!(goldens.publishes("settings.get", Some("memory")));
        assert!(!goldens.publishes("settings.get", Some("nothing")));
    }
}
