//! Every sentence the update surfaces show, and what each shows when: the
//! tray's row, Home's attention row, Settings' Updates group and the dialog.
//! The words follow the macOS app's (Localizable.strings 164-221, 588)
//! where it has them. A failed check is never shown as up to date.

use crate::update::check::CheckError;
use crate::update::feed::{utc_date, FeedError};
use crate::update::fetch::FetchError;
use crate::update::packagekit::{Ending, Fallback};
use crate::update::policy::{Offer, Outcome};
use crate::update::state::{Failure, Installation, Manual, Stage, Step, Updates, Waiting};
use crate::update::store::Checked;

pub const CHECK_FOR_UPDATES: &str = "Check for Updates…";
pub const AUTOMATIC: &str = "Check for updates automatically";
pub const AUTOMATIC_DETAIL: &str = "When Fermix starts, then once a day while it runs.";
/// Shown as PackageKit is asked, before the system's own prompt appears.
pub const PASSWORD_NOTICE: &str = "Your system now asks for your password. It may call this an \
    untrusted package, because it comes from Fermix rather than your system's software \
    sources. Fermix has already checked that Fermix signed it.";
const NOT_CHECKED: &str = "Fermix has not checked for updates yet";
const CHECKING: &str = "Checking for updates";
const CHECK_FAILED: &str = "Fermix could not check for updates";
const UP_TO_DATE: &str = "Up to date";
const DEVELOPMENT: &str = "This is a development build, which does not update itself";
const UNKNOWN: &str = "This copy of Fermix cannot check for updates";
const RESTART_TITLE: &str = "Restart to finish updating";
const ASKS_FIRST: &str = "Fermix installs an update only when you ask it to.";
const ASKS_FIRST_CRITICAL: &str =
    "This update fixes something important. Fermix still installs it only when you ask it to.";

/// What a row's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    CheckNow,
    /// Opens the dialog on the update.
    Show,
    /// Opens the dialog on the restart that finishes updating.
    Restart,
}

/// The tray's update row, under Run Doctor.
pub fn tray_label(updates: &Updates) -> String {
    if let Some(offer) = updates.announced() {
        return format!("Update to {}…", offer.release.version);
    }
    if updates.restart_pending {
        return "Restart to Finish Updating…".into();
    }
    CHECK_FOR_UPDATES.into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeRow {
    pub title: String,
    pub body: String,
    pub button: &'static str,
    pub action: RowAction,
}

/// Home's attention row: an update not skipped, or a restart that finishes one.
pub fn home_row(updates: &Updates) -> Option<HomeRow> {
    if let Some(offer) = updates.announced() {
        let body = if offer.critical {
            ASKS_FIRST_CRITICAL
        } else {
            ASKS_FIRST
        };
        return Some(HomeRow {
            title: available(offer),
            body: body.into(),
            button: "Show",
            action: RowAction::Show,
        });
    }
    updates.restart_pending.then(|| HomeRow {
        title: RESTART_TITLE.into(),
        body: "A newer Fermix is installed than the one running.".into(),
        button: "Restart…",
        action: RowAction::Restart,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    /// This version and the engine's, under "This version".
    pub version: String,
    pub status: String,
    pub detail: Option<String>,
    pub button: Option<(&'static str, RowAction)>,
    pub checking: bool,
    pub automatic: bool,
    /// Off for a copy that never checks.
    pub automatic_sensitive: bool,
}

/// Settings' Updates group. `engine` is the running engine's version.
pub fn settings_view(updates: &Updates, engine: Option<&str>, now: u64) -> SettingsView {
    let checks = matches!(updates.installation, Some(Installation::Package { .. }));
    let (status, detail, button) = settings_status(updates, now);
    SettingsView {
        version: version_line(updates.installation.as_ref(), engine),
        status,
        detail,
        button: if checks { button } else { None },
        checking: updates.checking,
        automatic: updates.remembered.automatic,
        automatic_sensitive: checks,
    }
}

type Status = (String, Option<String>, Option<(&'static str, RowAction)>);

fn settings_status(updates: &Updates, now: u64) -> Status {
    let check_now = Some(("Check Now", RowAction::CheckNow));
    match &updates.installation {
        None => return ("Reading…".into(), None, None),
        Some(Installation::Development { .. }) => return (DEVELOPMENT.into(), None, None),
        Some(Installation::Unknown { .. }) => return (UNKNOWN.into(), None, None),
        Some(Installation::Package { .. }) => {}
    }
    if updates.checking {
        return (CHECKING.into(), None, None);
    }
    if let Some(offer) = updates.offer() {
        return (
            available_status(offer),
            last_checked(updates, now),
            Some(("Show…", RowAction::Show)),
        );
    }
    if updates.restart_pending {
        return (
            RESTART_TITLE.into(),
            None,
            Some(("Restart…", RowAction::Restart)),
        );
    }
    if let Some(sentence) = &updates.failure {
        return (CHECK_FAILED.into(), Some(sentence.clone()), check_now);
    }
    match &updates.found {
        Some(outcome) => (
            outcome_status(outcome),
            last_checked(updates, now),
            check_now,
        ),
        None => recalled(updates, now),
    }
}

/// An earlier run's last check, until this run's first ends.
fn recalled(updates: &Updates, now: u64) -> Status {
    let check_now = Some(("Check Now", RowAction::CheckNow));
    let Some(last) = &updates.remembered.last_check else {
        return (NOT_CHECKED.into(), None, check_now);
    };
    let installed = match &updates.installation {
        Some(Installation::Package { version, .. }) => Some(*version),
        _ => None,
    };
    let when = checked_words(last.at, now);
    match &last.result {
        Checked::Failed { sentence } => (
            CHECK_FAILED.into(),
            Some(format!("{sentence} Last tried {when}.")),
            check_now,
        ),
        Checked::Available { version, critical } if installed.is_some_and(|i| *version > i) => {
            let status = if *critical {
                format!("Fermix {version} is available and fixes something important")
            } else {
                format!("Fermix {version} is available")
            };
            (
                status,
                last_checked(updates, now),
                Some(("Show…", RowAction::Show)),
            )
        }
        Checked::Stranded { latest } => (
            stranded_status(&latest.to_string()),
            last_checked(updates, now),
            check_now,
        ),
        Checked::Development => (DEVELOPMENT.into(), None, None),
        Checked::UpToDate | Checked::Available { .. } => {
            (UP_TO_DATE.into(), last_checked(updates, now), check_now)
        }
    }
}

fn outcome_status(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Development => DEVELOPMENT.into(),
        Outcome::UpToDate => UP_TO_DATE.into(),
        Outcome::Available(offer) => available_status(offer),
        Outcome::Stranded { latest } => stranded_status(&latest.to_string()),
    }
}

fn stranded_status(latest: &str) -> String {
    format!("Fermix {latest} is available, but cannot update this version")
}

fn available_status(offer: &Offer) -> String {
    if offer.critical {
        format!("{} and fixes something important", available(offer))
    } else {
        available(offer)
    }
}

fn available(offer: &Offer) -> String {
    format!("Fermix {} is available", offer.release.version)
}

/// When the last check ran, and the date of the newest feed accepted, so a
/// feed frozen in place shows its age.
fn last_checked(updates: &Updates, now: u64) -> Option<String> {
    let last = updates.remembered.last_check.as_ref()?;
    let checked = format!("Last checked {}", checked_words(last.at, now));
    match updates.remembered.accepted_generated_at {
        Some(written) => Some(format!(
            "{checked} · Update information from {}",
            date_words(written, now)
        )),
        None => Some(checked),
    }
}

fn version_line(installation: Option<&Installation>, engine: Option<&str>) -> String {
    let own = match installation {
        None => return "Reading…".into(),
        Some(Installation::Development { version }) => format!("{version}, a development build"),
        Some(installation) => installation.version_text(),
    };
    match engine {
        Some(engine) => format!("{own} · engine {engine}"),
        None => own,
    }
}

/// How long ago, in the largest whole unit.
pub fn checked_words(at: u64, now: u64) -> String {
    let ago = now.saturating_sub(at);
    let (count, unit) = match ago {
        0..60 => return "just now".into(),
        60..3_600 => (ago / 60, "minute"),
        3_600..86_400 => (ago / 3_600, "hour"),
        _ => (ago / 86_400, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// A UTC date as "22 October", with its year when that is not `now`'s.
pub fn date_words(at: u64, now: u64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let (year, month, day) = utc_date(at);
    let name = MONTHS[usize::try_from(month - 1).expect("a month is 1 to 12")];
    if year == utc_date(now).0 {
        format!("{day} {name}")
    } else {
        format!("{day} {name} {year}")
    }
}

/// A size as GNOME writes one: decimal units, one place.
pub fn size_words(bytes: u64) -> String {
    let units = [(1e9, "GB"), (1e6, "MB"), (1e3, "kB")];
    // Precision is lost only past 2^53 bytes, far beyond any package.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    match units.iter().find(|(size, _)| value >= *size) {
        Some((size, unit)) => format!("{:.1} {unit}", value / size),
        None => format!("{bytes} bytes"),
    }
}

/// A part of a note: plain text, or inline `code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Code(String),
}

/// A note's inline code, the only markup notes carry. An unpaired backtick
/// leaves the whole note as text.
pub fn note_spans(note: &str) -> Vec<Span> {
    let parts: Vec<&str> = note.split('`').collect();
    if parts.len().is_multiple_of(2) {
        return vec![Span::Text(note.to_owned())];
    }
    parts
        .iter()
        .enumerate()
        .filter(|(_, part)| !part.is_empty())
        .map(|(index, part)| match index % 2 {
            0 => Span::Text((*part).to_owned()),
            _ => Span::Code((*part).to_owned()),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogAction {
    Install,
    Later,
    Skip,
    /// Stops a download; nothing has changed yet.
    Cancel,
    /// Runs the failed step again.
    TryAgain,
    CheckAgain,
    Close,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogButton {
    pub label: &'static str,
    pub action: DialogAction,
    pub suggested: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// `None` while PackageKit cannot say.
    pub fraction: Option<f64>,
    pub text: String,
}

/// The dialog, drawn under the app's icon.
#[derive(Debug, Clone, PartialEq)]
pub struct DialogView {
    pub title: String,
    pub subtitle: Option<String>,
    /// A sentence above the notes or the progress.
    pub notice: Option<String>,
    pub notes: Vec<Vec<Span>>,
    pub progress: Option<Progress>,
    pub spinner: bool,
    pub failure: Option<String>,
    pub manual: Option<Manual>,
    pub buttons: Vec<DialogButton>,
    /// False once the install is committed: it runs to the restart.
    pub closable: bool,
}

impl DialogView {
    fn titled(title: impl Into<String>) -> DialogView {
        DialogView {
            title: title.into(),
            subtitle: None,
            notice: None,
            notes: Vec::new(),
            progress: None,
            spinner: false,
            failure: None,
            manual: None,
            buttons: vec![button("Close", DialogAction::Close, false)],
            closable: true,
        }
    }
}

fn button(label: &'static str, action: DialogAction, suggested: bool) -> DialogButton {
    DialogButton {
        label,
        action,
        suggested,
    }
}

/// The dialog for where updates stand. `engine` is the running engine's version.
pub fn dialog_view(updates: &Updates, engine: Option<&str>) -> DialogView {
    if let Some(step) = &updates.step {
        return step_view(step, updates.offer());
    }
    match &updates.installation {
        Some(Installation::Development { version }) => return development_view(version),
        Some(Installation::Unknown { version }) => return unknown_view(version),
        _ => {}
    }
    if updates.checking || updates.installation.is_none() {
        let mut view = DialogView::titled("Checking for updates…");
        view.spinner = true;
        return view;
    }
    if let Some(offer) = updates.offer() {
        return offer_view(updates, offer, engine);
    }
    if updates.restart_pending {
        return restart_view();
    }
    if let Some(sentence) = &updates.failure {
        let mut view = DialogView::titled(CHECK_FAILED);
        view.failure = Some(sentence.clone());
        view.buttons
            .push(button("Try Again", DialogAction::CheckAgain, true));
        return view;
    }
    found_view(updates, engine)
}

fn development_view(version: &str) -> DialogView {
    let mut view = DialogView::titled("This is a development build");
    view.subtitle = Some(format!("You have {version}"));
    view.notice = Some("A development build of Fermix does not update itself.".into());
    view
}

fn unknown_view(version: &str) -> DialogView {
    let mut view = DialogView::titled(UNKNOWN);
    view.notice = Some(format!(
        "This copy of Fermix is version {version}, which no release of Fermix has."
    ));
    view
}

fn found_view(updates: &Updates, engine: Option<&str>) -> DialogView {
    let installed = installed_text(updates);
    match &updates.found {
        Some(Outcome::Stranded { latest }) => {
            let mut view = DialogView::titled(format!("Fermix {latest} is available"));
            view.notice = Some(format!(
                "It cannot be installed over Fermix {installed} directly."
            ));
            view
        }
        Some(Outcome::UpToDate) => {
            let mut view = DialogView::titled("Fermix is up to date");
            view.subtitle = Some(you_have(&installed, engine, None));
            view
        }
        Some(Outcome::Development) => development_view(&installed),
        Some(Outcome::Available(_)) | None => {
            let mut view = DialogView::titled("Checking for updates…");
            view.spinner = true;
            view
        }
    }
}

fn installed_text(updates: &Updates) -> String {
    updates
        .installation
        .as_ref()
        .map_or_else(String::new, Installation::version_text)
}

fn offer_view(updates: &Updates, offer: &Offer, engine: Option<&str>) -> DialogView {
    let mut view = DialogView::titled(available(offer));
    let installed = installed_text(updates);
    view.subtitle = Some(you_have(
        &installed,
        engine,
        Some(&offer.release.engine.version),
    ));
    let mut notices = Vec::new();
    if offer.critical {
        notices.push("This update fixes something important.".to_owned());
    }
    if let Some(held) = offer.held_back {
        notices.push(format!(
            "Fermix {held} is out too. Install this update first."
        ));
    }
    if updates.format().is_none() {
        notices.push(
            "This copy of Fermix was not installed from its package, so it cannot install \
             updates itself."
                .into(),
        );
    }
    view.notice = (!notices.is_empty()).then(|| notices.join(" "));
    view.notes = offer.release.notes.iter().map(|n| note_spans(n)).collect();
    view.buttons = Vec::new();
    if !offer.critical {
        view.buttons
            .push(button("Skip This Version", DialogAction::Skip, false));
    }
    view.buttons
        .push(button("Later", DialogAction::Later, false));
    if updates.format().is_some() {
        view.buttons
            .push(button("Install Update", DialogAction::Install, true));
    }
    view
}

/// "You have 0.3.0 · engine 0.16.0 → 0.17.0", the engine's part as known.
fn you_have(installed: &str, engine: Option<&str>, next: Option<&str>) -> String {
    match (engine, next) {
        (Some(now), Some(next)) if now != next => {
            format!("You have {installed} · engine {now} → {next}")
        }
        (Some(now), _) => format!("You have {installed} · engine {now}"),
        (None, Some(next)) => format!("You have {installed} · engine {next} with the update"),
        (None, None) => format!("You have {installed}"),
    }
}

fn restart_view() -> DialogView {
    let mut view = DialogView::titled(RESTART_TITLE);
    view.subtitle = Some("A newer Fermix is installed than the one running.".into());
    view.notice = Some(
        "Fermix and its engine restart. Anything they are doing right now is interrupted.".into(),
    );
    view.buttons = vec![
        button("Later", DialogAction::Later, false),
        button("Restart", DialogAction::Restart, true),
    ];
    view
}

fn step_view(step: &Step, offer: Option<&Offer>) -> DialogView {
    let version = offer.map_or_else(String::new, |o| format!(" {}", o.release.version));
    match step {
        Step::Downloading { got, size } => {
            let mut view = DialogView::titled(format!("Downloading Fermix{version}"));
            view.progress = Some(Progress {
                fraction: Some(fraction(*got, *size)),
                text: format!("{} of {}", size_words(*got), size_words(*size)),
            });
            view.buttons = vec![button("Cancel", DialogAction::Cancel, false)];
            view
        }
        Step::Verifying => committed("Checking the download", None),
        Step::Asking => committed(
            &format!("Installing Fermix{version}"),
            Some(PASSWORD_NOTICE),
        ),
        Step::Installing { percent, waiting } => {
            let notice = (*waiting == Some(Waiting::Password)).then_some(PASSWORD_NOTICE);
            let mut view = committed(&format!("Installing Fermix{version}"), notice);
            view.spinner = false;
            view.progress = Some(Progress {
                fraction: percent.map(|p| f64::from(p.min(100)) / 100.0),
                text: installing_words(*waiting).into(),
            });
            view
        }
        Step::Restarting => committed(
            "Restarting Fermix",
            Some("Fermix closes and opens again on its own."),
        ),
        Step::Failed(failure) => failed_view(failure),
    }
}

/// A step past the point of no return: no buttons, and no closing.
fn committed(title: &str, notice: Option<&str>) -> DialogView {
    let mut view = DialogView::titled(title);
    view.notice = notice.map(str::to_owned);
    view.spinner = true;
    view.buttons = Vec::new();
    view.closable = false;
    view
}

fn fraction(got: u64, size: u64) -> f64 {
    if size == 0 {
        return 0.0;
    }
    // Both are package sizes, far below where f64 loses whole bytes.
    #[allow(clippy::cast_precision_loss)]
    let fraction = got.min(size) as f64 / size as f64;
    fraction
}

fn installing_words(waiting: Option<Waiting>) -> &'static str {
    match waiting {
        Some(Waiting::Lock) => "Waiting for another installation to finish…",
        Some(Waiting::Password) => "Waiting for your password…",
        None => "Installing…",
    }
}

fn failed_view(failure: &Failure) -> DialogView {
    let title = match failure.stage {
        Stage::Download => "The update did not download",
        Stage::Install => "The update did not install",
        Stage::Restart => "Fermix did not finish updating",
    };
    let mut view = DialogView::titled(title);
    view.failure = Some(failure.sentence.clone());
    view.manual = Some(failure.manual.clone());
    view.buttons
        .push(button("Try Again", DialogAction::TryAgain, true));
    view
}

/// What the person does instead, above the command, the link or nothing.
pub fn manual_intro(manual: &Manual) -> &'static str {
    match manual {
        Manual::Command { .. } => "To install it yourself, run this in a terminal:",
        Manual::Link { .. } => "You can also download it yourself:",
        Manual::Reopen => "Quit Fermix and open it again to finish.",
    }
}

/// Why a check failed, as Settings and the dialog say it.
pub fn check_failure(error: &CheckError) -> String {
    match error {
        CheckError::Fetch(FetchError::Status(status)) => {
            format!("fermix.ai answered with an error ({status}).")
        }
        CheckError::Fetch(FetchError::NoRoots(_)) => {
            "This computer has no certificates to check fermix.ai with.".into()
        }
        CheckError::Fetch(_) => "Fermix could not reach fermix.ai.".into(),
        CheckError::Feed(FeedError::Signature | FeedError::UnknownKey(_)) => {
            "The update information is not signed by Fermix, so it was ignored.".into()
        }
        CheckError::Feed(_) => "The update information from fermix.ai could not be read.".into(),
        CheckError::Stale { .. } => "The update information from fermix.ai is older than what \
            Fermix has already seen, so it was ignored."
            .into(),
        CheckError::OutOfDate { generated_at, now } => format!(
            "The update information is out of date. It is from {}.",
            date_words(*generated_at, *now)
        ),
        CheckError::Unknown(version) => {
            format!("This copy of Fermix is version {version}, which no release of Fermix has.")
        }
    }
}

/// Why a download failed.
pub fn download_failure(error: &FetchError) -> String {
    match error {
        FetchError::Status(status) => {
            format!("The download server answered with an error ({status}).")
        }
        FetchError::NoRoots(_) => {
            "This computer has no certificates to check the download server with.".into()
        }
        FetchError::TooLong { .. } | FetchError::Short { .. } | FetchError::Digest => {
            "The download was not the file the update names, so it was deleted.".into()
        }
        FetchError::Disk(e) => format!("The download could not be saved: {e}."),
        FetchError::Cancelled => "The download was cancelled.".into(),
        FetchError::Transport(_) => "Fermix could not reach the download server.".into(),
    }
}

/// Why PackageKit did not install the package.
pub fn install_failure(ending: &Ending) -> String {
    match ending {
        Ending::Fallback(Fallback::Unavailable) => {
            "This system has no software installer Fermix can ask.".into()
        }
        Ending::Fallback(Fallback::NotAuthorized) => {
            "The system did not allow the installation.".into()
        }
        Ending::Fallback(Fallback::Locked) => {
            "Another program is installing software right now.".into()
        }
        Ending::Failed(details) => format!("The system could not install it: {details}"),
        Ending::Installed | Ending::RetryUntrusted => {
            unreachable!("only an install that did not happen has a failure")
        }
    }
}

/// The toast an automatic check shows once for a new version, with Show.
pub fn found_toast(offer: &Offer) -> String {
    available(offer)
}
