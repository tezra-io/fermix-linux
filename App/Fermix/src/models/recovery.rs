//! Recovery, as data.
//!
//! Where an installed engine cannot launch, bind or read its settings. It never
//! calls the socket: the evidence and the export both come from
//! `diagnostics export --offline --json`, which is the one collector that works
//! while the daemon does not. If the command line cannot run either, the page
//! says so and offers the journal command instead of a silent second collector.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gio;

use crate::copy::Key;
use crate::management::vocabulary::ConfigCondition;
use crate::service::types::DiagnosticsExport;

use super::settings_model::{Sentence, SettingsModel};
use super::Observers;

/// Why this surface is being shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// The settings file cannot be read. The sentence is the parser's own.
    ConfigUnreadable(String),
    /// The command line refused. The sentence is its own.
    ServiceRefused(String),
}

impl Cause {
    /// The title over the sentence.
    pub fn title(&self) -> Key {
        match self {
            Cause::ConfigUnreadable(_) => Key::RecoveryConfigUnreadableTitle,
            Cause::ServiceRefused(_) => Key::RecoveryServiceRefusedTitle,
        }
    }

    /// The sentence, which is always somebody else's words.
    pub fn sentence(&self) -> &str {
        match self {
            Cause::ConfigUnreadable(sentence) | Cause::ServiceRefused(sentence) => sentence,
        }
    }
}

/// Recovery's own model.
pub struct RecoveryModel {
    settings: Rc<SettingsModel>,
    evidence: RefCell<Vec<String>>,
    export: RefCell<Option<DiagnosticsExport>>,
    refusal: RefCell<Option<Sentence>>,
    observers: Observers,
}

impl RecoveryModel {
    /// A Recovery model over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        Rc::new(Self {
            settings,
            evidence: RefCell::new(Vec::new()),
            export: RefCell::new(None),
            refusal: RefCell::new(None),
            observers: Observers::default(),
        })
    }

    /// Tell me when the evidence lands.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// Why this surface is showing, where it should be showing at all.
    pub fn cause(&self) -> Option<Cause> {
        let state = self.settings.state();

        if state.config == ConfigCondition::Unreadable {
            let sentence = state
                .unreadable
                .as_ref()
                .map(|reason| reason.text.clone())
                .unwrap_or_default();
            return Some(Cause::ConfigUnreadable(sentence));
        }

        state
            .service_refusal
            .as_ref()
            .map(|refusal| Cause::ServiceRefused(refusal.text.clone()))
    }

    /// The offline evidence, as lines.
    pub fn evidence(&self) -> Vec<String> {
        self.evidence.borrow().clone()
    }

    /// Whether an export can be written at all.
    pub fn can_export(&self) -> bool {
        self.export.borrow().is_some()
    }

    /// The command line's own refusal, where it refused.
    pub fn refusal(&self) -> Option<Sentence> {
        self.refusal.borrow().clone()
    }

    /// The bundle, as bytes to write to the file a person chose.
    ///
    /// It is the command line's own object, re-encoded rather than rebuilt:
    /// every source it collected reaches the file, including the ones that said
    /// why they could not be read.
    pub fn export_bytes(&self) -> Option<Vec<u8>> {
        let export = self.export.borrow();
        serde_json::to_vec_pretty(export.as_ref()?).ok()
    }

    /// Collect the offline evidence.
    pub async fn collect(&self) {
        let cancellable = gio::Cancellable::new();
        match self
            .settings
            .service()
            .export_diagnostics(&cancellable)
            .await
        {
            Ok(export) => {
                self.evidence.replace(log_lines(&export));
                self.export.replace(Some(export));
                self.refusal.replace(None);
            }
            Err(error) => {
                self.evidence.borrow_mut().clear();
                self.export.replace(None);
                self.refusal.replace(Some(Sentence::of_service(&error)));
            }
        }

        self.observers.notify();
    }

    /// Try the same owned lifecycle transaction again: it clears the start
    /// counter, starts once, and answers what happened.
    pub async fn retry(&self) -> Result<(), Sentence> {
        let outcome = self.settings.set_background_service(true).await;
        self.collect().await;
        outcome
    }
}

/// The log tail inside an export, as lines.
///
/// Public because Recovery is not the only surface that reads it: the Setup
/// assistant's Boot failed screen shows the same offline evidence, and two
/// readers of one shape would be two shapes.
pub fn log_lines(export: &DiagnosticsExport) -> Vec<String> {
    let Some(entries) = export
        .sources
        .get(LOGS_SOURCE)
        .and_then(|source| source.data.as_ref())
        .and_then(|data| data.get("entries"))
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let time = entry.get("time")?.as_str()?;
            let message = entry.get("message")?.as_str()?;
            // A journal line carries no level of its own, and the bundle says
            // so with a null rather than by leaving the key out. The line is
            // still evidence, so it is shown without one.
            match entry.get("level").and_then(serde_json::Value::as_str) {
                Some(level) => Some(format!("{time} {level} {message}")),
                None => Some(format!("{time} {message}")),
            }
        })
        .collect()
}

/// The source of the bundle that carries the log tail, named by the contract.
const LOGS_SOURCE: &str = "logs";

#[cfg(test)]
mod tests {
    use super::*;

    /// One published golden, decoded the way the command line's answer is.
    fn export(relative: &str) -> DiagnosticsExport {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contracts/cli/fixtures/diagnostics_export")
            .join(relative);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).expect("parses");
        serde_json::from_value(envelope["result"].clone()).expect("decodes")
    }

    #[test]
    fn the_evidence_is_the_exports_own_log_tail() {
        assert_eq!(
            log_lines(&export("ok.json")),
            vec![
                "2026-01-01T00:00:00Z info started".to_string(),
                // A journal line has no level of its own, and it is still
                // evidence.
                "2026-01-01T00:00:00+0000 host fermix[4711]: started".to_string(),
            ]
        );
    }

    #[test]
    fn a_source_that_could_not_be_read_has_no_evidence_rather_than_a_blank_line() {
        let degraded = export("degraded.json");
        assert!(
            log_lines(&degraded).is_empty(),
            "the logs source is available and empty, which is no lines rather than one blank one"
        );
    }

    #[test]
    fn a_cause_always_carries_somebody_elses_sentence() {
        let cause = Cause::ConfigUnreadable("expected a table key".into());
        assert_eq!(cause.sentence(), "expected a table key");
        assert_eq!(cause.title(), Key::RecoveryConfigUnreadableTitle);

        let cause = Cause::ServiceRefused("This session has no user service manager.".into());
        assert_eq!(cause.title(), Key::RecoveryServiceRefusedTitle);
    }
}
