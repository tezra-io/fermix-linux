//! The install the dialog runs, from Install Update to the window coming
//! back: the package, held to the feed's size and sha256, then PackageKit,
//! the package database's word on what is installed, and the restart. One
//! committed step from the download on. It runs on a `Host`, which holds the
//! window's update state and reaches everything beyond it, so a test can
//! stand in for the package database, systemd, the engine and exec.

use crate::packagekit::Progress;
use fermix_client::update::feed::{Format, Package};
use fermix_client::update::fetch::{FetchError, Fetcher};
use fermix_client::update::identity::Alignment;
use fermix_client::update::packagekit::Ending;
use fermix_client::update::state::{
    engine_needs_restart, install_command, waiting, Failure, Installation, Manual, Stage, Step,
    Updates, RELAUNCH,
};
use fermix_client::update::version::Version;
use fermix_client::update::view::{download_failure, install_failure};
use gtk::{gio, glib};
use std::cell::{RefCell, RefMut};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How often the dialog reads how far the download is.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
/// Reads for an hour at most, the longest a download may take
/// (`Timeouts::PRODUCTION.download`).
const PROGRESS_READS: u32 = 18_000;

/// What the install runs on. `App` is the real one.
pub trait Host: 'static {
    /// The window's update state. Only the controller changes it.
    fn updates(&self) -> RefMut<'_, Updates>;
    fn downloads(&self) -> &Downloads;
    /// Redraws every surface, the dialog included.
    fn render(&self);
    fn redraw_update_dialog(&self);
    /// The HTTPS client packages download through.
    async fn fetcher(&self) -> Result<Fetcher, FetchError>;
    /// PackageKit's InstallFiles, as `packagekit::install` runs it.
    async fn install_package(
        &self,
        file: &Path,
        progress: impl Fn(Progress) + Clone + 'static,
    ) -> Ending;
    /// What the package database names as installed, when it answered.
    async fn installed_version(&self, format: Format) -> Option<String>;
    /// systemd reads the unit files again.
    async fn reload_units(&self) -> Result<(), String>;
    /// Reads the daemon again.
    async fn refresh(&self);
    /// The running engine against the installed one, as last read.
    fn engine_alignment(&self) -> Alignment;
    /// The engine's restart, as Restart Fermix runs it.
    async fn restart_now(&self) -> Result<(), String>;
    /// Replaces this process with `argv`. Returns only when that failed.
    fn exec(&self, argv: &[&str]) -> std::io::Error;
}

/// What the install keeps between its steps.
pub struct Downloads {
    /// Where packages download to, which only this app writes.
    pub dir: PathBuf,
    /// Set to stop the download running now.
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    /// The package downloaded and checked for this version, until installed.
    checked: RefCell<Option<(Version, PathBuf)>>,
}

impl Downloads {
    pub fn new(dir: PathBuf) -> Downloads {
        Downloads {
            dir,
            cancel: RefCell::default(),
            checked: RefCell::default(),
        }
    }

    /// Stops the download running now, if any.
    pub fn cancel(&self) {
        if let Some(cancel) = self.cancel.borrow().as_ref() {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// Try Again runs the failed step: the download and install, or the restart.
pub async fn update_try_again<H: Host>(host: Rc<H>) {
    let stage = match &host.updates().step {
        Some(Step::Failed(failure)) => failure.stage,
        _ => return,
    };
    match stage {
        Stage::Download | Stage::Install => install_update(host).await,
        Stage::Restart => finish_update(&*host).await,
    }
}

/// Install Update: the download, held to the feed's size and sha256, then
/// PackageKit, then the restart. One committed step from here on.
pub async fn install_update<H: Host>(host: Rc<H>) {
    let Some((package, version)) = chosen_package(&*host) else {
        glib::g_warning!(
            "fermix",
            "Install Update with nothing this copy can install"
        );
        return;
    };
    let Some(file) = download(&host, &package, version).await else {
        return;
    };
    set_step(&*host, Some(Step::Asking));
    let weak = Rc::downgrade(&host);
    let ending = host
        .install_package(&file, move |progress| {
            if let Some(host) = weak.upgrade() {
                installing(&*host, progress);
            }
        })
        .await;
    if installed(&*host, &ending, &file, version).await {
        finish_update(&*host).await;
    }
}

fn chosen_package(host: &impl Host) -> Option<(Package, Version)> {
    let updates = host.updates();
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

fn installing(host: &impl Host, progress: Progress) {
    host.updates().step = Some(Step::Installing {
        percent: progress.percent,
        waiting: waiting(progress.status),
    });
    host.redraw_update_dialog();
}

/// The package, from an earlier try when it was already checked. `None`
/// once the step says why there is none.
async fn download<H: Host>(host: &Rc<H>, package: &Package, version: Version) -> Option<PathBuf> {
    let checked = host.downloads().checked.borrow().clone();
    if let Some((_, file)) = checked.filter(|(v, f)| *v == version && f.is_file()) {
        return Some(file);
    }
    let file = host.downloads().dir.join(&package.file);
    let (got, cancel) = (
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    *host.downloads().cancel.borrow_mut() = Some(cancel.clone());
    set_step(
        &**host,
        Some(Step::Downloading {
            got: 0,
            size: package.size,
        }),
    );
    let ticker = follow_download(host, got.clone(), package.size);
    let fetched = fetch(&**host, package, &file, got, cancel).await;
    ticker.abort();
    host.downloads().cancel.borrow_mut().take();
    match fetched {
        Ok(()) => {
            *host.downloads().checked.borrow_mut() = Some((version, file.clone()));
            Some(file)
        }
        Err(FetchError::Cancelled) => {
            set_step(&**host, None);
            None
        }
        Err(e) => {
            let manual = Manual::Link {
                url: package.url.clone(),
            };
            fail(&**host, Stage::Download, download_failure(&e), manual);
            None
        }
    }
}

/// Downloads `package` to `to` on a worker thread.
async fn fetch(
    host: &impl Host,
    package: &Package,
    to: &Path,
    got: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
) -> Result<(), FetchError> {
    let fetcher = host.fetcher().await?;
    let (dir, wanted, to) = (host.downloads().dir.clone(), package.clone(), to.to_owned());
    let job = move || fetch_package(&fetcher, &dir, &wanted, &to, &got, &cancel);
    gio::spawn_blocking(job)
        .await
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// After PackageKit: whether the package database now names `version`.
/// The package is deleted once installed; on a failure it stays for the
/// command that installs it by hand, until the next launch.
async fn installed(host: &impl Host, ending: &Ending, file: &Path, version: Version) -> bool {
    let format = host
        .updates()
        .format()
        .expect("only a copy a database owns installs");
    let failure = match ending {
        Ending::Installed => None,
        Ending::RetryUntrusted => Some("The system asked for a second untrusted install.".into()),
        ending => Some(install_failure(ending)),
    };
    if let Some(sentence) = failure {
        let command = install_command(format, file);
        let manual = Manual::Command {
            command,
            file: file.to_owned(),
        };
        fail(host, Stage::Install, sentence, manual);
        return false;
    }
    host.downloads().checked.borrow_mut().take();
    if let Err(e) = std::fs::remove_file(file) {
        glib::g_warning!("fermix", "the installed package was not deleted: {e}");
    }
    let now = host.installed_version(format).await;
    let now = now.as_deref().map(str::trim).unwrap_or("nothing");
    if now != version.to_string() {
        let sentence = format!("The system installed it, but reports Fermix {now}, not {version}.");
        fail(host, Stage::Install, sentence, Manual::Reopen);
        return false;
    }
    true
}

/// The restart that finishes an update: the engine, when another build of
/// it is installed, then this window, replaced by the installed one.
pub async fn finish_update(host: &impl Host) {
    let packaged = matches!(
        host.updates().installation,
        Some(Installation::Package { .. })
    );
    if !packaged || host.updates().busy() {
        return glib::g_warning!("fermix", "no update to restart into");
    }
    set_step(host, Some(Step::Restarting));
    if let Err(sentence) = restart_engine(host).await {
        return fail(host, Stage::Restart, sentence, Manual::Reopen);
    }
    glib::g_info!("fermix", "opening the updated Fermix: {RELAUNCH:?}");
    let e = host.exec(&RELAUNCH);
    fail(
        host,
        Stage::Restart,
        format!("Fermix could not open again: {e}."),
        Manual::Reopen,
    );
}

/// systemd reads the new unit, the engine restarts through the usual
/// restart when its build changed, and its hello must then name the
/// installed build. An engine that is not this package's is left alone,
/// and one that keeps running needs no reload to go on.
async fn restart_engine(host: &impl Host) -> Result<(), String> {
    let reloaded = host.reload_units().await;
    host.refresh().await;
    if engine_needs_restart(&host.engine_alignment()) {
        reloaded?;
        host.restart_now().await?;
    } else if let Err(sentence) = reloaded {
        glib::g_warning!("fermix", "systemd did not reload its units: {sentence}");
    }
    match host.engine_alignment() {
        Alignment::Aligned | Alignment::NotRunning | Alignment::OwnershipConflict(_) => Ok(()),
        Alignment::PendingRestart => {
            Err("The engine still runs the version from before the update.".into())
        }
        Alignment::Unknown => {
            Err("Fermix could not tell whether its engine is the updated one.".into())
        }
    }
}

fn set_step(host: &impl Host, step: Option<Step>) {
    host.updates().step = step;
    host.render();
}

fn fail(host: &impl Host, stage: Stage, sentence: String, manual: Manual) {
    glib::g_warning!("fermix", "the update stopped at {stage:?}: {sentence}");
    set_step(
        host,
        Some(Step::Failed(Failure {
            stage,
            sentence,
            manual,
        })),
    );
}

/// The dialog's download progress, read every `PROGRESS_EVERY` until the
/// caller aborts it. Every byte in, the step becomes the check of them.
fn follow_download<H: Host>(host: &Rc<H>, got: Arc<AtomicU64>, size: u64) -> glib::JoinHandle<()> {
    let weak = Rc::downgrade(host);
    glib::MainContext::ref_thread_default().spawn_local(async move {
        for _ in 0..PROGRESS_READS {
            glib::timeout_future(PROGRESS_EVERY).await;
            let Some(host) = weak.upgrade() else {
                return;
            };
            let got = got.load(Ordering::SeqCst);
            let step = if got >= size {
                Step::Verifying
            } else {
                Step::Downloading { got, size }
            };
            host.updates().step = Some(step);
            host.redraw_update_dialog();
        }
    })
}

fn fetch_package(
    fetcher: &Fetcher,
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
    let mut progress = |bytes: u64| got.store(bytes, Ordering::SeqCst);
    fetcher.download(&package.url, package, to, &mut progress, cancel)
}
