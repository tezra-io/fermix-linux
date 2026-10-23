//! The controller's side of in-app updates: how this copy was installed, the
//! checks (at launch once the daemon has settled, then daily, and whenever
//! asked), the dialog, and the window as the `Host` the install runs on
//! (`update_install`). Network and disk work runs on worker threads; what the
//! surfaces show is core's (`update::view`). A failed step is never left
//! silent: it says what went wrong and how to do it by hand.

use crate::app::App;
use crate::packagekit;
use crate::shell::plain_toast;
use crate::systemd::reload_units;
use crate::update_dialog::UpdateDialog;
use crate::update_install::{Downloads, Host};
use adw::prelude::*;
use fermix_client::capped::{self, Ended};
use fermix_client::model::EngineIdentity;
use fermix_client::runtime_env::runs_from_package;
use fermix_client::update::check::{check, due, remember, CheckError, Found};
use fermix_client::update::feed::{Format, FEED_URL};
use fermix_client::update::fetch::{native_roots, FetchConfig, FetchError, Fetcher};
use fermix_client::update::identity::{
    engine_alignment, installed_engine, restart_pending, window_replaced, Alignment,
    INSTALLED_ENGINE,
};
use fermix_client::update::keys::production_keys;
use fermix_client::update::packagekit::Ending;
use fermix_client::update::policy::{origin, version_argv, Outcome, OWNER_DPKG, OWNER_RPM};
use fermix_client::update::state::{installation, Installation, Step, Updates};
use fermix_client::update::store::{self, Remembered};
use fermix_client::update::version::Version;
use fermix_client::update::view::{check_failure, dialog_view, found_toast};
use gtk::{gio, glib};
use std::cell::{Cell, RefCell, RefMut};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// After the first read of the daemon, the launch check waits this long, so
/// it never competes with the window's own first work.
const SETTLE: Duration = Duration::from_secs(10);
/// How often the window asks whether a day has passed since the last check.
const TICK: Duration = Duration::from_secs(60 * 60);
/// How long a package database may take to answer.
const QUERY_CAP: Duration = Duration::from_secs(20);
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the controller keeps for updates beside `State::updates`.
pub struct Updater {
    state_file: PathBuf,
    pub downloads: Downloads,
    dialog: RefCell<Option<UpdateDialog>>,
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
            downloads: Downloads::new(store::download_dir(cache_home.as_deref(), &home)),
            dialog: RefCell::default(),
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
        let downloads = self.updater.downloads.dir.clone();
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
        self.updater.downloads.cancel();
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
}

/// The real system: the host's PackageKit, package database and systemd,
/// the daemon, and exec.
impl Host for App {
    fn updates(&self) -> RefMut<'_, Updates> {
        RefMut::map(self.state.borrow_mut(), |state| &mut state.updates)
    }

    fn downloads(&self) -> &Downloads {
        &self.updater.downloads
    }

    fn render(&self) {
        App::render(self);
    }

    fn redraw_update_dialog(&self) {
        App::redraw_update_dialog(self);
    }

    async fn fetcher(&self) -> Result<Fetcher, FetchError> {
        gio::spawn_blocking(production_fetcher)
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    async fn install_package(
        &self,
        file: &Path,
        flags: u64,
        progress: impl Fn(packagekit::Progress) + 'static,
    ) -> Ending {
        packagekit::install(file, flags, progress).await
    }

    async fn installed_version(&self, format: Format) -> Option<String> {
        let argv = version_argv(format);
        gio::spawn_blocking(move || query(&argv))
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    async fn reload_units(&self) -> Result<(), String> {
        reload_units().await
    }

    async fn refresh(&self) {
        App::refresh(self).await;
    }

    fn engine_alignment(&self) -> Alignment {
        App::engine_alignment(self)
    }

    async fn restart_now(&self) -> Result<(), String> {
        App::restart_now(self).await
    }

    /// The process id stays, so a session or D-Bus activation that started
    /// the window still has it, and the installed window takes the app's name
    /// over (`RELAUNCH`).
    fn exec(&self, argv: &[&str]) -> std::io::Error {
        let (program, args) = argv.split_first().expect("exec names its program");
        std::process::Command::new(program).args(args).exec()
    }
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
