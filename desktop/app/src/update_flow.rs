//! The controller's side of in-app updates: how this copy was installed, the
//! checks (at launch once the daemon has settled, then daily, and whenever
//! asked), and the install the dialog runs from the download to the window
//! coming back. Network and disk work runs on worker threads; what the
//! surfaces show is core's (`update::view`). A failed step is never left
//! silent: it says what went wrong and how to do it by hand.

use crate::app::App;
use crate::packagekit;
use crate::shell::plain_toast;
use crate::systemd::reload_units;
use crate::update_dialog::UpdateDialog;
use adw::prelude::*;
use fermix_client::capped::{self, Ended};
use fermix_client::model::EngineIdentity;
use fermix_client::runtime_env::runs_from_package;
use fermix_client::update::check::{check, due, remember, CheckError, Found};
use fermix_client::update::feed::{Package, FEED_URL};
use fermix_client::update::fetch::{native_roots, FetchConfig, FetchError, Fetcher};
use fermix_client::update::identity::{
    engine_alignment, installed_engine, restart_pending, window_replaced, Alignment,
    INSTALLED_ENGINE,
};
use fermix_client::update::keys::production_keys;
use fermix_client::update::packagekit::Ending;
use fermix_client::update::policy::{origin, version_argv, Outcome, OWNER_DPKG, OWNER_RPM};
use fermix_client::update::state::{
    engine_needs_restart, install_command, installation, waiting, Failure, Installation, Manual,
    Stage, Step, RELAUNCH,
};
use fermix_client::update::store::{self, Remembered};
use fermix_client::update::version::Version;
use fermix_client::update::view::{
    check_failure, dialog_view, download_failure, found_toast, install_failure,
};
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// After the first read of the daemon, the launch check waits this long, so
/// it never competes with the window's own first work.
const SETTLE: Duration = Duration::from_secs(10);
/// How often the window asks whether a day has passed since the last check.
const TICK: Duration = Duration::from_secs(60 * 60);
/// How often the dialog reads how far the download is.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
/// How long a package database may take to answer.
const QUERY_CAP: Duration = Duration::from_secs(20);
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the controller keeps for updates beside `State::updates`.
pub struct Updater {
    state_file: PathBuf,
    downloads: PathBuf,
    dialog: RefCell<Option<UpdateDialog>>,
    /// Set to stop the download running now.
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    /// The package downloaded and checked for this version, until installed.
    checked: RefCell<Option<(Version, PathBuf)>>,
    /// The version an automatic check last announced, so it is announced once.
    toasted: Cell<Option<Version>>,
}

impl Updater {
    /// Its files go where XDG says, under the home folder by default.
    pub fn new() -> Updater {
        let home = glib::home_dir();
        let state_home = std::env::var_os("XDG_STATE_HOME");
        let cache_home = std::env::var_os("XDG_CACHE_HOME");
        Updater {
            state_file: store::state_path(state_home.as_deref(), &home),
            downloads: store::download_dir(cache_home.as_deref(), &home),
            dialog: RefCell::default(),
            cancel: RefCell::default(),
            checked: RefCell::default(),
            toasted: Cell::default(),
        }
    }
}

impl App {
    /// Before the first render: what earlier runs remembered. A file that
    /// cannot be read leaves the defaults, which the next save replaces.
    pub fn load_updates(&self) {
        let remembered = match store::load(&self.updater.state_file) {
            Ok(remembered) => remembered,
            Err(e) => {
                glib::g_warning!("fermix", "the update state was not read: {e}");
                Remembered::default()
            }
        };
        self.state.borrow_mut().updates.remembered = remembered;
    }

    /// After the first read of the daemon: last run's downloads go, the
    /// package databases say how this copy was installed, and the checks
    /// start: once now, unless switched off, and then daily.
    pub async fn start_updates(self: &Rc<Self>) {
        let downloads = self.updater.downloads.clone();
        let read = gio::spawn_blocking(move || (clear_downloads(&downloads), read_installation()));
        let ((), found) = read
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        glib::g_info!("fermix", "this copy of Fermix: {found:?}");
        self.state.borrow_mut().updates.installation = Some(found);
        self.read_restart_pending();
        self.render();
        // A dialog opened before the databases answered checks now.
        if self.updater.dialog.borrow().is_some() {
            self.open_updates();
        }
        start_daily_checks(self);
        glib::timeout_future(SETTLE).await;
        let (automatic, checked) = {
            let updates = &self.state.borrow().updates;
            let checked = updates.found.is_some() || updates.failure.is_some();
            (updates.remembered.automatic, checked || updates.checking)
        };
        if automatic && !checked {
            self.clone().check_updates(false).await;
        }
    }

    /// One check of the feed. `asked`: the person asked for it, so a version
    /// they skipped shows; an automatic one announces a new version once.
    pub async fn check_updates(self: Rc<Self>, asked: bool) {
        let (installation, remembered) = {
            let updates = &self.state.borrow().updates;
            if updates.checking || updates.busy() {
                return;
            }
            let Some(installation) = updates.installation.clone() else {
                return;
            };
            (installation, updates.remembered.clone())
        };
        self.state.borrow_mut().updates.checking = true;
        self.render();
        let job = move || check_feed(&installation, &remembered);
        let answer = gio::spawn_blocking(job)
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        self.checked_feed(&answer);
        self.save_updates();
        self.render();
        if !asked {
            self.announce_update();
        }
    }

    fn checked_feed(&self, answer: &Result<Found, CheckError>) {
        let mut state = self.state.borrow_mut();
        let updates = &mut state.updates;
        updates.checking = false;
        remember(&mut updates.remembered, unix_now(), answer);
        match answer {
            Ok(found) => {
                updates.found = Some(found.outcome.clone());
                updates.failure = None;
            }
            Err(e) => {
                glib::g_warning!("fermix", "the update check failed: {e:?}");
                updates.failure = Some(check_failure(e));
            }
        }
    }

    /// A toast with Show, once per version an automatic check finds.
    fn announce_update(&self) {
        let found = {
            let state = self.state.borrow();
            let offer = state.updates.announced();
            offer.map(|o| (o.release.version, found_toast(o)))
        };
        let Some((version, sentence)) = found else {
            return;
        };
        if self.updater.toasted.replace(Some(version)) == Some(version) {
            return;
        }
        let toast = plain_toast(&sentence);
        toast.set_button_label(Some("Show"));
        toast.set_action_name(Some("win.updates-open"));
        self.shell.toasts.add_toast(toast);
    }

    /// Whether what is installed is newer than what runs: the engine's build,
    /// or this window's own file. Only for a packaged window, which a
    /// restart would replace with the installed one.
    pub fn read_restart_pending(&self) {
        let packaged = matches!(
            self.state.borrow().updates.installation,
            Some(Installation::Package { .. })
        );
        let pending = packaged && restart_pending(&self.engine_alignment(), replaced_window());
        self.state.borrow_mut().updates.restart_pending = pending;
    }

    fn engine_alignment(&self) -> Alignment {
        let running = self.state.borrow().snapshot().map(|s| EngineIdentity {
            product_version: s.version.clone(),
            build_id: s.build_id.clone(),
            pid: s.pid.clone(),
            architecture: s.architecture.clone(),
            distribution_identity: s.distribution.clone(),
        });
        engine_alignment(read_installed_engine().as_ref(), running.as_ref())
    }

    /// The tray's row, the main menu, Home's and Settings' buttons: the dialog,
    /// on the update found, the restart pending, or a check run now.
    pub fn open_updates(self: &Rc<Self>) {
        self.shell.window.present();
        if self.updater.dialog.borrow().is_none() {
            self.present_update_dialog();
        }
        let start_check = {
            let updates = &self.state.borrow().updates;
            let shown = updates.offer().is_some() || updates.restart_pending;
            let checks = matches!(updates.installation, Some(Installation::Package { .. }));
            checks && !shown && !updates.checking && !updates.busy()
        };
        if start_check {
            glib::spawn_future_local(self.clone().check_updates(true));
        }
    }

    fn present_update_dialog(self: &Rc<Self>) {
        let dialog = UpdateDialog::new();
        let weak = Rc::downgrade(self);
        dialog.dialog.connect_closed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.update_dialog_closed();
            }
        });
        dialog.dialog.present(Some(&self.shell.window));
        *self.updater.dialog.borrow_mut() = Some(dialog);
        self.redraw_update_dialog();
    }

    /// Closing the dialog stops a download, and forgets a failure, so the
    /// update shows again next time.
    fn update_dialog_closed(&self) {
        self.updater.dialog.borrow_mut().take();
        self.cancel_download();
        let mut state = self.state.borrow_mut();
        if matches!(state.updates.step, Some(Step::Failed(_))) {
            state.updates.step = None;
        }
    }

    fn close_update_dialog(&self) {
        let dialog = self
            .updater
            .dialog
            .borrow()
            .as_ref()
            .map(|d| d.dialog.clone());
        if let Some(dialog) = dialog {
            dialog.close();
        }
    }

    /// Draws the dialog, if open, from where updates stand.
    pub fn redraw_update_dialog(&self) {
        let view = {
            let state = self.state.borrow();
            let engine = state.snapshot().map(|s| s.version.as_str());
            dialog_view(&state.updates, engine)
        };
        if let Some(dialog) = self.updater.dialog.borrow().as_ref() {
            dialog.show(&view);
        }
    }

    pub fn skip_update(&self) {
        {
            let mut state = self.state.borrow_mut();
            let updates = &mut state.updates;
            // A window action can come from outside the dialog, as over D-Bus.
            let Some(Outcome::Available(offer)) = &mut updates.found else {
                return glib::g_warning!("fermix", "Skip This Version with nothing offered");
            };
            if offer.critical {
                return glib::g_warning!("fermix", "a critical update cannot be skipped");
            }
            offer.skipped = true;
            updates.remembered.skipped = Some(offer.release.version);
        }
        self.save_updates();
        self.close_update_dialog();
        self.render();
    }

    /// The Settings switch. A move that only mirrors what was drawn is ignored.
    pub fn set_automatic(&self, on: bool) {
        {
            let remembered = &mut self.state.borrow_mut().updates.remembered;
            if remembered.automatic == on {
                return;
            }
            remembered.automatic = on;
        }
        self.save_updates();
        self.render();
    }

    pub fn cancel_download(&self) {
        if let Some(cancel) = self.updater.cancel.borrow().as_ref() {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Writes what is remembered, atomically. A failed write is logged, and
    /// the next change tries again.
    fn save_updates(&self) {
        let remembered = self.state.borrow().updates.remembered.clone();
        if let Err(e) = store::save(&self.updater.state_file, &remembered) {
            glib::g_warning!(
                "fermix",
                "the update state was not saved to {}: {e}",
                self.updater.state_file.display()
            );
        }
    }

    fn set_step(&self, step: Option<Step>) {
        self.state.borrow_mut().updates.step = step;
        self.render();
    }

    fn fail(&self, stage: Stage, sentence: String, manual: Manual) {
        glib::g_warning!("fermix", "the update stopped at {stage:?}: {sentence}");
        self.set_step(Some(Step::Failed(Failure {
            stage,
            sentence,
            manual,
        })));
    }

    /// Try Again runs the failed step: the download and install, or the restart.
    pub async fn update_try_again(self: Rc<Self>) {
        let stage = match &self.state.borrow().updates.step {
            Some(Step::Failed(failure)) => failure.stage,
            _ => return,
        };
        match stage {
            Stage::Download | Stage::Install => self.install_update().await,
            Stage::Restart => self.finish_update().await,
        }
    }

    /// Install Update: the download, held to the feed's size and sha256, then
    /// PackageKit, then the restart. One committed step from here on.
    pub async fn install_update(self: Rc<Self>) {
        let Some((package, version)) = self.chosen_package() else {
            glib::g_warning!(
                "fermix",
                "Install Update with nothing this copy can install"
            );
            return;
        };
        let Some(file) = self.download(&package, version).await else {
            return;
        };
        self.set_step(Some(Step::Asking));
        let weak = Rc::downgrade(&self);
        let ending = packagekit::install(&file, move |progress| {
            if let Some(app) = weak.upgrade() {
                app.installing(progress);
            }
        })
        .await;
        if self.installed(&ending, &file, version).await {
            self.finish_update().await;
        }
    }

    fn chosen_package(&self) -> Option<(Package, Version)> {
        let state = self.state.borrow();
        let updates = &state.updates;
        if updates.busy() {
            return None;
        }
        let offer = updates.offer()?;
        let package = offer
            .release
            .packages
            .pick(updates.arch?, updates.format()?);
        Some((package.clone(), offer.release.version))
    }

    fn installing(&self, progress: packagekit::Progress) {
        self.state.borrow_mut().updates.step = Some(Step::Installing {
            percent: progress.percent,
            waiting: waiting(progress.status),
        });
        self.redraw_update_dialog();
    }
}

impl App {
    /// The package, from an earlier try when it was already checked. `None`
    /// once the step says why there is none.
    async fn download(self: &Rc<Self>, package: &Package, version: Version) -> Option<PathBuf> {
        let checked = self.updater.checked.borrow().clone();
        if let Some((_, file)) = checked.filter(|(v, f)| *v == version && f.is_file()) {
            return Some(file);
        }
        let file = self.updater.downloads.join(&package.file);
        let (got, cancel) = (
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        *self.updater.cancel.borrow_mut() = Some(cancel.clone());
        self.set_step(Some(Step::Downloading {
            got: 0,
            size: package.size,
        }));
        let ticker = follow_download(self, got.clone(), package.size);
        let (dir, wanted, to) = (
            self.updater.downloads.clone(),
            package.clone(),
            file.clone(),
        );
        let job = move || fetch_package(&dir, &wanted, &to, &got, &cancel);
        let fetched = gio::spawn_blocking(job).await;
        ticker.remove();
        self.updater.cancel.borrow_mut().take();
        match fetched.unwrap_or_else(|panic| std::panic::resume_unwind(panic)) {
            Ok(()) => {
                *self.updater.checked.borrow_mut() = Some((version, file.clone()));
                Some(file)
            }
            Err(FetchError::Cancelled) => {
                self.set_step(None);
                None
            }
            Err(e) => {
                let manual = Manual::Link {
                    url: package.url.clone(),
                };
                self.fail(Stage::Download, download_failure(&e), manual);
                None
            }
        }
    }

    /// After PackageKit: whether the package database now names `version`.
    /// The package is deleted once installed; on a failure it stays for the
    /// command that installs it by hand, until the next launch.
    async fn installed(&self, ending: &Ending, file: &Path, version: Version) -> bool {
        let format = self
            .state
            .borrow()
            .updates
            .format()
            .expect("only a copy a database owns installs");
        let failure = match ending {
            Ending::Installed => None,
            Ending::RetryUntrusted => {
                Some("The system asked for a second untrusted install.".into())
            }
            ending => Some(install_failure(ending)),
        };
        if let Some(sentence) = failure {
            let command = install_command(format, file);
            let manual = Manual::Command {
                command,
                file: file.to_owned(),
            };
            self.fail(Stage::Install, sentence, manual);
            return false;
        }
        self.updater.checked.borrow_mut().take();
        if let Err(e) = std::fs::remove_file(file) {
            glib::g_warning!("fermix", "the installed package was not deleted: {e}");
        }
        let argv = version_argv(format);
        let now = gio::spawn_blocking(move || query(&argv)).await;
        let now = now.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        let now = now.as_deref().map(str::trim).unwrap_or("nothing");
        if now != version.to_string() {
            let sentence =
                format!("The system installed it, but reports Fermix {now}, not {version}.");
            self.fail(Stage::Install, sentence, Manual::Reopen);
            return false;
        }
        true
    }

    /// The restart that finishes an update: the engine, when another build of
    /// it is installed, then this window, replaced by the installed one.
    pub async fn finish_update(self: Rc<Self>) {
        let packaged = matches!(
            self.state.borrow().updates.installation,
            Some(Installation::Package { .. })
        );
        if !packaged || self.state.borrow().updates.busy() {
            return glib::g_warning!("fermix", "no update to restart into");
        }
        self.set_step(Some(Step::Restarting));
        if let Err(sentence) = self.restart_engine().await {
            return self.fail(Stage::Restart, sentence, Manual::Reopen);
        }
        let e = relaunch();
        self.fail(
            Stage::Restart,
            format!("Fermix could not open again: {e}."),
            Manual::Reopen,
        );
    }

    /// systemd reads the new unit, the engine restarts through the usual
    /// restart when its build changed, and its hello must then name the
    /// installed build. An engine that is not this package's is left alone,
    /// and one that keeps running needs no reload to go on.
    async fn restart_engine(&self) -> Result<(), String> {
        let reloaded = reload_units().await;
        self.refresh().await;
        if engine_needs_restart(&self.engine_alignment()) {
            reloaded?;
            self.restart_now().await?;
        } else if let Err(sentence) = reloaded {
            glib::g_warning!("fermix", "systemd did not reload its units: {sentence}");
        }
        match self.engine_alignment() {
            Alignment::Aligned | Alignment::NotRunning | Alignment::OwnershipConflict(_) => Ok(()),
            Alignment::PendingRestart => {
                Err("The engine still runs the version from before the update.".into())
            }
            Alignment::Unknown => {
                Err("Fermix could not tell whether its engine is the updated one.".into())
            }
        }
    }
}

/// The dialog's download progress, read every `PROGRESS_EVERY` until the
/// caller removes it. Every byte in, the step becomes the check of them.
fn follow_download(app: &Rc<App>, got: Arc<AtomicU64>, size: u64) -> glib::SourceId {
    let weak = Rc::downgrade(app);
    glib::timeout_add_local(PROGRESS_EVERY, move || {
        let Some(app) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let got = got.load(Ordering::SeqCst);
        let step = if got >= size {
            Step::Verifying
        } else {
            Step::Downloading { got, size }
        };
        app.state.borrow_mut().updates.step = Some(step);
        app.redraw_update_dialog();
        glib::ControlFlow::Continue
    })
}

fn start_daily_checks(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    glib::timeout_add_local(TICK, move || {
        let Some(app) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if due(&app.state.borrow().updates.remembered, unix_now()) {
            glib::spawn_future_local(app.clone().check_updates(false));
        }
        glib::ControlFlow::Continue
    });
}

/// Replaces this process with the installed window, which takes the app's
/// name over. The process id stays, so a session or D-Bus activation that
/// started the window still has it. Returns only when that failed.
fn relaunch() -> std::io::Error {
    let (program, args) = RELAUNCH.split_first().expect("RELAUNCH names its program");
    glib::g_info!("fermix", "opening the updated Fermix: {RELAUNCH:?}");
    std::process::Command::new(program).args(args).exec()
}

fn check_feed(installation: &Installation, remembered: &Remembered) -> Result<Found, CheckError> {
    let fetcher = production_fetcher().map_err(CheckError::Fetch)?;
    check(
        &fetcher,
        FEED_URL,
        &production_keys(),
        installation,
        remembered,
        unix_now(),
    )
}

fn production_fetcher() -> Result<Fetcher, FetchError> {
    let (roots, unreadable) = native_roots()?;
    for problem in unreadable {
        glib::g_info!("fermix", "a CA certificate was skipped: {problem}");
    }
    Fetcher::new(FetchConfig::production(roots))
}

fn fetch_package(
    dir: &Path,
    package: &Package,
    to: &Path,
    got: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), FetchError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| FetchError::Disk(format!("{}: {e}", dir.display())))?;
    let fetcher = production_fetcher()?;
    let mut progress = |bytes: u64| got.store(bytes, Ordering::SeqCst);
    fetcher.download(&package.url, package, to, &mut progress, cancel)
}

fn clear_downloads(dir: &Path) {
    match store::clear_downloads(dir) {
        Ok(0) => {}
        Ok(count) => glib::g_info!("fermix", "deleted {count} leftover update downloads"),
        Err(e) => glib::g_warning!("fermix", "{} was not cleared: {e}", dir.display()),
    }
}

/// How this copy was installed, asked of the package databases.
fn read_installation() -> Installation {
    let packaged = match std::fs::read_link("/proc/self/exe") {
        Ok(exe) => runs_from_package(&exe),
        Err(e) => {
            glib::g_warning!("fermix", "the window's own file is unknown: {e}");
            false
        }
    };
    if !packaged {
        return installation(false, None, None, VERSION);
    }
    let format = origin(query(&OWNER_DPKG).as_deref(), query(&OWNER_RPM).as_deref());
    let version = format.and_then(|format| query(&version_argv(format)));
    installation(true, format, version.as_deref().map(str::trim), VERSION)
}

/// What a package tool printed, or `None` where it is not installed, did
/// not run, or ran past `QUERY_CAP`.
fn query(argv: &[&str]) -> Option<String> {
    match capped::run(argv, QUERY_CAP) {
        Ok(Ended::Exited(out)) => Some(String::from_utf8_lossy(&out).into_owned()),
        Ok(Ended::Capped) => {
            glib::g_warning!("fermix", "{argv:?} ran past {QUERY_CAP:?}");
            None
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            glib::g_warning!("fermix", "{argv:?} did not run: {e}");
            None
        }
    }
}

/// The engine the package installed. None is installed on a development
/// machine; one that cannot be read is logged.
fn read_installed_engine() -> Option<EngineIdentity> {
    let bytes = match std::fs::read(INSTALLED_ENGINE) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            glib::g_warning!("fermix", "{INSTALLED_ENGINE} was not read: {e}");
            return None;
        }
    };
    installed_engine(&bytes)
        .inspect_err(|e| glib::g_warning!("fermix", "{e}"))
        .ok()
}

fn replaced_window() -> bool {
    match std::fs::read_link("/proc/self/exe") {
        Ok(exe) => window_replaced(&exe),
        Err(e) => {
            glib::g_warning!("fermix", "the window's own file is unknown: {e}");
            false
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970")
        .as_secs()
}
