//! The install the dialog runs, from Install Update to the window coming
//! back: the package, held to the feed's size and sha256, then PackageKit
//! (only-trusted, and once more untrusted when PackageKit asks for it, the
//! package checked again before each), the package database's word on what
//! is installed, and the restart. One committed step from the download on. It
//! runs on a `Host`, which holds the window's update state and reaches
//! everything beyond it, so a test can stand in for the package database,
//! systemd, the engine and exec.

use crate::packagekit::Progress;
use fermix_client::update::feed::{Format, Package};
use fermix_client::update::fetch::{check_file, FetchError, Fetcher};
use fermix_client::update::identity::Alignment;
use fermix_client::update::packagekit::{Ending, ONLY_TRUSTED, UNTRUSTED};
use fermix_client::update::state::{
    engine_needs_restart, install_command, may_restart, waiting, Failure, Manual, Restart, Stage,
    Step, Updates, RELAUNCH,
};
use fermix_client::update::store::prepare_downloads;
use fermix_client::update::version::Version;
use fermix_client::update::view::{download_failure, install_failure};
use gtk::{gio, glib};
use std::cell::{RefCell, RefMut};
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
    /// One InstallFiles with `flags`, as `packagekit::install` runs it.
    async fn install_package(
        &self,
        file: &Path,
        flags: u64,
        progress: impl Fn(Progress) + 'static,
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
    /// This app's downloads folder, private to the user.
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
    let Some(ending) = hand_over(&host, &package, &file).await else {
        return;
    };
    if !installed(&*host, &ending, &file, version).await {
        return;
    }
    // The install's own step goes on to the restart; `busy` still holds it.
    if !may_restart(&host.updates(), Restart::AfterInstall) {
        let step = host.updates().step.clone();
        glib::g_warning!("fermix", "an install confirmed at {step:?}");
        let sentence = "The update installed, but Fermix did not restart.".into();
        return fail(&*host, Stage::Restart, sentence, Manual::Reopen);
    }
    restart_into_update(&*host).await;
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

/// PackageKit's InstallFiles: only-trusted, then once more untrusted when
/// PackageKit says the package carries no GPG signature, as any local .deb
/// and the unsigned .rpm do. Before each, the package is checked again.
/// `None` once the step says why it was not handed over.
async fn hand_over<H: Host>(host: &Rc<H>, package: &Package, file: &Path) -> Option<Ending> {
    let mut ending = Ending::RetryUntrusted;
    for flags in [ONLY_TRUSTED, UNTRUSTED] {
        if !still_checked(&**host, package, file).await {
            return None;
        }
        set_step(&**host, Some(Step::Asking));
        ending = host.install_package(file, flags, progress_of(host)).await;
        if ending != Ending::RetryUntrusted {
            break;
        }
        glib::g_info!(
            "fermix",
            "PackageKit wants the unsigned package installed untrusted"
        );
    }
    Some(ending)
}

/// The package read again and held to the feed's size and sha256, so one
/// that changed since its download, or during the pass before, is never
/// handed over. PackageKit then opens it by its path: a process of this
/// user's could still rewrite it, which is outside Fermix's boundary
/// (SECURITY.md). One that changed is deleted, and the step says the
/// download failed.
async fn still_checked(host: &impl Host, package: &Package, file: &Path) -> bool {
    set_step(host, Some(Step::Verifying));
    let (wanted, at) = (package.clone(), file.to_owned());
    let checked = gio::spawn_blocking(move || check_file(&at, &wanted))
        .await
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    let Err(e) = checked else {
        return true;
    };
    host.downloads().checked.borrow_mut().take();
    match std::fs::remove_file(file) {
        Ok(()) => {}
        Err(gone) if gone.kind() == std::io::ErrorKind::NotFound => {}
        Err(left) => glib::g_warning!("fermix", "a refused package was not deleted: {left}"),
    }
    let manual = Manual::Link {
        url: package.url.clone(),
    };
    fail(host, Stage::Download, download_failure(&e), manual);
    false
}

/// What PackageKit says of the transaction, drawn as the install's step.
fn progress_of<H: Host>(host: &Rc<H>) -> impl Fn(Progress) + 'static {
    let weak = Rc::downgrade(host);
    move |progress| {
        if let Some(host) = weak.upgrade() {
            installing(&*host, progress);
        }
    }
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
/// The package is deleted once it does; until then it stays for the command
/// that installs what is at its path, until the next launch.
async fn installed(host: &impl Host, ending: &Ending, file: &Path, version: Version) -> bool {
    let format = host
        .updates()
        .format()
        .expect("only a copy a database owns installs");
    let by_hand = Manual::Command {
        command: install_command(format, file),
        file: file.to_owned(),
    };
    let failure = match ending {
        Ending::Installed => None,
        Ending::RetryUntrusted => Some("The system asked for a second untrusted install.".into()),
        ending => Some(install_failure(ending)),
    };
    if let Some(sentence) = failure {
        fail(host, Stage::Install, sentence, by_hand);
        return false;
    }
    let now = host.installed_version(format).await;
    let now = now.as_deref().map(str::trim).unwrap_or("nothing");
    if now != version.to_string() {
        let sentence = format!("The system installed it, but reports Fermix {now}, not {version}.");
        fail(host, Stage::Install, sentence, by_hand);
        return false;
    }
    host.downloads().checked.borrow_mut().take();
    if let Err(e) = std::fs::remove_file(file) {
        glib::g_warning!("fermix", "the installed package was not deleted: {e}");
    }
    true
}

/// Restart, and Try Again after a failed restart: the restart into an
/// update installed already.
pub async fn finish_update(host: &impl Host) {
    if !may_restart(&host.updates(), Restart::Asked) {
        return glib::g_warning!("fermix", "no update to restart into");
    }
    restart_into_update(host).await;
}

/// The restart that finishes an update: the engine, when another build of
/// it is installed, then this window, replaced by the installed one.
async fn restart_into_update(host: &impl Host) {
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

/// When the engine's build changed, systemd reads the new unit and the
/// engine restarts through the usual restart; its hello must then name the
/// installed build. An engine that keeps running (a desktop-only release, or
/// one that is not this package's) is left alone, with no reload: the CLI
/// reloads before any later lifecycle change (M38 §9.2).
async fn restart_engine(host: &impl Host) -> Result<(), String> {
    host.refresh().await;
    if engine_needs_restart(&host.engine_alignment()) {
        host.reload_units().await?;
        host.restart_now().await?;
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
    prepare_downloads(dir).map_err(|e| FetchError::Disk(format!("{}: {e}", dir.display())))?;
    let mut progress = |bytes: u64| got.store(bytes, Ordering::SeqCst);
    fetcher.download(&package.url, package, to, &mut progress, cancel)
}

#[cfg(test)]
mod tests {
    //! The whole install, offer to exec: the package served from 127.0.0.1,
    //! PackageKit a fake on a bus of the test's own, and the package database,
    //! systemd, the engine and exec stood in for, each noting what it was
    //! asked. Nothing reaches the host's buses, tools or network.

    use super::*;
    use crate::packagekit::{install_on, TRANSACTION_CAP};
    use crate::packagekit_fake::{fake, Answer, PrivateBus, Seen, TEST_CAP};
    use fermix_client::update::feed::{Arch, Engine, Packages, Pair, Release};
    use fermix_client::update::fetch::{FetchConfig, Timeouts};
    use fermix_client::update::packagekit::error;
    use fermix_client::update::policy::{Offer, Outcome};
    use fermix_client::update::state::Installation;
    use fermix_client::update::store::Remembered;
    use std::future::Future;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicU32;

    const FILE: &str = "fermix-desktop_0.5.0_amd64.deb";
    const BYTES: &[u8] = b"test package fermix-desktop_0.5.0_amd64.deb\n";
    /// The fetcher needs a root; the test's server is plain http, so TLS never reads it.
    const ROOT: &[u8] = include_bytes!("../../core/tests/fixtures/update-tls/test-server.der");
    const WAIT: Duration = Duration::from_secs(5);
    /// The longest request head the test's server reads.
    const HEAD_CAP: usize = 16 * 1024;
    const EXEC: &str = "exec /usr/bin/fermix-desktop --gapplication-replace";
    static RUNS: AtomicU32 = AtomicU32::new(0);

    /// What the test plants.
    struct Plant {
        /// PackageKit's answer to each InstallFiles, in turn.
        answers: Vec<Answer>,
        /// The package's bytes change on disk during the first InstallFiles.
        tamper: bool,
        /// What the package database names as installed after the install.
        version: &'static str,
        /// The engine until it restarts, when it becomes the installed build.
        engine: Alignment,
    }

    /// An install PackageKit allows at once, of 0.5.0 that the database then
    /// names, the engine on another build until it restarts.
    fn plant() -> Plant {
        Plant {
            answers: vec![Answer::Succeed],
            tamper: false,
            version: "0.5.0\n",
            engine: Alignment::PendingRestart,
        }
    }

    /// The system the install runs on, as planted, and what it was asked.
    struct Stand {
        updates: RefCell<Updates>,
        downloads: Downloads,
        bus: gio::DBusConnection,
        tamper: bool,
        version: &'static str,
        engine: RefCell<Alignment>,
        /// Each seam the install reached, in order.
        heard: RefCell<Vec<String>>,
        /// Each step the surfaces were drawn with, a repeat once.
        drawn: RefCell<Vec<String>>,
    }

    impl Stand {
        fn hear(&self, what: &str) {
            self.heard.borrow_mut().push(what.to_owned());
        }
    }

    impl Host for Stand {
        fn updates(&self) -> RefMut<'_, Updates> {
            self.updates.borrow_mut()
        }

        fn downloads(&self) -> &Downloads {
            &self.downloads
        }

        fn render(&self) {
            let name = step_name(self.updates.borrow().step.as_ref());
            let mut drawn = self.drawn.borrow_mut();
            if drawn.last() != Some(&name) {
                drawn.push(name);
            }
        }

        fn redraw_update_dialog(&self) {
            self.render();
        }

        async fn fetcher(&self) -> Result<Fetcher, FetchError> {
            Fetcher::new(FetchConfig {
                roots: vec![ROOT.to_vec()],
                https_only: false,
                proxy_from_env: false,
                timeouts: Timeouts {
                    connect: WAIT,
                    feed: WAIT,
                    per_read: WAIT,
                    download: WAIT,
                },
            })
        }

        async fn install_package(
            &self,
            file: &Path,
            flags: u64,
            progress: impl Fn(Progress) + 'static,
        ) -> Ending {
            self.hear("PackageKit");
            let ending = install_on(&self.bus, file, flags, TRANSACTION_CAP, progress).await;
            if self.tamper && flags == ONLY_TRUSTED {
                std::fs::write(file, b"test package swapped while the password was asked\n")
                    .unwrap();
            }
            ending
        }

        async fn installed_version(&self, format: Format) -> Option<String> {
            assert_eq!(format, Format::Deb);
            self.hear("package database");
            Some(self.version.to_owned())
        }

        async fn reload_units(&self) -> Result<(), String> {
            self.hear("systemd reload");
            Ok(())
        }

        async fn refresh(&self) {}

        fn engine_alignment(&self) -> Alignment {
            self.engine.borrow().clone()
        }

        async fn restart_now(&self) -> Result<(), String> {
            self.hear("engine restart");
            *self.engine.borrow_mut() = Alignment::Aligned;
            Ok(())
        }

        fn exec(&self, argv: &[&str]) -> std::io::Error {
            self.hear(&format!("exec {}", argv.join(" ")));
            std::io::Error::other("the test stands in for exec")
        }
    }

    fn step_name(step: Option<&Step>) -> String {
        match step {
            None => "none".into(),
            Some(Step::Downloading { .. }) => "Downloading".into(),
            Some(Step::Verifying) => "Verifying".into(),
            Some(Step::Asking) => "Asking".into(),
            Some(Step::Installing { .. }) => "Installing".into(),
            Some(Step::Restarting) => "Restarting".into(),
            Some(Step::Failed(failure)) => format!("Failed at {:?}", failure.stage),
        }
    }

    /// 0.5.0 offered over the installed 0.4.0, its package at `url`.
    fn offer(url: &str) -> Offer {
        let sha256 = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, BYTES).unwrap();
        let package = Package {
            file: FILE.into(),
            url: url.into(),
            size: u64::try_from(BYTES.len()).unwrap(),
            sha256: sha256.to_string(),
        };
        let pair = Pair {
            deb: package.clone(),
            rpm: package,
        };
        let release = Release {
            version: Version::parse("0.5.0").unwrap(),
            published_at: 0,
            critical: false,
            minimum_installed_version: None,
            engine: Engine {
                version: "0.17.0".into(),
                tag: "v0.17.0".into(),
                source_commit: "0".repeat(40),
            },
            notes: vec!["Fermix installs its own updates.".into()],
            packages: Packages {
                amd64: pair.clone(),
                arm64: pair,
            },
        };
        Offer {
            release,
            critical: false,
            held_back: None,
            skipped: false,
        }
    }

    fn planted(plant: Plant, bus: gio::DBusConnection, downloads: PathBuf, url: &str) -> Stand {
        let mut updates = Updates::new(Remembered::default());
        updates.installation = Some(Installation::Package {
            version: Version::parse("0.4.0").unwrap(),
            format: Some(Format::Deb),
        });
        updates.arch = Some(Arch::Amd64);
        updates.found = Some(Outcome::Available(Box::new(offer(url))));
        Stand {
            updates: RefCell::new(updates),
            downloads: Downloads::new(downloads),
            bus,
            tamper: plant.tamper,
            version: plant.version,
            engine: RefCell::new(plant.engine),
            heard: RefCell::default(),
            drawn: RefCell::default(),
        }
    }

    /// Serves `BYTES` to one request, on a port of its own. Returns the package's URL.
    fn serve_package() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Some(Ok(stream)) = listener.incoming().next() {
                answer(stream);
            }
        });
        format!("http://127.0.0.1:{port}/{FILE}")
    }

    fn answer(mut stream: TcpStream) {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && head.len() < HEAD_CAP {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                return;
            }
            head.push(byte[0]);
        }
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            BYTES.len()
        );
        // The client may hang up first: that is its answer.
        let _ = stream
            .write_all(reply.as_bytes())
            .and_then(|()| stream.write_all(BYTES));
    }

    /// A folder of the test's own, removed when dropped.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// What one run left: the seams reached, the steps drawn, what PackageKit
    /// was asked, the step it ended on, and whether the package is still there.
    struct Ran {
        heard: Vec<String>,
        drawn: Vec<String>,
        seen: Seen,
        step: Option<Step>,
        file: PathBuf,
        kept: bool,
    }

    /// Runs `flow` on a stand planted with `plant`, on a main context and a
    /// bus of its own.
    fn run<F, Fut>(plant: Plant, flow: F) -> Ran
    where
        F: FnOnce(Rc<Stand>) -> Fut,
        Fut: Future<Output = ()>,
    {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| context.block_on(run_on_bus(plant, flow)))
            .unwrap()
    }

    async fn run_on_bus<F, Fut>(mut plant: Plant, flow: F) -> Ran
    where
        F: FnOnce(Rc<Stand>) -> Fut,
        Fut: Future<Output = ()>,
    {
        let bus = PrivateBus::start();
        let (client, service) = (bus.connect().await, bus.connect().await);
        let seen = fake(&service, std::mem::take(&mut plant.answers));
        let n = RUNS.fetch_add(1, Ordering::SeqCst);
        let scratch = Scratch(
            std::env::temp_dir().join(format!("fermix-install-{}-{n}", std::process::id())),
        );
        let downloads = scratch.0.join("updates");
        let stand = Rc::new(planted(plant, client, downloads, &serve_package()));
        glib::future_with_timeout(TEST_CAP, flow(stand.clone()))
            .await
            .expect("the install ends within the test's cap");
        let file = stand.downloads.dir.join(FILE);
        let step = stand.updates.borrow().step.clone();
        Ran {
            heard: stand.heard.take(),
            drawn: stand.drawn.take(),
            seen: seen.take(),
            step,
            kept: file.exists(),
            file,
        }
    }

    fn failure(ran: &Ran) -> &Failure {
        match &ran.step {
            Some(Step::Failed(failure)) => failure,
            other => panic!("the install ended on {other:?}, not a failure"),
        }
    }

    #[test]
    fn a_confirmed_install_goes_on_to_the_engine_s_restart_and_exec() {
        let ran = run(plant(), install_update);
        let file = ran.file.to_string_lossy().into_owned();
        assert_eq!(ran.seen.installs, [(ONLY_TRUSTED, vec![file])]);
        assert_eq!(
            ran.heard,
            [
                "PackageKit",
                "package database",
                "systemd reload",
                "engine restart",
                EXEC
            ]
        );
        assert_eq!(ran.drawn.first().map(String::as_str), Some("Downloading"));
        assert!(
            ran.drawn.ends_with(
                &[
                    "Verifying",
                    "Asking",
                    "Installing",
                    "Restarting",
                    "Failed at Restart"
                ]
                .map(String::from)
            ),
            "drawn: {:?}",
            ran.drawn
        );
        assert_eq!(
            failure(&ran).sentence,
            "Fermix could not open again: the test stands in for exec."
        );
        assert!(!ran.kept, "the installed package is deleted");
    }

    #[test]
    fn a_version_the_system_does_not_confirm_keeps_the_package_for_the_command() {
        let plant = Plant {
            version: "0.4.0\n",
            ..plant()
        };
        let ran = run(plant, install_update);
        assert_eq!(ran.heard, ["PackageKit", "package database"]);
        let failure = failure(&ran);
        assert_eq!(failure.stage, Stage::Install);
        assert_eq!(
            failure.sentence,
            "The system installed it, but reports Fermix 0.4.0, not 0.5.0."
        );
        assert_eq!(
            failure.manual,
            Manual::Command {
                command: install_command(Format::Deb, &ran.file),
                file: ran.file.clone(),
            }
        );
        assert!(ran.kept, "the package stays for the command");
    }

    #[test]
    fn a_desktop_only_update_reloads_nothing_and_restarts_only_the_window() {
        let plant = Plant {
            engine: Alignment::Aligned,
            ..plant()
        };
        let ran = run(plant, install_update);
        assert_eq!(ran.heard, ["PackageKit", "package database", EXEC]);
    }

    #[test]
    fn an_unsigned_package_is_checked_again_and_installed_on_the_untrusted_pass() {
        let unsigned = Answer::Fail(error::MISSING_GPG_SIGNATURE, "package is not signed");
        let plant = Plant {
            answers: vec![unsigned, Answer::Succeed],
            ..plant()
        };
        let ran = run(plant, install_update);
        let file = ran.file.to_string_lossy().into_owned();
        assert_eq!(
            ran.seen.installs,
            [(ONLY_TRUSTED, vec![file.clone()]), (UNTRUSTED, vec![file])],
            "only-trusted first, then untrusted, nothing more"
        );
        assert_eq!(
            ran.heard[..3],
            ["PackageKit", "PackageKit", "package database"]
        );
        let tail = [
            "Verifying",
            "Asking",
            "Installing",
            "Verifying",
            "Asking",
            "Installing",
            "Restarting",
            "Failed at Restart",
        ];
        assert!(
            ran.drawn.ends_with(&tail.map(String::from)),
            "drawn: {:?}",
            ran.drawn
        );
    }

    #[test]
    fn a_package_that_changed_during_the_trusted_pass_is_not_handed_to_the_untrusted_one() {
        let unsigned = Answer::Fail(error::MISSING_GPG_SIGNATURE, "package is not signed");
        let plant = Plant {
            answers: vec![unsigned, Answer::Succeed],
            tamper: true,
            ..plant()
        };
        let ran = run(plant, install_update);
        assert_eq!(ran.seen.installs.len(), 1, "only the pass before the swap");
        assert_eq!(ran.heard, ["PackageKit"]);
        let failure = failure(&ran);
        assert_eq!(failure.stage, Stage::Download);
        assert_eq!(
            failure.sentence,
            "The download was not the file the update names, so it was deleted."
        );
        assert!(!ran.kept, "the changed package is deleted");
    }

    #[test]
    fn try_again_checks_the_package_it_reuses_before_handing_it_over() {
        let conflict = Answer::Fail(35, "file /usr/bin/fermix conflicts with fermix 0.15.0");
        let plant = Plant {
            answers: vec![conflict, Answer::Succeed],
            ..plant()
        };
        let ran = run(plant, |stand: Rc<Stand>| async move {
            install_update(stand.clone()).await;
            assert!(
                matches!(&stand.updates.borrow().step, Some(Step::Failed(f)) if f.stage == Stage::Install)
            );
            let file = stand.downloads.dir.join(FILE);
            std::fs::write(file, b"test package swapped before Try Again\n").unwrap();
            update_try_again(stand).await;
        });
        assert_eq!(
            ran.seen.installs.len(),
            1,
            "the swapped package is never handed over"
        );
        assert_eq!(failure(&ran).stage, Stage::Download);
        assert!(!ran.kept, "the changed package is deleted");
    }
}
