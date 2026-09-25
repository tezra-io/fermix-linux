//! Integrations: one flat list of plugins under four counted filters, with
//! search, a detail dialog per plugin, and the sign-in clients at the foot
//! (M38 §5.7). Every sentence and button word is the daemon's, and buttons
//! route on its action ids, never on their words. The pane owns its data and
//! its daemon calls: it reads `plugins.list` when shown, reads it again after
//! every write and every job, and re-attaches to plugin jobs that outlived it.

use crate::daemon::Daemon;
use crate::dialogs::{confirm, secret_dialog, SecretPrompt};
use crate::marks::{mark, Kind};
use adw::prelude::*;
use fermix_client::job::{outcome, poll_cap, Outcome};
use fermix_client::management::CallError;
use fermix_client::model::{Features, JobView};
use fermix_client::plugins::{
    client_answer, client_for, client_secret_id, client_state, client_title, consent_body, count,
    feature_state, job_words, line, needs_operator, plugin_secret_id, reattachable,
    sign_in_provider, verbs, visible, visible_features, AccessProfile, Filter, OAuthClient,
    PluginAction, PluginList, PluginRow, PluginSetting, Region, SettingKind, Verb, Workspace,
    FEATURES,
};
use fermix_client::settings::UNKNOWN_KIND;
use fermix_client::view::daemon_problem;
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Jobs are polled twice a second, within the cap their own budget sets.
const POLL_MS: u64 = 500;
const DIALOG_WIDTH: i32 = 480;
const DIALOG_HEIGHT: i32 = 620;

const INTRO: &str = "Plugins, MCP servers and the features Fermix can use.";
const READING: &str = "Reading from Fermix…";
const NO_MATCH: &str = "Nothing matches your search.";
const NOTHING: &str = "Nothing to show here.";
const GONE: &str = "This plugin is no longer in the list.";
const STOPPED: &str = "Stopped before it finished.";
const OVERDUE: &str = "This took too long, so Fermix stopped waiting.";
const NO_LINK: &str = "Fermix did not hand out a sign-in link.";
const NO_BROWSER: &str = "The browser did not open. Copy the link and open it yourself.";
const NO_CLIENT: &str = "This plugin has no sign-in client to set up.";
const NOT_CHOSEN: &str = "Not chosen";
const WRITE_WARNING: &str = "This access level can change data in the workspace.";
const NO_WORKSPACES: &str = "No workspaces have been found yet.";
const PICK_WORKSPACE: &str = "Find workspaces, then choose one.";
const CLIENTS_NOTE: &str = "The app registrations plugins sign in through.";
const CLIENT_NOTE: &str = "Store the client secret first, then the client ID. Leave the port \
    blank to use the default.";
const REGION_NOTE: &str =
    "Pick the region the account belongs to. Its sign-in goes to that region's host.";
const REGION_PROMPT: &str = "Choose a region";
const KEYRING_NOTE: &str = "Stored in your keyring. Fermix never shows it again.";

/// A job the pane is following.
#[derive(Debug, Clone, PartialEq)]
struct Running {
    job_id: String,
    /// The plugin, when this pane started the job. Jobs do not name their
    /// plugin, so one re-attached after the pane was gone is anonymous.
    name: Option<String>,
    kind: String,
    /// A sign-in's link, handed out once, kept for Copy link while it runs.
    link: Option<String>,
}

/// What the pane last read from the daemon, and what it is doing now.
#[derive(Default)]
struct Data {
    /// `None` until the first read; `Err` holds why the last read failed.
    list: Option<Result<PluginList, String>>,
    /// `None` until read, or when the last read failed: unread is not Off.
    features: Option<Features>,
    /// The settings file changed outside Fermix or cannot be read: nothing may be written.
    locked: bool,
    jobs: Vec<Running>,
    /// The last refusal or failed job per plugin, shown until its next action.
    errors: HashMap<String, String>,
}

impl Data {
    fn list(&self) -> Option<&PluginList> {
        self.list.as_ref()?.as_ref().ok()
    }

    fn row(&self, name: &str) -> Option<PluginRow> {
        self.list()?
            .plugins
            .iter()
            .find(|p| p.name == name)
            .cloned()
    }

    fn running(&self, name: &str) -> Option<Running> {
        self.jobs
            .iter()
            .find(|j| j.name.as_deref() == Some(name))
            .cloned()
    }
}

/// One plugin line as drawn; the list is rebuilt only when a line changes.
#[derive(Debug, Clone, PartialEq)]
struct Line {
    name: String,
    title: String,
    text: String,
    error: Option<String>,
    enabled: bool,
    busy: bool,
    locked: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Body {
    Note(String, bool),
    Plugins(Vec<Line>),
    /// Feature ids with their state words.
    Features(Vec<(&'static str, &'static str)>),
}

#[derive(Debug, Clone, PartialEq)]
struct ClientLine {
    provider: String,
    title: String,
    state: String,
    configured: bool,
    locked: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    counts: Vec<usize>,
    body: Body,
    clients: Vec<ClientLine>,
    /// What anonymous re-attached jobs are doing.
    elsewhere: Vec<&'static str>,
}

/// A plugin's detail: one dialog whose pages are the plugin, and its workspace
/// or sign-in client when those are pushed. The first page is rebuilt when
/// what it shows changes.
struct Detail {
    name: String,
    dialog: adw::Dialog,
    nav: adw::NavigationView,
    holder: adw::Bin,
    shown: RefCell<Option<DetailView>>,
}

#[derive(Debug, Clone, PartialEq)]
struct DetailView {
    row: Option<PluginRow>,
    client: Option<(OAuthClient, String)>,
    running: Option<Running>,
    error: Option<String>,
    locked: bool,
}

pub struct IntegrationsPane {
    pub page: adw::PreferencesPage,
    daemon: Daemon,
    toast: Rc<dyn Fn(&str)>,
    filters: adw::ToggleGroup,
    search: gtk::SearchEntry,
    list: adw::PreferencesGroup,
    clients: adw::PreferencesGroup,
    data: RefCell<Data>,
    drawn: RefCell<Option<Drawn>>,
    list_rows: RefCell<Vec<gtk::Widget>>,
    client_rows: RefCell<Vec<gtk::Widget>>,
    detail: RefCell<Option<Rc<Detail>>>,
}

impl IntegrationsPane {
    /// `toast` shows one short line in the window, for what finished off screen.
    pub fn new(daemon: Daemon, toast: Rc<dyn Fn(&str)>) -> Rc<IntegrationsPane> {
        let filters = filter_toggles();
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search integrations")
            .build();
        let head = adw::PreferencesGroup::builder().description(INTRO).build();
        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .build();
        controls.append(&filters);
        controls.append(&search);
        head.add(&controls);
        let list = adw::PreferencesGroup::new();
        let clients = adw::PreferencesGroup::builder()
            .title("Sign-in clients")
            .description(CLIENTS_NOTE)
            .visible(false)
            .build();
        let page = adw::PreferencesPage::new();
        page.add(&head);
        page.add(&list);
        page.add(&clients);
        let pane = Rc::new(IntegrationsPane {
            page,
            daemon,
            toast,
            filters,
            search,
            list,
            clients,
            data: RefCell::default(),
            drawn: RefCell::default(),
            list_rows: RefCell::default(),
            client_rows: RefCell::default(),
            detail: RefCell::default(),
        });
        pane.wire_controls();
        pane.render();
        pane
    }

    /// Call whenever the pane comes into view: reads the list, the feature
    /// flags and the running jobs, and follows any plugin job not yet followed.
    pub fn shown(self: &Rc<Self>) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            pane.refresh().await;
            pane.reattach().await;
        });
    }

    fn wire_controls(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.filters.connect_active_name_notify(move |_| {
            if let Some(pane) = weak.upgrade() {
                pane.render();
            }
        });
        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(pane) = weak.upgrade() {
                pane.render();
            }
        });
    }

    fn filter(&self) -> Filter {
        self.filters
            .active_name()
            .and_then(|slug| Filter::from_slug(&slug))
            .unwrap_or(Filter::Installed)
    }

    // ---- reading ----

    /// Reads the plugins and the setup state, then redraws.
    async fn refresh(self: &Rc<Self>) {
        let list = self.daemon.call(|m| m.plugins_list()).await;
        let state = self.daemon.call(|m| m.setup_state()).await;
        let mut data = self.data.borrow_mut();
        data.list = Some(list.map_err(|e| sentence_for(e, "plugins.list")));
        match state {
            Ok(state) => {
                data.features = Some(state.features);
                data.locked = state.coexistence.config_state != "clear";
            }
            Err(e) => {
                glib::g_warning!(
                    "fermix",
                    "integrations could not read the setup state: {e:?}"
                );
                data.features = None;
            }
        }
        drop(data);
        self.render();
    }

    /// Follows plugin jobs that are running but not followed: started before
    /// the app was, or left when a poll lost the daemon. Each re-reads the list when it ends.
    async fn reattach(self: &Rc<Self>) {
        let jobs = match self.daemon.call(|m| m.job_list()).await {
            Ok(list) => list.jobs,
            Err(e) => {
                glib::g_warning!("fermix", "integrations could not list running jobs: {e:?}");
                return;
            }
        };
        let followed: Vec<String> = self
            .data
            .borrow()
            .jobs
            .iter()
            .map(|j| j.job_id.clone())
            .collect();
        for job in reattachable(&jobs, &followed) {
            let pane = self.clone();
            glib::spawn_future_local(async move {
                if let Some(sentence) = failure_sentence(pane.follow(None, job, None).await) {
                    (pane.toast)(&sentence);
                }
                pane.refresh().await;
            });
        }
    }

    /// Polls one job until it ends or its budget runs out, showing it as running meanwhile.
    async fn follow(
        self: &Rc<Self>,
        name: Option<&str>,
        started: JobView,
        link: Option<String>,
    ) -> Outcome {
        let running = Running {
            job_id: started.job_id.clone(),
            name: name.map(str::to_owned),
            kind: started.kind.clone(),
            link,
        };
        self.data.borrow_mut().jobs.push(running);
        self.render();
        let ended = self.poll(&started).await;
        self.data
            .borrow_mut()
            .jobs
            .retain(|j| j.job_id != started.job_id);
        ended
    }

    async fn poll(&self, started: &JobView) -> Outcome {
        let first = outcome(started);
        if !matches!(first, Outcome::Running(_)) {
            return first;
        }
        for _ in 0..poll_cap(started.budget_ms, POLL_MS) {
            glib::timeout_future(Duration::from_millis(POLL_MS)).await;
            let id = started.job_id.clone();
            let polled = match self.daemon.call(move |m| m.job_get(&id)).await {
                Ok(view) => outcome(&view),
                Err(e) => Outcome::Failed(sentence_for(e, "job.get")),
            };
            if !matches!(polled, Outcome::Running(_)) {
                return polled;
            }
        }
        Outcome::Failed(OVERDUE.to_owned())
    }

    // ---- drawing the list ----

    fn render(self: &Rc<Self>) {
        self.render_detail();
        let drawn = self.drawn_now();
        if self.drawn.borrow().as_ref() == Some(&drawn) {
            return;
        }
        for (filter, n) in Filter::ALL.iter().zip(&drawn.counts) {
            if let Some(toggle) = self.filters.toggle_by_name(filter.slug()) {
                toggle.set_label(Some(&format!("{} {n}", filter.title())));
            }
        }
        self.replace_rows(&drawn);
        *self.drawn.borrow_mut() = Some(drawn);
    }

    /// Rebuilds every row, so each switch shows the daemon's value again.
    fn redraw(self: &Rc<Self>) {
        self.drawn.replace(None);
        self.render();
    }

    fn drawn_now(&self) -> Drawn {
        let filter = self.filter();
        let query = self.search.text().to_string();
        let data = self.data.borrow();
        let rows = data.list().map_or(&[][..], |l| l.plugins.as_slice());
        let clients = data
            .list()
            .map_or_else(Vec::new, |l| client_lines(l, data.locked));
        Drawn {
            counts: Filter::ALL.iter().map(|f| count(*f, rows)).collect(),
            body: body(&data, filter, &query),
            clients,
            elsewhere: data
                .jobs
                .iter()
                .filter(|j| j.name.is_none())
                .map(|j| job_words(&j.kind))
                .collect(),
        }
    }

    fn replace_rows(self: &Rc<Self>, drawn: &Drawn) {
        for old in self.list_rows.borrow_mut().drain(..) {
            self.list.remove(&old);
        }
        for old in self.client_rows.borrow_mut().drain(..) {
            self.clients.remove(&old);
        }
        let rows: Vec<gtk::Widget> = match &drawn.body {
            Body::Note(text, spinning) => vec![note_row(text, *spinning).upcast()],
            Body::Plugins(lines) => lines.iter().map(|l| self.plugin_row(l).upcast()).collect(),
            Body::Features(lines) => lines
                .iter()
                .map(|(id, state)| feature_row(id, state).upcast())
                .collect(),
        };
        let clients: Vec<gtk::Widget> = drawn
            .clients
            .iter()
            .map(|c| self.client_row(c).upcast())
            .collect();
        for row in &rows {
            self.list.add(row);
        }
        for row in &clients {
            self.clients.add(row);
        }
        self.clients.set_visible(!clients.is_empty());
        let elsewhere = (!drawn.elsewhere.is_empty())
            .then(|| format!("Still running from before: {}", drawn.elsewhere.join(" ")));
        self.list.set_description(elsewhere.as_deref());
        *self.list_rows.borrow_mut() = rows;
        *self.client_rows.borrow_mut() = clients;
    }

    /// A plugin: its mark, name and one line, an enable switch, and a chevron
    /// to its detail. The switch is its own control: flipping it never opens the detail.
    fn plugin_row(self: &Rc<Self>, line: &Line) -> adw::ActionRow {
        let text = line.error.as_deref().unwrap_or(&line.text);
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&line.title))
            .subtitle(glib::markup_escape_text(text))
            .subtitle_lines(1)
            .activatable(true)
            .build();
        row.add_prefix(&mark(Kind::Plugin, &line.name));
        if line.error.is_some() {
            let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
            warning.add_css_class("error");
            row.add_suffix(&warning);
        }
        if line.busy {
            row.add_suffix(&adw::Spinner::new());
        }
        row.add_suffix(&self.plugin_switch(line));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let (weak, name) = (Rc::downgrade(self), line.name.clone());
        row.connect_activated(move |_| {
            if let Some(pane) = weak.upgrade() {
                pane.open_detail(&name);
            }
        });
        row
    }

    fn plugin_switch(self: &Rc<Self>, line: &Line) -> gtk::Switch {
        let switch = gtk::Switch::builder()
            .active(line.enabled)
            .valign(gtk::Align::Center)
            .sensitive(!line.locked && !line.busy)
            .build();
        switch.update_property(&[gtk::accessible::Property::Label(&format!(
            "Turn {} on or off",
            line.title
        ))]);
        let (weak, name) = (Rc::downgrade(self), line.name.clone());
        switch.connect_active_notify(move |switch| {
            switch.set_sensitive(false);
            if let Some(pane) = weak.upgrade() {
                pane.switched(name.clone(), switch.is_active());
            }
        });
        switch
    }

    fn client_row(self: &Rc<Self>, line: &ClientLine) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&line.title))
            .subtitle(glib::markup_escape_text(&line.state))
            .build();
        row.add_prefix(&mark(Kind::OAuthClient, &line.provider));
        let verb = if line.configured {
            "Edit…"
        } else {
            "Set up…"
        };
        let button = gtk::Button::builder()
            .label(verb)
            .valign(gtk::Align::Center)
            .sensitive(!line.locked)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&format!(
            "{verb} the {} sign-in client",
            line.title
        ))]);
        let (weak, provider) = (Rc::downgrade(self), line.provider.clone());
        button.connect_clicked(move |_| {
            if let Some(pane) = weak.upgrade() {
                pane.open_client(&provider);
            }
        });
        row.add_suffix(&button);
        row
    }

    // ---- the switch, install, enable and the other actions ----

    fn switched(self: &Rc<Self>, name: String, on: bool) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let row = pane.data.borrow().row(&name);
            let Some(row) = row else {
                glib::g_warning!(
                    "fermix",
                    "switched plugin {name}, which the list no longer has"
                );
                return pane.redraw();
            };
            pane.data.borrow_mut().errors.remove(&name);
            match (on, row.installed) {
                (true, false) => pane.install(&row).await,
                (true, true) => pane.enable(&name).await,
                (false, _) => pane.disable(&name).await,
            }
            pane.redraw();
        });
    }

    /// Runs one button's action. Routing is on the daemon's action id alone.
    fn act(self: &Rc<Self>, name: String, action: PluginAction) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let row = pane.data.borrow().row(&name);
            let Some(row) = row else {
                glib::g_warning!(
                    "fermix",
                    "an action on plugin {name}, which the list no longer has"
                );
                return;
            };
            pane.data.borrow_mut().errors.remove(&name);
            pane.render();
            pane.run(&row, action).await;
        });
    }

    async fn run(self: &Rc<Self>, row: &PluginRow, action: PluginAction) {
        match action {
            PluginAction::Install => self.install(row).await,
            PluginAction::Enable => self.enable(&row.name).await,
            PluginAction::Disable => self.disable(&row.name).await,
            PluginAction::SignIn => self.sign_in(&row.name).await,
            PluginAction::Check => self.check(&row.name).await,
            PluginAction::Disconnect => self.disconnect(row).await,
            PluginAction::AddToken | PluginAction::ReplaceToken => self.ask_token(row),
            PluginAction::SetUpClient => self.push_client(row),
            PluginAction::ChooseWorkspace => self.push_workspace(row),
        }
    }

    /// Consent first: the daemon asks none of its own. Then the install job,
    /// and only a completed one goes on to the enable the switch asked for.
    async fn install(self: &Rc<Self>, row: &PluginRow) {
        let heading = format!("Install {}?", row.title);
        if !confirm(&self.page, &heading, &consent_body(row), "Install", false).await {
            return;
        }
        let name = row.name.clone();
        let started = self
            .daemon
            .call(move |m| m.plugins_install_start(&name))
            .await;
        let job = match started {
            Ok(job) => job,
            Err(e) => return self.refused(&row.name, e).await,
        };
        match self.follow(Some(&row.name), job, None).await {
            Outcome::Completed => self.enable(&row.name).await,
            ended => self.ended(&row.name, ended).await,
        }
    }

    /// Enables, reads the list again, and opens the detail when the re-read
    /// row leads with a step only the person can take. It never starts that step.
    async fn enable(self: &Rc<Self>, name: &str) {
        let n = name.to_owned();
        if let Err(e) = self.daemon.call(move |m| m.plugins_enable(&n)).await {
            return self.refused(name, e).await;
        }
        self.refresh().await;
        let next = self.data.borrow().row(name);
        if next.as_ref().is_some_and(needs_operator) {
            self.open_detail(name);
        }
    }

    async fn disable(self: &Rc<Self>, name: &str) {
        let n = name.to_owned();
        match self.daemon.call(move |m| m.plugins_disable(&n)).await {
            Ok(_) => self.refresh().await,
            Err(e) => self.refused(name, e).await,
        }
    }

    async fn check(self: &Rc<Self>, name: &str) {
        let n = name.to_owned();
        match self.daemon.call(move |m| m.plugins_check_start(&n)).await {
            Ok(job) => {
                let ended = self.follow(Some(name), job, None).await;
                self.ended(name, ended).await;
            }
            Err(e) => self.refused(name, e).await,
        }
    }

    async fn disconnect(self: &Rc<Self>, row: &PluginRow) {
        let heading = format!("Disconnect {}?", row.title);
        let body = format!(
            "Fermix forgets this plugin's credential on this computer. Nothing is revoked at {}.",
            row.title
        );
        if !confirm(&self.page, &heading, &body, "Disconnect", true).await {
            return;
        }
        let name = row.name.clone();
        match self.daemon.call(move |m| m.plugins_disconnect(&name)).await {
            Ok(_) => self.refresh().await,
            Err(e) => self.refused(&row.name, e).await,
        }
    }

    /// The plugin's own browser sign-in, started only by its button.
    async fn sign_in(self: &Rc<Self>, name: &str) {
        let provider = sign_in_provider(name);
        let start = match self.daemon.call(move |m| m.auth_start(&provider)).await {
            Ok(start) => start,
            Err(e) => return self.refused(name, e).await,
        };
        let Some(url) = start.authorize_url.clone() else {
            return self.ended(name, Outcome::Failed(NO_LINK.to_owned())).await;
        };
        self.open_browser(name, &url);
        let ended = self.follow(Some(name), start.job, Some(url)).await;
        self.ended(name, ended).await;
    }

    /// Opens the link through the desktop's browser. If it does not open, the
    /// plugin says so, and the running sign-in keeps its Copy link.
    fn open_browser(self: &Rc<Self>, name: &str, url: &str) {
        let (pane, name, launcher) = (self.clone(), name.to_owned(), gtk::UriLauncher::new(url));
        glib::spawn_future_local(async move {
            let window = pane.page.root().and_downcast::<gtk::Window>();
            let Err(e) = launcher.launch_future(window.as_ref()).await else {
                return;
            };
            glib::g_warning!("fermix", "the browser did not open for plugin {name}: {e}");
            pane.data
                .borrow_mut()
                .errors
                .insert(name, NO_BROWSER.to_owned());
            pane.render();
        });
    }

    fn cancel_job(self: &Rc<Self>, job_id: String) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = pane.daemon.call(move |m| m.job_cancel(&job_id)).await {
                (pane.toast)(&sentence_for(e, "job.cancel"));
            }
        });
    }

    /// A job that did not complete puts its sentence on the plugin; either way the list is read again.
    async fn ended(self: &Rc<Self>, name: &str, ended: Outcome) {
        if let Some(sentence) = failure_sentence(ended) {
            self.data
                .borrow_mut()
                .errors
                .insert(name.to_owned(), sentence);
        }
        self.refresh().await;
    }

    async fn refused(self: &Rc<Self>, name: &str, e: CallError) {
        let sentence = sentence_for(e, name);
        self.data
            .borrow_mut()
            .errors
            .insert(name.to_owned(), sentence);
        self.refresh().await;
    }

    fn set_setting(self: &Rc<Self>, name: String, key: String, value: String) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let n = name.clone();
            match pane
                .daemon
                .call(move |m| m.plugins_setting_set(&n, &key, &value))
                .await
            {
                Ok(_) => pane.refresh().await,
                Err(e) => pane.refused(&name, e).await,
            }
        });
    }

    // ---- secrets ----

    fn ask_token(self: &Rc<Self>, row: &PluginRow) {
        let title = format!("{} token", row.title);
        let prompt = SecretPrompt {
            title: &title,
            description: KEYRING_NOTE,
            entry_title: "Token",
        };
        let (pane, id) = (self.clone(), plugin_secret_id(&row.name));
        secret_dialog(&self.page, prompt, move |value| {
            pane.clone()
                .store_secret(id.clone(), value, "Token stored", Weak::new())
        });
    }

    fn ask_client_secret(self: &Rc<Self>, form: &Rc<ClientForm>) {
        let title = format!("{} client secret", form.title);
        let prompt = SecretPrompt {
            title: &title,
            description: KEYRING_NOTE,
            entry_title: "Client secret",
        };
        let (pane, id, form) = (
            self.clone(),
            client_secret_id(&form.provider),
            Rc::downgrade(form),
        );
        secret_dialog(&self.page, prompt, move |value| {
            pane.clone()
                .store_secret(id.clone(), value, "Client secret stored", form.clone())
        });
    }

    /// Stores a plugin token or a client secret, then reads the list again.
    /// The value goes to the daemon and nowhere else.
    async fn store_secret(
        self: Rc<Self>,
        id: String,
        value: String,
        stored: &'static str,
        form: Weak<ClientForm>,
    ) -> Result<(), String> {
        let answer = self.daemon.call(move |m| m.secret_set(&id, &value)).await;
        self.refresh().await;
        answer.map_err(|e| sentence_for(e, "secret.set"))?;
        if let Some(form) = form.upgrade() {
            form.secret_stored();
        }
        (self.toast)(stored);
        Ok(())
    }

    // ---- the detail dialog ----

    fn open_detail(self: &Rc<Self>, name: &str) {
        let open = self.detail.borrow().clone();
        if let Some(open) = open {
            if open.name == name {
                return;
            }
            open.dialog.close();
        }
        let title = self.data.borrow().row(name).map(|r| r.title);
        let Some(title) = title else {
            glib::g_warning!(
                "fermix",
                "the detail of plugin {name}, which the list no longer has"
            );
            return;
        };
        let holder = adw::Bin::new();
        let nav = adw::NavigationView::new();
        nav.add(&nav_page(&title, &holder, None));
        let dialog = dialog_with(&title, &nav);
        let detail = Rc::new(Detail {
            name: name.to_owned(),
            dialog: dialog.clone(),
            nav,
            holder,
            shown: RefCell::default(),
        });
        let weak = Rc::downgrade(self);
        dialog.connect_closed(move |dialog| {
            let Some(pane) = weak.upgrade() else { return };
            let current = pane
                .detail
                .borrow()
                .as_ref()
                .is_some_and(|d| d.dialog == *dialog);
            if current {
                pane.detail.replace(None);
            }
        });
        *self.detail.borrow_mut() = Some(detail);
        self.render_detail();
        dialog.present(Some(&self.page));
    }

    fn render_detail(self: &Rc<Self>) {
        let Some(detail) = self.detail.borrow().clone() else {
            return;
        };
        let view = self.detail_view(&detail.name);
        if detail.shown.borrow().as_ref() == Some(&view) {
            return;
        }
        detail.holder.set_child(Some(&self.detail_content(&view)));
        *detail.shown.borrow_mut() = Some(view);
    }

    fn detail_view(&self, name: &str) -> DetailView {
        let data = self.data.borrow();
        let row = data.row(name);
        let client = data.list().zip(row.as_ref()).and_then(|(list, row)| {
            let client = client_for(list, row)?;
            Some((client.clone(), client_title(list, &client.provider)))
        });
        DetailView {
            row,
            client,
            running: data.running(name),
            error: data.errors.get(name).cloned(),
            locked: data.locked,
        }
    }

    fn detail_content(self: &Rc<Self>, view: &DetailView) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let Some(row) = &view.row else {
            page.add(&adw::PreferencesGroup::builder().description(GONE).build());
            return page;
        };
        let blocked = view.locked || view.running.is_some();
        page.add(&self.about_group(row, view));
        let drawn = verbs(row);
        if !drawn.is_empty() {
            page.add(&self.verbs_group(&row.name, &drawn, blocked));
        }
        if !row.settings.is_empty() {
            page.add(&self.settings_group(row, blocked));
        }
        if !row.access_profiles.is_empty() {
            page.add(&self.workspace_group(row, blocked));
        }
        if let Some((client, title)) = &view.client {
            page.add(&self.client_group(row, client, title, view.locked));
        }
        page
    }

    /// Where the plugin stands, in the daemon's words, the account it uses,
    /// what is running for it, and the last refusal.
    fn about_group(self: &Rc<Self>, row: &PluginRow, view: &DetailView) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        if let Some(summary) = &row.summary {
            group.set_description(Some(&glib::markup_escape_text(summary)));
        }
        let status = fact_row("Status", &row.status_sentence);
        status.add_prefix(&mark(Kind::Plugin, &row.name));
        group.add(&status);
        if let Some(account) = &row.account_label {
            group.add(&fact_row("Account", account));
        }
        if let Some(running) = &view.running {
            group.add(&self.running_row(running));
        }
        if let Some(error) = &view.error {
            group.add(&error_label(error));
        }
        group
    }

    fn running_row(self: &Rc<Self>, running: &Running) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(job_words(&running.kind))
            .build();
        row.add_prefix(&adw::Spinner::new());
        if let Some(link) = &running.link {
            row.add_suffix(&self.copy_link_button(link));
        }
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .valign(gtk::Align::Center)
            .build();
        let (weak, job_id) = (Rc::downgrade(self), running.job_id.clone());
        cancel.connect_clicked(move |button| {
            button.set_sensitive(false);
            if let Some(pane) = weak.upgrade() {
                pane.cancel_job(job_id.clone());
            }
        });
        row.add_suffix(&cancel);
        row
    }

    fn copy_link_button(self: &Rc<Self>, link: &str) -> gtk::Button {
        let button = gtk::Button::builder()
            .label("Copy link")
            .valign(gtk::Align::Center)
            .tooltip_text("Copy the sign-in link to open it in any browser")
            .build();
        let (toast, link) = (self.toast.clone(), link.to_owned());
        button.connect_clicked(move |button| {
            button.clipboard().set_text(&link);
            toast("Link copied");
        });
        button
    }

    /// The daemon's buttons: its word on each, its action id behind each.
    fn verbs_group(
        self: &Rc<Self>,
        name: &str,
        drawn: &[Verb],
        blocked: bool,
    ) -> adw::PreferencesGroup {
        let buttons = adw::WrapBox::builder()
            .child_spacing(6)
            .line_spacing(6)
            .build();
        for verb in drawn {
            let button = gtk::Button::builder()
                .label(&verb.label)
                .sensitive(!blocked)
                .build();
            if verb.primary {
                button.add_css_class("suggested-action");
            }
            let (weak, name, action) = (Rc::downgrade(self), name.to_owned(), verb.action);
            button.connect_clicked(move |_| {
                if let Some(pane) = weak.upgrade() {
                    pane.act(name.clone(), action);
                }
            });
            buttons.append(&button);
        }
        let group = adw::PreferencesGroup::new();
        group.add(&buttons);
        group
    }

    fn settings_group(self: &Rc<Self>, row: &PluginRow, blocked: bool) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder().title("Settings").build();
        for setting in &row.settings {
            let widget: gtk::Widget = match setting.kind {
                SettingKind::Text => self.text_setting(&row.name, setting).upcast(),
                SettingKind::Boolean => self.switch_setting(&row.name, setting).upcast(),
                SettingKind::Unknown => unknown_setting(setting).upcast(),
            };
            widget.set_sensitive(!blocked);
            group.add(&widget);
        }
        group
    }

    /// A text setting commits on apply, and only when the text changed.
    fn text_setting(self: &Rc<Self>, name: &str, setting: &PluginSetting) -> adw::EntryRow {
        let current = setting.value.clone().unwrap_or_default();
        let entry = adw::EntryRow::builder()
            .title(glib::markup_escape_text(&setting.label))
            .text(current.as_str())
            .show_apply_button(true)
            .build();
        let (weak, name, key) = (Rc::downgrade(self), name.to_owned(), setting.key.clone());
        entry.connect_apply(move |entry| {
            let typed = entry.text().trim().to_owned();
            let Some(pane) = weak.upgrade() else { return };
            if typed != current {
                pane.set_setting(name.clone(), key.clone(), typed);
            }
        });
        entry
    }

    /// A boolean setting is a switch whose value travels as "true" or "false".
    fn switch_setting(self: &Rc<Self>, name: &str, setting: &PluginSetting) -> adw::SwitchRow {
        let switch = adw::SwitchRow::builder()
            .title(glib::markup_escape_text(&setting.label))
            .active(setting.is_on())
            .build();
        let (weak, name, key) = (Rc::downgrade(self), name.to_owned(), setting.key.clone());
        switch.connect_active_notify(move |switch| {
            let value = if switch.is_active() { "true" } else { "false" };
            if let Some(pane) = weak.upgrade() {
                pane.set_setting(name.clone(), key.clone(), value.to_owned());
            }
        });
        switch
    }

    fn workspace_group(self: &Rc<Self>, row: &PluginRow, blocked: bool) -> adw::PreferencesGroup {
        let current = row.workspace_label.as_deref().unwrap_or(NOT_CHOSEN);
        let line = fact_row("Workspace", current);
        let choose = gtk::Button::builder()
            .label("Choose…")
            .valign(gtk::Align::Center)
            .sensitive(!blocked)
            .build();
        choose.update_property(&[gtk::accessible::Property::Label(&format!(
            "Choose a workspace for {}",
            row.title
        ))]);
        let (weak, name) = (Rc::downgrade(self), row.name.clone());
        choose.connect_clicked(move |_| {
            if let Some(pane) = weak.upgrade() {
                pane.act(name.clone(), PluginAction::ChooseWorkspace);
            }
        });
        line.add_suffix(&choose);
        let group = adw::PreferencesGroup::new();
        group.add(&line);
        group
    }

    fn client_group(
        self: &Rc<Self>,
        row: &PluginRow,
        client: &OAuthClient,
        title: &str,
        locked: bool,
    ) -> adw::PreferencesGroup {
        let line = fact_row(&format!("{title} sign-in client"), &client_state(client));
        let verb = if client.configured {
            "Edit…"
        } else {
            "Set up…"
        };
        let edit = gtk::Button::builder()
            .label(verb)
            .valign(gtk::Align::Center)
            .sensitive(!locked)
            .build();
        let (weak, name) = (Rc::downgrade(self), row.name.clone());
        edit.connect_clicked(move |_| {
            if let Some(pane) = weak.upgrade() {
                pane.act(name.clone(), PluginAction::SetUpClient);
            }
        });
        line.add_suffix(&edit);
        let group = adw::PreferencesGroup::new();
        group.add(&line);
        group
    }

    /// Pushes a page onto the open detail, or shows it in a dialog of its own.
    fn push(&self, page: &adw::NavigationPage) {
        let detail = self.detail.borrow().clone();
        match detail {
            Some(detail) => detail.nav.push(page),
            None => present_alone(&self.page, page),
        }
    }

    // ---- the sign-in client editor ----

    fn push_client(self: &Rc<Self>, row: &PluginRow) {
        let found = {
            let data = self.data.borrow();
            data.list().and_then(|list| {
                let client = client_for(list, row)?;
                Some((client.clone(), client_title(list, &client.provider)))
            })
        };
        let Some((client, title)) = found else {
            glib::g_warning!(
                "fermix",
                "plugin {} offers a client it does not name",
                row.name
            );
            self.data
                .borrow_mut()
                .errors
                .insert(row.name.clone(), NO_CLIENT.to_owned());
            return self.render();
        };
        self.push(&self.client_page(&client, &title));
    }

    /// The foot of the list: one client's editor in a dialog of its own.
    fn open_client(self: &Rc<Self>, provider: &str) {
        let found = {
            let data = self.data.borrow();
            data.list().and_then(|list| {
                let client = list.oauth_clients.iter().find(|c| c.provider == provider)?;
                Some((client.clone(), client_title(list, provider)))
            })
        };
        let Some((client, title)) = found else {
            glib::g_warning!(
                "fermix",
                "the {provider} sign-in client is no longer listed"
            );
            return;
        };
        present_alone(&self.page, &self.client_page(&client, &title));
    }

    fn client_page(self: &Rc<Self>, client: &OAuthClient, title: &str) -> adw::NavigationPage {
        let locked = self.data.borrow().locked;
        let form = Rc::new(ClientForm::new(client, title, locked));
        let page = nav_page(
            &format!("{title} sign-in client"),
            &form.page(),
            Some(&form.save),
        );
        let (weak, weak_form) = (Rc::downgrade(self), Rc::downgrade(&form));
        form.secret_button.connect_clicked(move |_| {
            let (Some(pane), Some(form)) = (weak.upgrade(), weak_form.upgrade()) else {
                return;
            };
            pane.ask_client_secret(&form);
        });
        let (weak, weak_form) = (Rc::downgrade(self), Rc::downgrade(&form));
        form.save.connect_clicked(move |button| {
            let (Some(pane), Some(form)) = (weak.upgrade(), weak_form.upgrade()) else {
                return;
            };
            pane.save_client(&form, button);
        });
        keep_with(&page, form);
        page
    }

    /// Checks the editor, then registers the client. The secret must be stored
    /// first, a blank port is left out, and a region goes only where one is offered.
    fn save_client(self: &Rc<Self>, form: &Rc<ClientForm>, button: &gtk::Button) {
        let client = {
            let data = self.data.borrow();
            let clients = data.list().map_or(&[][..], |l| l.oauth_clients.as_slice());
            clients
                .iter()
                .find(|c| c.provider == form.provider)
                .cloned()
        };
        let Some(client) = client else {
            return form.refuse("This sign-in client is no longer listed.");
        };
        let region = form.region_choice();
        let typed = (form.client_id.text(), form.port.text());
        let answer = match client_answer(&client, &typed.0, &typed.1, region.as_deref()) {
            Ok(answer) => answer,
            Err(sentence) => return form.refuse(sentence),
        };
        button.set_sensitive(false);
        let (pane, form, button) = (self.clone(), form.clone(), button.clone());
        glib::spawn_future_local(async move {
            let set = pane
                .daemon
                .call(move |m| m.plugins_oauth_client_set(&answer))
                .await;
            pane.refresh().await;
            button.set_sensitive(true);
            if let Err(e) = set {
                return form.refuse(&sentence_for(e, "plugins.oauth_client.set"));
            }
            (pane.toast)("Sign-in client saved");
            leave(&button);
        });
    }

    // ---- choosing a workspace ----

    fn push_workspace(self: &Rc<Self>, row: &PluginRow) {
        let locked = self.data.borrow().locked;
        let form = Rc::new(WorkspaceForm::new(row, locked));
        let page = nav_page("Choose a workspace", &form.page(), Some(&form.use_it));
        let (weak, weak_form) = (Rc::downgrade(self), Rc::downgrade(&form));
        form.find.connect_activated(move |_| {
            let (Some(pane), Some(form)) = (weak.upgrade(), weak_form.upgrade()) else {
                return;
            };
            pane.find_workspaces(&form);
        });
        let (weak, weak_form) = (Rc::downgrade(self), Rc::downgrade(&form));
        form.use_it.connect_clicked(move |button| {
            let (Some(pane), Some(form)) = (weak.upgrade(), weak_form.upgrade()) else {
                return;
            };
            pane.use_workspace(&form, button);
        });
        keep_with(&page, form);
        self.push(&page);
    }

    /// Discovery puts what it found on the plugin row, not in the job, so the
    /// list is read again before the choice is filled.
    fn find_workspaces(self: &Rc<Self>, form: &Rc<WorkspaceForm>) {
        form.busy(true);
        let (pane, form) = (self.clone(), form.clone());
        glib::spawn_future_local(async move {
            let name = form.name.clone();
            let n = name.clone();
            let ended = match pane
                .daemon
                .call(move |m| m.plugins_workspaces_discover_start(&n))
                .await
            {
                Ok(job) => pane.follow(Some(&name), job, None).await,
                Err(e) => Outcome::Failed(sentence_for(e, "plugins.workspaces.discover.start")),
            };
            pane.refresh().await;
            form.busy(false);
            let found = pane
                .data
                .borrow()
                .row(&name)
                .map(|r| r.workspaces)
                .unwrap_or_default();
            form.fill(&found);
            if let Some(sentence) = failure_sentence(ended) {
                form.refuse(&sentence);
            }
        });
    }

    /// Binds the workspace, then goes back to the detail, which shows the
    /// daemon's own label for it once the job ends.
    fn use_workspace(self: &Rc<Self>, form: &Rc<WorkspaceForm>, button: &gtk::Button) {
        let (Some(profile), Some(workspace)) = (form.profile(), form.workspace()) else {
            return form.refuse(PICK_WORKSPACE);
        };
        form.busy(true);
        let (pane, form, button, name) = (
            self.clone(),
            form.clone(),
            button.clone(),
            form.name.clone(),
        );
        glib::spawn_future_local(async move {
            let n = name.clone();
            let started = pane
                .daemon
                .call(move |m| {
                    m.plugins_workspace_select_start(
                        &n,
                        &profile.id,
                        &workspace.id,
                        &workspace.label,
                    )
                })
                .await;
            form.busy(false);
            let job = match started {
                Ok(job) => job,
                Err(e) => return form.refuse(&sentence_for(e, "plugins.workspace.select.start")),
            };
            leave(&button);
            let ended = pane.follow(Some(&name), job, None).await;
            pane.ended(&name, ended).await;
        });
    }
}

/// The sign-in client editor's fields. The secret is stored through its own
/// dialog; the rest stay here until Save.
struct ClientForm {
    provider: String,
    title: String,
    secret: adw::ActionRow,
    secret_button: gtk::Button,
    client_id: adw::EntryRow,
    port: adw::EntryRow,
    /// Present only where the provider offers regions; index 0 is the prompt.
    region: Option<(adw::ComboRow, Vec<Region>)>,
    error: gtk::Label,
    save: gtk::Button,
}

impl ClientForm {
    fn new(client: &OAuthClient, title: &str, locked: bool) -> ClientForm {
        let secret = fact_row("Client secret", stored_word(client.secret_present));
        let secret_button = gtk::Button::builder()
            .label(if client.secret_present {
                "Replace…"
            } else {
                "Add…"
            })
            .valign(gtk::Align::Center)
            .sensitive(!locked)
            .build();
        secret_button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Store the {title} client secret"
        ))]);
        secret.add_suffix(&secret_button);
        let port = client
            .redirect_port
            .map(|p| p.to_string())
            .unwrap_or_default();
        ClientForm {
            provider: client.provider.clone(),
            title: title.to_owned(),
            secret,
            secret_button,
            client_id: adw::EntryRow::builder()
                .title("Client ID")
                .text(client.client_id.clone().unwrap_or_default())
                .build(),
            port: adw::EntryRow::builder()
                .title("Redirect port")
                .text(port)
                .input_purpose(gtk::InputPurpose::Digits)
                .build(),
            region: (!client.regions.is_empty()).then(|| region_combo(client)),
            error: error_label(""),
            save: gtk::Button::builder()
                .label("Save")
                .css_classes(["suggested-action"])
                .sensitive(!locked)
                .build(),
        }
    }

    fn page(&self) -> adw::PreferencesPage {
        let fields = adw::PreferencesGroup::builder()
            .description(CLIENT_NOTE)
            .build();
        fields.add(&self.secret);
        fields.add(&self.client_id);
        fields.add(&self.port);
        let page = adw::PreferencesPage::new();
        page.add(&fields);
        if let Some((combo, _)) = &self.region {
            let group = adw::PreferencesGroup::builder()
                .description(REGION_NOTE)
                .build();
            group.add(combo);
            page.add(&group);
        }
        let tail = adw::PreferencesGroup::new();
        tail.add(&self.error);
        page.add(&tail);
        page
    }

    fn region_choice(&self) -> Option<String> {
        let (combo, regions) = self.region.as_ref()?;
        let index = usize::try_from(combo.selected()).ok()?.checked_sub(1)?;
        regions.get(index).map(|r| r.id.clone())
    }

    fn secret_stored(&self) {
        self.secret.set_subtitle(stored_word(true));
        self.secret_button.set_label("Replace…");
        self.error.set_visible(false);
    }

    fn refuse(&self, sentence: &str) {
        self.error.set_text(sentence);
        self.error.set_visible(true);
    }
}

/// The workspace page's fields: the access level, then a workspace from the last discovery.
struct WorkspaceForm {
    name: String,
    profiles: Vec<AccessProfile>,
    access: adw::ComboRow,
    warning: gtk::Label,
    found: RefCell<Vec<Workspace>>,
    choice: adw::ComboRow,
    find: adw::ButtonRow,
    error: gtk::Label,
    use_it: gtk::Button,
}

impl WorkspaceForm {
    fn new(row: &PluginRow, locked: bool) -> WorkspaceForm {
        let labels: Vec<&str> = row
            .access_profiles
            .iter()
            .map(|p| p.label.as_str())
            .collect();
        let access = adw::ComboRow::builder()
            .title("Access level")
            .model(&gtk::StringList::new(&labels))
            .build();
        let warning = gtk::Label::builder()
            .label(WRITE_WARNING)
            .wrap(true)
            .xalign(0.0)
            .margin_top(6)
            .css_classes(["warning"])
            .build();
        let form = WorkspaceForm {
            name: row.name.clone(),
            profiles: row.access_profiles.clone(),
            access,
            warning,
            found: RefCell::default(),
            choice: adw::ComboRow::builder().title("Workspace").build(),
            find: adw::ButtonRow::builder()
                .title("Find workspaces")
                .sensitive(!locked)
                .build(),
            error: error_label(""),
            use_it: gtk::Button::builder()
                .label("Use")
                .css_classes(["suggested-action"])
                .sensitive(!locked)
                .build(),
        };
        form.use_it
            .update_property(&[gtk::accessible::Property::Label("Use this workspace")]);
        form.fill(&row.workspaces);
        form.warn();
        let warn = form.warning.clone();
        let profiles = form.profiles.clone();
        form.access.connect_selected_notify(move |combo| {
            let chosen = usize::try_from(combo.selected())
                .ok()
                .and_then(|i| profiles.get(i));
            warn.set_visible(chosen.is_some_and(|p| p.write));
        });
        form
    }

    fn page(&self) -> adw::PreferencesPage {
        let access = adw::PreferencesGroup::builder().title("Access").build();
        access.add(&self.access);
        access.add(&self.warning);
        let workspace = adw::PreferencesGroup::builder().title("Workspace").build();
        workspace.add(&self.choice);
        workspace.add(&self.find);
        workspace.add(&self.error);
        let page = adw::PreferencesPage::new();
        page.add(&access);
        page.add(&workspace);
        page
    }

    /// Shows what the last discovery found.
    fn fill(&self, workspaces: &[Workspace]) {
        let labels: Vec<&str> = workspaces.iter().map(|w| w.label.as_str()).collect();
        self.choice.set_model(Some(&gtk::StringList::new(&labels)));
        self.choice.set_sensitive(!workspaces.is_empty());
        let note = if workspaces.is_empty() {
            NO_WORKSPACES
        } else {
            ""
        };
        self.choice.set_subtitle(note);
        *self.found.borrow_mut() = workspaces.to_vec();
    }

    fn warn(&self) {
        self.warning
            .set_visible(self.profile().is_some_and(|p| p.write));
    }

    fn profile(&self) -> Option<AccessProfile> {
        let index = usize::try_from(self.access.selected()).ok()?;
        self.profiles.get(index).cloned()
    }

    fn workspace(&self) -> Option<Workspace> {
        let index = usize::try_from(self.choice.selected()).ok()?;
        self.found.borrow().get(index).cloned()
    }

    fn busy(&self, busy: bool) {
        self.find.set_sensitive(!busy);
        self.use_it.set_sensitive(!busy);
        if busy {
            self.error.set_visible(false);
        }
    }

    fn refuse(&self, sentence: &str) {
        self.error.set_text(sentence);
        self.error.set_visible(true);
    }
}

fn body(data: &Data, filter: Filter, query: &str) -> Body {
    let nothing = if query.trim().is_empty() {
        NOTHING
    } else {
        NO_MATCH
    };
    if filter == Filter::Features {
        let lines: Vec<(&'static str, &'static str)> = visible_features(query)
            .iter()
            .map(|f| (f.id, feature_state(f, data.features.as_ref())))
            .collect();
        if lines.is_empty() {
            return Body::Note(nothing.to_owned(), false);
        }
        return Body::Features(lines);
    }
    let list = match &data.list {
        None => return Body::Note(READING.to_owned(), true),
        Some(Err(sentence)) => return Body::Note(sentence.clone(), false),
        Some(Ok(list)) => list,
    };
    let lines: Vec<Line> = visible(&list.plugins, filter, query)
        .into_iter()
        .map(|row| Line {
            name: row.name.clone(),
            title: row.title.clone(),
            text: line(row).to_owned(),
            error: data.errors.get(&row.name).cloned(),
            enabled: row.enabled,
            busy: data.running(&row.name).is_some(),
            locked: data.locked,
        })
        .collect();
    if lines.is_empty() {
        return Body::Note(nothing.to_owned(), false);
    }
    Body::Plugins(lines)
}

fn client_lines(list: &PluginList, locked: bool) -> Vec<ClientLine> {
    list.oauth_clients
        .iter()
        .map(|c| ClientLine {
            provider: c.provider.clone(),
            title: client_title(list, &c.provider),
            state: client_state(c),
            configured: c.configured,
            locked,
        })
        .collect()
}

/// The four filters, each labelled with its count once the list is read.
fn filter_toggles() -> adw::ToggleGroup {
    let group = adw::ToggleGroup::builder()
        .halign(gtk::Align::Start)
        .build();
    for filter in Filter::ALL {
        let toggle = adw::Toggle::builder()
            .name(filter.slug())
            .label(filter.title())
            .build();
        group.add(toggle);
    }
    group.set_active_name(Some(Filter::Installed.slug()));
    group
}

/// A native feature: where it stands, and the way to the pane that owns its switch.
fn feature_row(id: &str, state: &str) -> adw::ActionRow {
    let feature = FEATURES
        .iter()
        .find(|f| f.id == id)
        .expect("a drawn feature is one of the listed ones");
    let row = adw::ActionRow::builder()
        .title(feature.title)
        .subtitle(feature.summary)
        .subtitle_lines(1)
        .activatable(true)
        .build();
    row.add_prefix(&mark(Kind::Feature, feature.id));
    let words = gtk::Label::builder()
        .label(state)
        .css_classes(["dim-label"])
        .build();
    row.add_suffix(&words);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let target = feature.pane;
    row.connect_activated(move |row| {
        if let Err(e) = row.activate_action("win.page", Some(&target.to_variant())) {
            glib::g_warning!("fermix", "the {target} pane could not open: {e}");
        }
    });
    row
}

fn note_row(text: &str, spinning: bool) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(text))
        .build();
    if spinning {
        row.add_suffix(&adw::Spinner::new());
    }
    row
}

/// A fact in the daemon's words: the label small, the value large.
fn fact_row(title: &str, value: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(value))
        .css_classes(["property"])
        .build()
}

fn unknown_setting(setting: &PluginSetting) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(&setting.label))
        .subtitle(UNKNOWN_KIND)
        .build()
}

fn error_label(sentence: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(sentence)
        .wrap(true)
        .xalign(0.0)
        .margin_top(6)
        .css_classes(["error"])
        .visible(!sentence.is_empty())
        .build()
}

fn stored_word(present: bool) -> &'static str {
    if present {
        "Stored"
    } else {
        "Not stored"
    }
}

fn region_combo(client: &OAuthClient) -> (adw::ComboRow, Vec<Region>) {
    let labels: Vec<&str> = std::iter::once(REGION_PROMPT)
        .chain(client.regions.iter().map(|r| r.label.as_str()))
        .collect();
    let current = client
        .region
        .as_deref()
        .and_then(|id| client.regions.iter().position(|r| r.id == id))
        .map_or(0, |index| index + 1);
    let combo = adw::ComboRow::builder()
        .title("Account region")
        .model(&gtk::StringList::new(&labels))
        .selected(u32::try_from(current).expect("a handful of regions"))
        .build();
    (combo, client.regions.clone())
}

/// A page of a dialog: its title in a header bar, and an optional action at its end.
fn nav_page(
    title: &str,
    content: &impl IsA<gtk::Widget>,
    action: Option<&gtk::Button>,
) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    if let Some(action) = action {
        header.pack_end(action);
    }
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(content));
    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
}

/// A dialog around a page stack. Its title is what assistive technology announces.
fn dialog_with(title: &str, nav: &adw::NavigationView) -> adw::Dialog {
    adw::Dialog::builder()
        .title(title)
        .content_width(DIALOG_WIDTH)
        .content_height(DIALOG_HEIGHT)
        .child(nav)
        .build()
}

fn present_alone(parent: &adw::PreferencesPage, page: &adw::NavigationPage) {
    let nav = adw::NavigationView::new();
    nav.add(page);
    dialog_with(&page.title(), &nav).present(Some(parent));
}

/// The page owns its form. The form's own widgets hold it only weakly, so
/// when the page goes, its handlers go, and the form with them.
fn keep_with<T: 'static>(page: &adw::NavigationPage, form: Rc<T>) {
    page.connect_destroy(move |_| {
        let _ = &form;
    });
}

/// Goes back from the page holding `widget`, or closes the dialog it is the
/// only page of. A page already left stays left.
fn leave(widget: &impl IsA<gtk::Widget>) {
    let page = widget
        .ancestor(adw::NavigationPage::static_type())
        .and_downcast::<adw::NavigationPage>();
    let nav = widget
        .ancestor(adw::NavigationView::static_type())
        .and_downcast::<adw::NavigationView>();
    let (Some(page), Some(nav)) = (page, nav) else {
        return;
    };
    if nav.visible_page().as_ref() != Some(&page) || nav.pop() {
        return;
    }
    let dialog = nav
        .ancestor(adw::Dialog::static_type())
        .and_downcast::<adw::Dialog>();
    if let Some(dialog) = dialog {
        dialog.close();
    }
}

/// What a job that did not complete leaves to say. A cancelled job has no
/// sentence of its own and is not a success.
fn failure_sentence(ended: Outcome) -> Option<String> {
    match ended {
        Outcome::Failed(sentence) => Some(sentence),
        Outcome::Cancelled => Some(STOPPED.to_owned()),
        Outcome::Completed | Outcome::Running(_) => None,
    }
}

/// The daemon's own sentence for a refusal. Losing the daemon is logged, and
/// says what became of it.
fn sentence_for(e: CallError, what: &str) -> String {
    match e {
        CallError::Refused(refusal) => refusal.sentence,
        other => {
            glib::g_warning!("fermix", "an integrations call ({what}) failed: {other:?}");
            daemon_problem(&other).sentence()
        }
    }
}
