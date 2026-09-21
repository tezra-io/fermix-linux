//! The in-memory peer the model tests run against.
//!
//! It answers the vendored goldens through the same envelope reader the socket
//! client uses, so a model driven by this peer meets exactly the results,
//! refusals and version window a model driven by a packaged daemon meets. What
//! it adds is what a socket cannot be asked for on demand: a scripted refusal,
//! a replaced result, and a daemon that is simply not there.
//!
//! It lives in the library rather than under `tests/` because both the unit
//! tests inside `src/` and the integration tests outside it drive models, and
//! two peers would be two behaviours.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::fixtures::Goldens;
use crate::management::client::decode_envelope;
use crate::management::contract;
use crate::management::errors::TransportError;
use crate::management::types::HelloResult;
use crate::management::{Issued, ManagementError};

use super::api::{Answer, ManagementApi};

/// One recorded call.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCall {
    pub method: String,
    pub params: serde_json::Value,
}

/// How long a held call waits before the fixture gives up on the test.
///
/// Two seconds at a millisecond a tick: long enough for any test that means to
/// release, short enough that one which does not fails rather than hangs.
const HELD_CALL_TICKS: u32 = 2000;

/// A daemon made of goldens.
pub struct FixturePeer {
    goldens: Goldens,
    epoch: Cell<u64>,
    negotiated: Cell<Option<u32>>,
    hello: RefCell<Option<HelloResult>>,
    calls: RefCell<Vec<RecordedCall>>,
    refusals: RefCell<Vec<(String, String)>>,
    overrides: RefCell<BTreeMap<String, serde_json::Value>>,
    rebinds: RefCell<Vec<std::path::PathBuf>>,
    silent: Cell<bool>,
    held: RefCell<Option<String>>,
}

impl FixturePeer {
    /// A peer answering one scenario's goldens.
    pub fn new(scenario: &str) -> Self {
        Self {
            goldens: Goldens::load(scenario)
                .unwrap_or_else(|reason| panic!("the {scenario} fixtures do not load: {reason}")),
            epoch: Cell::new(0),
            negotiated: Cell::new(None),
            hello: RefCell::new(None),
            calls: RefCell::new(Vec::new()),
            refusals: RefCell::new(Vec::new()),
            overrides: RefCell::new(BTreeMap::new()),
            rebinds: RefCell::new(Vec::new()),
            silent: Cell::new(false),
            held: RefCell::new(None),
        }
    }

    /// Hold every answer until [`FixturePeer::release`], so a caller can put
    /// calls in flight and keep them there.
    ///
    /// The model's gate counts calls that have started and not finished, and
    /// a fixture that answers the instant it is polled can never produce that
    /// state. This is the only way to test what the application does when its
    /// own concurrency ceiling is reached.
    pub fn hold(&self, method: &str) {
        self.held.replace(Some(method.to_string()));
    }

    /// Let held answers through.
    pub fn release(&self) {
        self.held.replace(None);
    }

    /// Whether this call is one of the held ones.
    fn is_held(&self, method: &str) -> bool {
        self.held.borrow().as_deref() == Some(method)
    }

    /// Every call this peer has been asked for, in order.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.calls.borrow().clone()
    }

    /// How many times one method has been called.
    pub fn count(&self, method: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|call| call.method == method)
            .count()
    }

    /// The parameters of the last call to one method.
    pub fn last(&self, method: &str) -> Option<serde_json::Value> {
        self.calls
            .borrow()
            .iter()
            .rev()
            .find(|call| call.method == method)
            .map(|call| call.params.clone())
    }

    /// Forget what has been recorded.
    pub fn forget(&self) {
        self.calls.borrow_mut().clear();
    }

    /// Answer the next call to one method with the published envelope for one
    /// error code. The sentence is the engine's own, never composed here.
    pub fn refuse_next(&self, method: &str, code: &str) {
        self.refusals
            .borrow_mut()
            .push((method.to_string(), code.to_string()));
    }

    /// Answer one method, or one settings section, with this result from now
    /// on. What a daemon does after a write landed.
    pub fn set_result(&self, method: &str, section: Option<&str>, result: serde_json::Value) {
        self.overrides
            .borrow_mut()
            .insert(key(method, section), serde_json::json!({"result": result}));
    }

    /// One method's current result, as this peer would answer it now.
    ///
    /// The override where one has been set, and the vendored golden
    /// otherwise, so a caller can take what the daemon publishes, change the
    /// one field it is testing and hand the whole thing back to
    /// [`FixturePeer::set_result`] without rebuilding a response by hand.
    pub fn result(&self, method: &str, section: Option<&str>) -> serde_json::Value {
        if let Some(replaced) = self.overrides.borrow().get(&key(method, section)) {
            return replaced
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }

        let mut request = serde_json::json!({
            "request_id": "fixture",
            "protocol_version": self.negotiated.get().unwrap_or(contract::supported_range().max),
            "method": method,
            "params": {},
        });
        if let Some(section) = section {
            request["params"] = serde_json::json!({"section": section});
        }
        self.goldens
            .answer(&request)
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    /// Every socket this peer was asked to point at, in order.
    pub fn rebinds(&self) -> Vec<std::path::PathBuf> {
        self.rebinds.borrow().clone()
    }

    /// Whether anything is listening at all.
    pub fn set_running(&self, running: bool) {
        self.silent.set(!running);
    }

    /// Move the connection epoch, as a disconnect does.
    pub fn disconnect(&self) {
        <Self as ManagementApi>::reset(self);
    }

    fn answer(&self, method: &str, params: &serde_json::Value) -> serde_json::Value {
        let queued = {
            let mut refusals = self.refusals.borrow_mut();
            refusals
                .iter()
                .position(|(name, _)| name == method)
                .map(|at| refusals.remove(at).1)
        };

        if let Some(code) = queued {
            return self.goldens.refusal(serde_json::json!("fixture"), &code);
        }

        let section = params.get("section").and_then(serde_json::Value::as_str);
        if let Some(replaced) = self.overrides.borrow().get(&key(method, section)) {
            return replaced.clone();
        }

        self.goldens.answer(&serde_json::json!({
            "request_id": "fixture",
            "protocol_version": self.negotiated.get().unwrap_or(contract::supported_range().max),
            "method": method,
            "params": params,
        }))
    }
}

fn key(method: &str, section: Option<&str>) -> String {
    match section {
        Some(section) if method == "settings.get" => format!("{method}:{section}"),
        _ => method.to_string(),
    }
}

impl ManagementApi for FixturePeer {
    fn epoch(&self) -> u64 {
        self.epoch.get()
    }

    fn reset(&self) {
        self.epoch.set(self.epoch.get().saturating_add(1));
        self.negotiated.set(None);
        self.hello.replace(None);
    }

    fn negotiated_version(&self) -> Option<u32> {
        self.negotiated.get()
    }

    fn last_hello(&self) -> Option<HelloResult> {
        self.hello.borrow().clone()
    }

    /// The peer is not a socket, so there is nowhere to point it. It records
    /// the ask, because where the application points its client is a fact worth
    /// proving.
    fn rebind(&self, socket: &std::path::Path) -> bool {
        self.rebinds.borrow_mut().push(socket.to_path_buf());
        false
    }

    fn hello(&self, _deadline: Duration) -> Answer<'_, HelloResult> {
        Box::pin(async move {
            let epoch = self.epoch.get();
            Issued {
                epoch,
                value: self.negotiate(),
            }
        })
    }

    fn call(
        &self,
        method: &str,
        params: serde_json::Value,
        _deadline: Duration,
    ) -> Answer<'_, serde_json::Value> {
        let method = method.to_string();
        Box::pin(async move {
            let epoch = self.epoch.get();
            self.calls.borrow_mut().push(RecordedCall {
                method: method.clone(),
                params: params.clone(),
            });

            if self.silent.get() {
                return Issued {
                    epoch,
                    value: Err(ManagementError::Transport(TransportError::NotRunning)),
                };
            }

            // Held calls wait here, which is where a real one waits: after the
            // permit is taken and before an answer exists. The wait is capped
            // so a test that forgets to release fails as a slow test rather
            // than hanging the suite forever.
            let mut waited = 0;
            while self.is_held(&method) && waited < HELD_CALL_TICKS {
                gtk4::glib::timeout_future(Duration::from_millis(1)).await;
                waited += 1;
            }
            assert!(
                !self.is_held(&method),
                "a held call was never released: {method} waited {waited} ticks"
            );

            // The negotiation a real client performs before its first call,
            // performed here for the same reason: a method above the window is
            // refused before it is sent.
            if let Err(error) = self.negotiate() {
                return Issued {
                    epoch,
                    value: Err(error),
                };
            }

            let value = match refuse_above_the_window(&method, self.negotiated.get()) {
                Some(error) => Err(error),
                None => decode_envelope(&method, &self.answer(&method, &params)),
            };

            Issued { epoch, value }
        })
    }
}

impl FixturePeer {
    fn negotiate(&self) -> Result<HelloResult, ManagementError> {
        if self.silent.get() {
            return Err(ManagementError::Transport(TransportError::NotRunning));
        }

        if let Some(hello) = self.hello.borrow().clone() {
            return Ok(hello);
        }

        // Through the same door every other method takes, so a scenario that
        // replaces `hello` is answered with what it replaced it with. The
        // Setup assistant's finish gate reads the web origin out of this
        // answer, and a test that stands one up has to be able to name it.
        let envelope = self.answer("hello", &serde_json::json!({}));
        let result = decode_envelope("hello", &envelope)?;
        let hello: HelloResult =
            serde_json::from_value(result).map_err(|error| ManagementError::Decode {
                method: "hello".to_string(),
                reason: error.to_string(),
            })?;

        let common = contract::supported_range()
            .max
            .min(hello.protocol.maximum_version);
        self.negotiated.set(Some(common));
        self.hello.replace(Some(hello.clone()));

        Ok(hello)
    }
}

/// The refusal a client makes before sending, reproduced here so a model meets
/// it against the peer exactly as it meets it against a daemon.
fn refuse_above_the_window(method: &str, negotiated: Option<u32>) -> Option<ManagementError> {
    let minimum = match contract::method_minimum(method) {
        Some(minimum) => minimum,
        None => {
            return Some(ManagementError::MethodNotInContract {
                method: method.to_string(),
            })
        }
    };

    let negotiated = negotiated?;
    if minimum > negotiated {
        return Some(ManagementError::MethodNeedsNewerDaemon {
            method: method.to_string(),
            requires: minimum,
            negotiated,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::management::types::OverviewResult;
    use crate::models::api::{ask, READ_DEADLINE};
    use gtk4::glib::MainContext;

    fn overview(peer: &FixturePeer) -> Result<OverviewResult, ManagementError> {
        MainContext::new().block_on(async {
            ask::<_, OverviewResult>(peer, "overview.get", &serde_json::json!({}), READ_DEADLINE)
                .await
                .value
        })
    }

    #[test]
    fn a_golden_answers_a_typed_result() {
        let peer = FixturePeer::new("default");
        let result = overview(&peer).expect("the golden decodes");

        assert_eq!(result.daemon.status.as_deref(), Some("running"));
        assert_eq!(peer.count("overview.get"), 1);
    }

    #[test]
    fn a_scripted_refusal_carries_the_daemons_own_sentence() {
        let peer = FixturePeer::new("default");
        peer.refuse_next("overview.get", "busy");

        match overview(&peer) {
            Err(ManagementError::Wire(error)) => {
                assert_eq!(error.code, "busy");
                assert!(!error.rendered().is_empty());
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        assert!(overview(&peer).is_ok(), "one refusal, once");
    }

    #[test]
    fn a_peer_that_is_not_running_says_so_rather_than_hanging() {
        let peer = FixturePeer::new("default");
        peer.set_running(false);

        match overview(&peer) {
            Err(ManagementError::Transport(TransportError::NotRunning)) => {}
            other => panic!("expected a stopped daemon, got {other:?}"),
        }
    }

    #[test]
    fn a_replaced_result_is_what_the_next_call_answers() {
        let peer = FixturePeer::new("default");
        let goldens = Goldens::load("default").expect("loads");
        let mut replaced = goldens.answer(&serde_json::json!({
            "request_id": "x",
            "protocol_version": contract::supported_range().max,
            "method": "overview.get",
            "params": {}
        }))["result"]
            .clone();
        replaced["daemon"]["status"] = serde_json::json!("stopping");
        peer.set_result("overview.get", None, replaced);

        assert_eq!(
            overview(&peer).expect("decodes").daemon.status.as_deref(),
            Some("stopping")
        );
    }

    #[test]
    fn the_window_is_negotiated_before_the_first_call() {
        let peer = FixturePeer::new("default");
        assert_eq!(peer.negotiated_version(), None);

        overview(&peer).expect("decodes");

        assert_eq!(peer.negotiated_version(), Some(2));
        assert!(peer.last_hello().is_some());
    }

    #[test]
    fn a_disconnect_moves_the_epoch_and_stales_what_was_in_flight() {
        let peer = FixturePeer::new("default");
        let issued = MainContext::new().block_on(async {
            ask::<_, OverviewResult>(&peer, "overview.get", &serde_json::json!({}), READ_DEADLINE)
                .await
        });

        peer.disconnect();

        assert!(super::super::api::accept(&peer, issued).is_none());
    }
}
