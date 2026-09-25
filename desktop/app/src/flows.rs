//! Provider flows: browser sign-in, importing a login, pasting a key or token,
//! signing out and choosing the primary (design_final §3). Each one shows its
//! progress on the provider's own row and ends by re-reading the daemon.

use crate::app::App;
use crate::dialogs::{confirm, secret_dialog, SecretPrompt};
use crate::state::Link;
use fermix_client::job::{adoptable_sign_in, outcome, poll_cap, Outcome};
use fermix_client::management::CallError;
use fermix_client::model::JobView;
use fermix_client::providers::{Door, ImportSource};
use fermix_client::view::{daemon_problem, Activity, DaemonProblem, Recent};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// One job poll per second: the `verifying` step is too brief to see at two.
const POLL_MS: u64 = 1_000;

fn job(job_id: &str, door: Door, phase: Option<String>, has_link: bool) -> Activity {
    Activity::Job {
        job_id: job_id.to_owned(),
        door,
        phase,
        has_link,
        browser_failed: false,
    }
}

impl App {
    pub async fn open_door(self: Rc<Self>, target: &str) {
        let Some((provider, door)) = Door::parse_target(target) else {
            glib::g_warning!("fermix", "unknown way in: {target}");
            return;
        };
        // Home's attention buttons start a way in too; its progress shows on the row.
        self.show_page("providers");
        match door {
            Door::BrowserSignIn => self.browser_sign_in(provider).await,
            Door::Import(source) => self.import_login(provider, source).await,
            Door::SetupToken => self.ask_secret(provider, door),
            Door::ApiKey => self.ask_secret(provider, door),
        }
    }

    async fn browser_sign_in(self: Rc<Self>, provider: String) {
        self.set_activity(
            &provider,
            job("", Door::BrowserSignIn, Some("binding".into()), false),
        );
        let p = provider.clone();
        let start = match self.daemon.call(move |m| m.auth_start(&p)).await {
            Ok(start) => start,
            Err(CallError::Refused(r)) if r.code == "busy" => {
                return self.adopt_sign_in(&provider, r.sentence).await
            }
            Err(e) => return self.failed(&provider, e).await,
        };
        let Some(url) = start.authorize_url.clone() else {
            let sentence = "Fermix did not hand out a sign-in link.".to_owned();
            return self.finish(&provider, Activity::Failed(sentence)).await;
        };
        let lifetime = Duration::from_millis(start.expires_in_ms.unwrap_or(start.job.budget_ms));
        let link = Link {
            url: url.clone(),
            expires: Instant::now() + lifetime,
        };
        self.state.borrow_mut().links.insert(provider.clone(), link);
        self.set_activity(
            &provider,
            job(
                &start.job.job_id,
                Door::BrowserSignIn,
                start.job.phase.clone(),
                true,
            ),
        );
        let opener = self.clone();
        let (p, job_id) = (provider.clone(), start.job.job_id.clone());
        glib::spawn_future_local(async move { opener.open_browser(&p, &job_id, &url).await });
        self.follow(&provider, Door::BrowserSignIn, start.job).await;
    }

    /// A sign-in for this provider is already running, most likely started before
    /// the app was. Follow it so the row can show it and cancel it; its link was
    /// handed out once and is gone, so the row offers no Copy link.
    async fn adopt_sign_in(self: &Rc<Self>, provider: &str, busy: String) {
        let answer = self.daemon.call(|m| m.job_list()).await;
        let jobs = match answer {
            Ok(list) => list.jobs,
            Err(e) => return self.failed(provider, e).await,
        };
        let followed = self.state.borrow().followed_jobs();
        let Some(running) = adoptable_sign_in(&jobs, &followed) else {
            return self.finish(provider, Activity::Failed(busy)).await;
        };
        let phase = running.phase.clone();
        self.set_activity(
            provider,
            job(&running.job_id, Door::BrowserSignIn, phase, false),
        );
        self.follow(provider, Door::BrowserSignIn, running).await;
    }

    /// Opens the authorize url through the desktop's browser (the OpenURI portal
    /// inside Flatpak). If it does not open, the row says so and keeps Copy link.
    async fn open_browser(&self, provider: &str, job_id: &str, url: &str) {
        let launched = gtk::UriLauncher::new(url)
            .launch_future(Some(&self.shell.window))
            .await;
        let Err(e) = launched else { return };
        glib::g_warning!("fermix", "the browser did not open for {provider}: {e}");
        let mut state = self.state.borrow_mut();
        if let Some(Activity::Job {
            job_id: running,
            browser_failed,
            ..
        }) = state.activity.get_mut(provider)
        {
            *browser_failed = running == job_id;
        }
        drop(state);
        self.render();
    }

    async fn import_login(self: Rc<Self>, provider: String, source: ImportSource) {
        self.set_activity(&provider, job("", Door::Import(source), None, false));
        match self.daemon.call(move |m| m.auth_import(source)).await {
            Ok(view) => {
                self.set_activity(
                    &provider,
                    job(
                        &view.job_id,
                        Door::Import(source),
                        view.phase.clone(),
                        false,
                    ),
                );
                self.follow(&provider, Door::Import(source), view).await;
            }
            Err(e) => self.failed(&provider, e).await,
        }
    }

    /// Polls one job until it ends, the user cancels it, or its budget runs out.
    /// Before and after every poll it checks the row still follows this job: a
    /// Cancel may have ended it while the poll was on the wire.
    async fn follow(self: &Rc<Self>, provider: &str, door: Door, started: JobView) {
        for _ in 0..poll_cap(started.budget_ms, POLL_MS) {
            glib::timeout_future(Duration::from_millis(POLL_MS)).await;
            if !self.follows(provider, &started.job_id) {
                return;
            }
            let id = started.job_id.clone();
            let answer = self.daemon.call(move |m| m.job_get(&id)).await;
            if !self.follows(provider, &started.job_id) {
                return;
            }
            let view = match answer {
                Ok(view) => view,
                Err(e) => return self.failed(provider, e).await,
            };
            if let Outcome::Running(phase) = outcome(&view) {
                self.set_phase(provider, phase);
                continue;
            }
            return self.ended(provider, door, &view).await;
        }
        let sentence = "The sign-in did not finish in time.".to_owned();
        self.finish(provider, Activity::Failed(sentence)).await;
    }

    fn follows(&self, provider: &str, job_id: &str) -> bool {
        self.state.borrow().job_id(provider).as_deref() == Some(job_id)
    }

    /// Shows how a job ended on its row. A job still running is left to `follow`.
    async fn ended(self: &Rc<Self>, provider: &str, door: Door, view: &JobView) {
        match outcome(view) {
            Outcome::Running(_) => {}
            Outcome::Completed => self.signed_in(provider, door).await,
            Outcome::Failed(sentence) => self.finish(provider, Activity::Failed(sentence)).await,
            Outcome::Cancelled => self.finish(provider, Activity::Cancelled).await,
        }
    }

    fn set_phase(&self, provider: &str, phase: Option<String>) {
        if let Some(Activity::Job { phase: current, .. }) =
            self.state.borrow_mut().activity.get_mut(provider)
        {
            *current = phase;
        }
        self.render();
    }

    async fn signed_in(self: &Rc<Self>, provider: &str, door: Door) {
        let was_ready = self.is_ready();
        self.clear_activity(provider);
        let recent = match door {
            Door::Import(source) => Recent::Imported(source),
            _ => Recent::SignedIn,
        };
        self.mark_recent(provider, recent);
        self.refresh().await;
        let label = self.state.borrow().label(provider);
        self.announce(&format!("Signed in to {label}"), was_ready);
    }

    /// A toast for a change that landed; offers Home when Fermix just became ready.
    /// It stays longer than an ordinary toast: the user may still be in the browser.
    fn announce(&self, text: &str, was_ready: bool) {
        let toast = adw::Toast::new(text);
        toast.set_timeout(10);
        toast.set_priority(adw::ToastPriority::High);
        if !was_ready && self.is_ready() {
            toast.set_button_label(Some("Open Home"));
            toast.set_action_name(Some("win.page"));
            toast.set_action_target_value(Some(&"home".to_variant()));
        }
        self.shell.toasts.add_toast(toast);
    }

    pub async fn finish(self: &Rc<Self>, provider: &str, ended: Activity) {
        self.state.borrow_mut().links.remove(provider);
        self.settle(provider, ended);
        self.refresh().await;
    }

    /// A refusal is shown on the row in the daemon's words. Losing the daemon, or
    /// a daemon this app is too old or too new for, flips the whole window instead.
    pub async fn failed(self: &Rc<Self>, provider: &str, e: CallError) {
        let problem = daemon_problem(&e);
        match e {
            CallError::Refused(refusal) if !matches!(problem, DaemonProblem::UpdateNeeded(_)) => {
                self.finish(provider, Activity::Failed(refusal.sentence))
                    .await
            }
            other => {
                glib::g_warning!(
                    "fermix",
                    "lost the daemon during a {provider} action: {other:?}"
                );
                self.clear_activity(provider);
                self.shell.toast(&problem.sentence());
                self.refresh().await;
            }
        }
    }

    /// Cancels the row's job and shows what the daemon says became of it: a
    /// sign-in can finish in the moment before the cancel lands.
    pub async fn cancel(self: &Rc<Self>, provider: &str) {
        let followed = self.state.borrow().activity(provider);
        let Activity::Job { job_id, door, .. } = followed else {
            return;
        };
        if job_id.is_empty() {
            return;
        }
        match self.daemon.call(move |m| m.job_cancel(&job_id)).await {
            Ok(view) => self.ended(provider, door, &view).await,
            Err(e) => self.failed(provider, e).await,
        }
    }

    pub fn copy_link(&self, provider: &str) {
        let state = self.state.borrow();
        match state.links.get(provider) {
            Some(link) if link.expires > Instant::now() => {
                self.shell.window.clipboard().set_text(&link.url);
                self.shell.toast("Link copied");
            }
            _ => self
                .shell
                .toast("The link has expired. Start the sign-in again."),
        }
    }

    fn ask_secret(self: Rc<Self>, provider: String, door: Door) {
        let label = self.state.borrow().label(&provider);
        let (title, description, entry_title, id) = if door == Door::SetupToken {
            (
                "Anthropic setup token".to_owned(),
                "Run claude setup-token in a terminal and paste what it prints.",
                "Setup token",
                Door::ANTHROPIC_SETUP_TOKEN_ID.to_owned(),
            )
        } else {
            (
                format!("{label} API key"),
                "Stored in your keyring. Fermix never shows it again.",
                "API key",
                Door::api_key_secret_id(&provider),
            )
        };
        let prompt = SecretPrompt {
            title: &title,
            description,
            entry_title,
        };
        let app = self.clone();
        secret_dialog(&self.shell.window, prompt, move |value| {
            app.clone()
                .save_secret(provider.clone(), id.clone(), door, value)
        });
    }

    async fn save_secret(
        self: Rc<Self>,
        provider: String,
        id: String,
        door: Door,
        value: String,
    ) -> Result<(), String> {
        let was_ready = self.is_ready();
        self.set_activity(&provider, Activity::Saving);
        let answer = self.daemon.call(move |m| m.secret_set(&id, &value)).await;
        self.clear_activity(&provider);
        if let Err(e) = answer {
            self.refresh().await;
            return Err(match e {
                CallError::Refused(refusal) => refusal.sentence,
                other => daemon_problem(&other).sentence(),
            });
        }
        let (recent, verb) = match door {
            Door::SetupToken => (Recent::SignedIn, "Signed in to"),
            _ => (Recent::KeyAdded, "Key added for"),
        };
        self.mark_recent(&provider, recent);
        self.refresh().await;
        let label = self.state.borrow().label(&provider);
        self.announce(&format!("{verb} {label}"), was_ready);
        Ok(())
    }

    pub async fn sign_out(self: Rc<Self>, provider: &str) {
        let label = self.state.borrow().label(provider);
        let body =
            "Fermix forgets this sign-in on this computer. Nothing is revoked at the provider.";
        if !confirm(
            &self.shell.window,
            &format!("Sign out of {label}?"),
            body,
            "Sign out",
            true,
        )
        .await
        {
            return;
        }
        self.set_activity(provider, Activity::SigningOut);
        let p = provider.to_owned();
        match self.daemon.call(move |m| m.auth_logout(&p)).await {
            Ok(_) => self.done(provider, &format!("Signed out of {label}")).await,
            Err(e) => self.failed(provider, e).await,
        }
    }

    pub async fn remove_key(self: Rc<Self>, provider: &str) {
        let label = self.state.borrow().label(provider);
        let body = "Fermix deletes the key from your keyring. You can add it again at any time.";
        if !confirm(
            &self.shell.window,
            &format!("Remove the {label} key?"),
            body,
            "Remove",
            true,
        )
        .await
        {
            return;
        }
        self.set_activity(provider, Activity::RemovingKey);
        let id = Door::api_key_secret_id(provider);
        match self.daemon.call(move |m| m.secret_clear(&id)).await {
            Ok(_) => {
                self.done(provider, &format!("Key removed for {label}"))
                    .await
            }
            Err(e) => self.failed(provider, e).await,
        }
    }

    pub async fn make_primary(self: Rc<Self>, provider: &str) {
        let label = self.state.borrow().label(provider);
        self.set_activity(provider, Activity::Switching);
        let p = provider.to_owned();
        match self.daemon.call(move |m| m.set_primary(&p)).await {
            Ok(_) => {
                self.done(provider, &format!("Fermix now answers with {label}"))
                    .await
            }
            Err(e) => self.failed(provider, e).await,
        }
    }

    async fn done(self: &Rc<Self>, provider: &str, toast: &str) {
        self.clear_activity(provider);
        self.refresh().await;
        self.shell.toast(toast);
    }
}
