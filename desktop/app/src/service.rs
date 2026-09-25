//! The daemon's own lifecycle: restart it, start it when it is stopped, wait for
//! it to answer again, and open its setup page for panes this app has not built.

use crate::app::App;
use crate::dialogs::confirm;
use crate::systemd::{systemd, UnitVerb};
use fermix_client::management::CallError;
use gtk::glib;
use std::rc::Rc;
use std::time::Duration;

/// Waiting for the daemon after a start or restart: every second, for at most
/// 90 s, the CLI's own ceiling (M38 §4.1).
const WAKE_INTERVAL: Duration = Duration::from_secs(1);
const WAKE_TRIES: u32 = 90;

impl App {
    /// "Restart Fermix…" (M38 §6.4): take the daemon's drain lease, then have
    /// systemd restart the unit. If systemd refuses, the lease goes back and the
    /// old daemon carries on. The lease is never committed: the old process
    /// takes it with it when it exits.
    pub async fn restart(self: Rc<Self>) {
        let reasons = self.restart_reasons();
        if !confirm(
            &self.shell.window,
            "Restart Fermix?",
            &reasons,
            "Restart",
            false,
        )
        .await
        {
            return;
        }
        if let Err(sentence) = self.restart_now().await {
            self.shell.toast(&sentence);
        }
    }

    /// The restart transaction itself, with no question asked. `Err` carries
    /// the sentence to show; `Ok` means a new daemon process answered.
    pub async fn restart_now(&self) -> Result<(), String> {
        let old_pid = self.state.borrow().snapshot().and_then(|s| s.pid.clone());
        let lease = match self.daemon.call(|m| m.prepare_restart()).await {
            Ok(lease) => lease,
            Err(CallError::Refused(refusal)) => return Err(refusal.sentence),
            Err(e) => {
                glib::g_warning!("fermix", "restart was not prepared: {e:?}");
                self.refresh().await;
                return Err("Fermix did not answer, so it was not restarted.".into());
            }
        };
        self.set_waking(true, false);
        self.render();
        if let Err(sentence) = systemd(UnitVerb::Restart).await {
            glib::g_warning!("fermix", "systemd refused the restart: {sentence}");
            let id = lease.lease_id.clone();
            if let Err(e) = self.daemon.call(move |m| m.cancel_restart(&id)).await {
                glib::g_warning!("fermix", "the drain lease was not handed back: {e:?}");
            }
            self.set_waking(false, false);
            self.refresh().await;
            return Err(format!("Fermix did not restart: {sentence}"));
        }
        if self.wait_for_daemon(old_pid, "Fermix restarted").await {
            Ok(())
        } else {
            Err("Fermix did not come back after the restart.".into())
        }
    }

    fn restart_reasons(&self) -> String {
        let state = self.state.borrow();
        let reasons: Vec<String> = state
            .snapshot()
            .map(|s| {
                s.state
                    .restart
                    .reasons
                    .iter()
                    .map(|r| r.sentence.clone())
                    .collect()
            })
            .unwrap_or_default();
        if reasons.is_empty() {
            "Fermix stops and starts again. Anything it is doing right now is interrupted.".into()
        } else {
            reasons.join("\n")
        }
    }

    pub async fn start_service(self: Rc<Self>) {
        self.ask_systemd(UnitVerb::Start, "Fermix started").await;
    }

    pub async fn restart_service(self: Rc<Self>) {
        self.ask_systemd(UnitVerb::Restart, "Fermix restarted")
            .await;
    }

    async fn ask_systemd(&self, verb: UnitVerb, done: &str) {
        let old_pid = self.state.borrow().snapshot().and_then(|s| s.pid.clone());
        self.state.borrow_mut().waking = true;
        self.render();
        if let Err(sentence) = systemd(verb).await {
            glib::g_warning!("fermix", "systemd refused {verb:?}: {sentence}");
            self.set_waking(false, true);
            self.shell
                .toast(&format!("Fermix did not start: {sentence}"));
            self.refresh().await;
            return;
        }
        self.wait_for_daemon(old_pid, done).await;
    }

    /// Re-reads the daemon until a process other than `old_pid` answers, or the
    /// tries run out. A restart that has not stopped the old process yet must
    /// not count as back.
    pub async fn wait_for_daemon(&self, old_pid: Option<String>, done: &str) -> bool {
        self.set_waking(true, false);
        self.render();
        for _ in 0..WAKE_TRIES {
            glib::timeout_future(WAKE_INTERVAL).await;
            self.refresh().await;
            let pid = self.state.borrow().snapshot().map(|s| s.pid.clone());
            if matches!(pid, Some(ref new) if old_pid.is_none() || *new != old_pid) {
                self.state.borrow_mut().waking = false;
                self.render();
                self.shell.toast(done);
                return true;
            }
        }
        self.set_waking(false, true);
        self.render();
        self.shell.toast("Fermix did not come back");
        false
    }

    fn set_waking(&self, waking: bool, wake_failed: bool) {
        let mut state = self.state.borrow_mut();
        state.waking = waking;
        state.wake_failed = wake_failed;
    }

    pub async fn open_setup_page(self: Rc<Self>) {
        let url = match self.daemon.call(|m| m.setup_session()).await {
            Ok(session) => session.url,
            Err(CallError::Refused(refusal)) => return self.shell.toast(&refusal.sentence),
            Err(e) => {
                glib::g_warning!("fermix", "no setup session: {e:?}");
                return self.refresh().await;
            }
        };
        let launched = gtk::UriLauncher::new(&url)
            .launch_future(Some(&self.shell.window))
            .await;
        if let Err(e) = launched {
            glib::g_warning!("fermix", "the setup page did not open: {e}");
            self.shell
                .toast("Your browser did not open the setup page.");
        }
    }
}
