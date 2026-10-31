//! The Always On pane (M75 lean v1 §7): one sentence and at most one main action per screen, the
//! engine's words, Details for the expert and Learn more to the docs. Which screen shows, what it
//! says, where each action goes and the calls it makes are `fermix_client::always_on`'s; this
//! file only draws the model and runs the calls it hands over, each off the window's thread.
//!
//! The pane owns its daemon calls, as Integrations does. It reads `home.status` for as long as
//! the app runs, window or none, at the pace the model sets, and raises the desktop's own
//! notifications from what changed between two reads. The `snapshots` section, which the daemon
//! files under this pane, is added below by `SettingsPage` and drawn by the one settings model;
//! it shows while Details is open. The fields someone types in stay for the pane's life, so a
//! read every few seconds never wipes a token or a passphrase being typed.

use crate::daemon::Daemon;
use crate::dialogs::{form_dialog, leave};
use crate::phone::{copy_secret, withdraw};
use crate::status::{down_view, DownPage};
use adw::prelude::*;
use fermix_client::always_on::actions::{perform, Context, Outcome};
use fermix_client::always_on::model::{Model, Press};
use fermix_client::always_on::notices::Notice;
use fermix_client::always_on::{
    self as words, details, own_server_command, sentence, Fact, Page, Screen, ScreenKind,
};
use fermix_client::home::{Action, ActionId, HomeStatus, ItemStatus, ReadinessItem};
use fermix_client::management::{CallError, Management};
use fermix_client::provision::{Options, Pick, Token};
use fermix_client::view::{daemon_problem, DAEMON_TOO_OLD};
use futures_channel::oneshot;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A token is at least this long, so a change that adds this much at once was pasted.
const PASTED: usize = 16;
/// How often the bar that claims no percentage moves.
const PULSE: Duration = Duration::from_millis(120);
const DIALOG_WIDTH: i32 = 480;
const DIALOG_HEIGHT: i32 = 560;

/// Hears each size and place picked in a list of offers, with the widget it came from.
type Chosen = Rc<dyn Fn(Pick, &gtk::Widget)>;

/// One read: the methods `hello` lists, and `home.status` where it lists it.
struct HomeRead {
    methods: Vec<String>,
    status: Option<Result<HomeStatus, CallError>>,
}

fn read_home(m: &Management) -> Result<HomeRead, CallError> {
    let methods = m.hello()?.capabilities.methods;
    let serves = methods.iter().any(|name| name == "home.status");
    let status = serves.then(|| m.home_status());
    Ok(HomeRead { methods, status })
}

/// What the page was drawn from; a page equal to it draws nothing again.
#[derive(Clone, PartialEq)]
struct Drawn {
    page: Page,
    sentence: String,
    note: Option<String>,
    readiness: Vec<ReadinessItem>,
    facts: Vec<Fact>,
    working: Option<ActionId>,
    refusal: Option<String>,
    notice: Option<String>,
    hetzner: bool,
    options_read: bool,
    code: Option<(String, String)>,
    code_refusal: Option<String>,
    getting_code: bool,
}

/// A group whose rows follow what was drawn.
struct Rows {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
}

impl Rows {
    fn new(group: adw::PreferencesGroup) -> Rows {
        Rows {
            group,
            rows: RefCell::default(),
        }
    }

    fn replace(&self, rows: Vec<gtk::Widget>) {
        for row in self.rows.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
        for row in &rows {
            self.group.add(row);
        }
        self.group.set_visible(!rows.is_empty());
        *self.rows.borrow_mut() = rows;
    }
}

/// The paste field and what sits with it.
struct TokenField {
    group: adw::PreferencesGroup,
    entry: adw::PasswordEntryRow,
    checking: adw::ActionRow,
    refusal: adw::ActionRow,
    /// The update sentence, in the field's place on an engine without the provisioner.
    newer: adw::ActionRow,
    how: adw::ExpanderRow,
    steps: RefCell<(Vec<String>, Vec<gtk::Widget>)>,
    /// Open Hetzner, after the steps.
    open: adw::ButtonRow,
    length: Cell<usize>,
}

/// The recovery passphrase, typed twice while the server starts.
struct PassphraseFields {
    group: adw::PreferencesGroup,
    first: adw::PasswordEntryRow,
    second: adw::PasswordEntryRow,
    set: gtk::Button,
    spinner: adw::Spinner,
    refusal: adw::ActionRow,
}

struct Widgets {
    /// The sentence, the bar and a note on the last read, with Learn more in its header.
    screen: Rows,
    learn: gtk::Button,
    token: TokenField,
    /// What the screen holds below the field: its links, Change, the review.
    body: Rows,
    /// The one main action, and what the daemon said of the last one.
    action: Rows,
    passphrase: PassphraseFields,
    /// The own server's command, with its code inside.
    own: Rows,
    details: adw::PreferencesGroup,
    details_row: adw::ExpanderRow,
    details_rows: RefCell<Vec<gtk::Widget>>,
}

pub struct AlwaysOnPane {
    me: Weak<AlwaysOnPane>,
    /// The page, or the down page while Fermix on this computer cannot be read.
    pub root: gtk::Stack,
    page: adw::PreferencesPage,
    down: DownPage,
    daemon: Daemon,
    notify: Rc<dyn Fn(&Notice)>,
    model: RefCell<Model>,
    w: Widgets,
    drawn: RefCell<Option<Drawn>>,
    /// The daemon's sections filed under this pane, shown while Details is open.
    sections: RefCell<Vec<adw::PreferencesGroup>>,
    /// Answers when the last read asked has landed, so reads land in the order they were asked.
    reading: RefCell<Option<oneshot::Receiver<()>>>,
    /// Each wait scheduled counts up, so a timer from before does nothing when it fires.
    generation: Cell<u64>,
    /// The command this pane copied, taken back when the pane goes.
    copied: Rc<RefCell<Option<gdk::ContentProvider>>>,
    dialog: RefCell<Option<adw::Dialog>>,
}

impl AlwaysOnPane {
    /// `notify` raises one desktop notification.
    pub fn new(daemon: Daemon, notify: Rc<dyn Fn(&Notice)>) -> Rc<AlwaysOnPane> {
        let page = adw::PreferencesPage::new();
        let w = widgets();
        for group in [
            &w.screen.group,
            &w.token.group,
            &w.body.group,
            &w.action.group,
            &w.passphrase.group,
            &w.own.group,
            &w.details,
        ] {
            page.add(group);
        }
        let down = DownPage::new();
        let root = gtk::Stack::new();
        root.add_named(&page, Some("page"));
        root.add_named(&down.page, Some("down"));
        let pane = Rc::new_cyclic(|me| AlwaysOnPane {
            me: me.clone(),
            root,
            page,
            down,
            daemon,
            notify,
            model: RefCell::default(),
            w,
            drawn: RefCell::default(),
            sections: RefCell::default(),
            reading: RefCell::default(),
            generation: Cell::new(0),
            copied: Rc::default(),
            dialog: RefCell::default(),
        });
        pane.wire();
        pane.render();
        pane
    }

    /// Adds one of the daemon's sections filed under this pane, drawn by the settings model.
    pub fn add_section(&self, group: &adw::PreferencesGroup) {
        self.page.add(group);
        self.sections.borrow_mut().push(group.clone());
        self.show_sections();
    }

    /// Reads the summary now, and from then on at the model's pace for as long as the app runs.
    /// The app starts it once, at launch.
    pub fn start_follow(&self) {
        self.spawn(|pane| async move { pane.read().await });
    }

    fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Rc<AlwaysOnPane>) -> Fut,
        Fut: std::future::Future<Output = ()> + 'static,
    {
        if let Some(pane) = self.me.upgrade() {
            glib::spawn_future_local(f(pane));
        }
    }

    // ---- Wiring --------------------------------------------------------------------------

    fn wire(&self) {
        let me = self.me.clone();
        self.root.connect_map(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.shown();
            }
        });
        let me = self.me.clone();
        self.root.connect_unmap(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.hidden();
            }
        });
        let me = self.me.clone();
        self.w.learn.connect_clicked(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.act(ActionId::LearnMore);
            }
        });
        let me = self.me.clone();
        self.w.details_row.connect_expanded_notify(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.show_sections();
            }
        });
        self.wire_token();
        self.wire_passphrase();
    }

    fn wire_token(&self) {
        let me = self.me.clone();
        self.w.token.entry.connect_changed(move |entry| {
            let Some(pane) = me.upgrade() else { return };
            let length = entry.text().chars().count();
            let before = pane.w.token.length.replace(length);
            if length >= before + PASTED {
                pane.check_token();
            }
        });
        let me = self.me.clone();
        self.w.token.entry.connect_entry_activated(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.check_token();
            }
        });
        let me = self.me.clone();
        self.w.token.open.connect_activated(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.act(ActionId::OpenHetzner);
            }
        });
    }

    fn wire_passphrase(&self) {
        let me = self.me.clone();
        self.w.passphrase.set.connect_clicked(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.spawn(|pane| async move { pane.set_passphrase().await });
            }
        });
    }

    // ---- Coming and going ----------------------------------------------------------------

    /// The pane appeared: it reads now, reads Hetzner back first where the summary rests on
    /// it, and what Hetzner sells on the choose screen.
    fn shown(&self) {
        self.model.borrow_mut().pane_shown();
        self.spawn(|pane| async move {
            pane.read().await;
            if pane.model.borrow().needs_reconcile() {
                pane.reconcile().await;
            }
            if pane.model.borrow().choosing() {
                pane.read_options().await;
            }
        });
    }

    /// The pane went: the code it showed, and its copy on the clipboard, go with it.
    fn hidden(&self) {
        self.model.borrow_mut().pane_hidden();
        withdraw(&self.page.clipboard(), &self.copied);
        self.render();
        self.schedule();
    }

    // ---- Reads -----------------------------------------------------------------------------

    /// One read of `home.status`, asked once the read before it has landed; the notification
    /// the change calls for is raised, and the next read is scheduled.
    async fn read(&self) {
        let (landed, turn) = oneshot::channel();
        let before = self.reading.replace(Some(turn));
        if let Some(before) = before {
            // A cancelled sender is a read whose future was dropped; its turn is over all the same.
            let _ = before.await;
        }
        let answer = self.daemon.call(read_home).await;
        let told = self.apply(answer);
        // Nobody waiting is the newest read; there is no one to hand the turn to.
        let _ = landed.send(());
        if let Some(notice) = told {
            (self.notify)(&notice);
        }
        self.render();
        self.schedule();
    }

    fn apply(&self, answer: Result<HomeRead, CallError>) -> Option<Notice> {
        let mut model = self.model.borrow_mut();
        let read = match answer {
            Ok(read) => read,
            Err(e) => {
                glib::g_debug!("fermix", "Always On could not read hello: {e:?}");
                model.unanswered(daemon_problem(&e));
                return None;
            }
        };
        model.served(read.methods);
        match read.status? {
            Ok(status) => model.answered(status, unix_now()),
            Err(e) => {
                glib::g_debug!("fermix", "home.status was not read: {e:?}");
                model.unanswered(daemon_problem(&e));
                None
            }
        }
    }

    fn schedule(&self) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let wait = self.model.borrow().next_wait();
        let me = self.me.clone();
        glib::timeout_add_local_once(wait, move || {
            let Some(pane) = me.upgrade() else { return };
            if pane.generation.get() == generation {
                glib::spawn_future_local(async move { pane.read().await });
            }
        });
    }

    /// Reads the provisioner's record back from Hetzner, then the summary that rests on it. A
    /// refused read is logged: the summary then says what the record last held.
    async fn reconcile(&self) {
        let answer = self.daemon.call(|m| m.provision_resources()).await;
        if let Err(e) = answer {
            glib::g_warning!("fermix", "home.provision.resources was not read: {e:?}");
        }
        self.read().await;
    }

    async fn read_options(&self) {
        let answer = self.daemon.call(|m| m.provision_options()).await;
        let answer = answer.map_err(|e| sentence(&e));
        self.model.borrow_mut().options_read(answer);
        self.render();
    }

    // ---- Actions -----------------------------------------------------------------------------

    /// One action the screen offers was pressed.
    fn act(&self, id: ActionId) {
        let press = self.model.borrow_mut().press(id);
        self.render();
        self.spawn(|pane| async move { pane.pressed(press).await });
    }

    async fn pressed(self: Rc<Self>, press: Press) {
        match press {
            Press::Drawn => self.step_taken().await,
            Press::Open(url) => self.open(url).await,
            Press::Update => self.activate("win.updates-open"),
            Press::GetCode => self.get_code().await,
            Press::Run(context) => self.run(*context).await,
            Press::Ignored => {}
        }
    }

    /// The calls of one action, off the window's thread, then a read of what they changed.
    async fn run(&self, context: Context) {
        self.render();
        let answer = self.daemon.call(move |m| Ok(perform(m, &context))).await;
        let outcome = answer.unwrap_or_else(|e| Outcome {
            refusal: Some(sentence(&e)),
            ..Outcome::default()
        });
        self.model.borrow_mut().finished(outcome);
        self.render();
        self.read().await;
    }

    fn check_token(&self) {
        let Some(token) = Token::pasted(&self.w.token.entry.text()) else {
            return;
        };
        if self.model.borrow().checking_token {
            return;
        }
        self.model.borrow_mut().checking_token = true;
        self.render();
        self.spawn(|pane| async move {
            let answer = pane.daemon.call(move |m| m.provision_connect(&token)).await;
            let answer = answer.map_err(|e| sentence(&e));
            let worked = answer.is_ok();
            pane.model.borrow_mut().token_checked(answer);
            if worked {
                pane.w.token.entry.set_text("");
            }
            pane.read().await;
            if pane.model.borrow().choosing() {
                pane.read_options().await;
            }
        });
    }

    async fn set_passphrase(&self) {
        let (first, second) = (
            self.w.passphrase.first.text(),
            self.w.passphrase.second.text(),
        );
        let typed = self.model.borrow_mut().passphrase_typed(&first, &second);
        self.render();
        let Some(typed) = typed else { return };
        self.model.borrow_mut().setting_passphrase = true;
        self.render();
        let sent = typed.clone();
        let answer = self
            .daemon
            .call(move |m| m.home_snapshots_passphrase(&sent))
            .await;
        let answer = answer.map(|_| ()).map_err(|e| sentence(&e));
        let worked = answer.is_ok();
        self.model.borrow_mut().passphrase_set(typed, answer);
        if worked {
            self.w.passphrase.first.set_text("");
            self.w.passphrase.second.set_text("");
        }
        self.read().await;
    }

    /// The code for a server of the owner's own, shown once inside the command it runs.
    async fn get_code(&self) {
        let answer = self.daemon.call(|m| m.home_enroll_open()).await;
        let answer = answer.map_err(|e| sentence(&e));
        self.model.borrow_mut().code_answered(answer);
        self.render();
        self.read().await;
    }

    async fn open(&self, url: &str) {
        let window = self.root.root().and_downcast::<gtk::Window>();
        let launched = gtk::UriLauncher::new(url)
            .launch_future(window.as_ref())
            .await;
        if let Err(e) = launched {
            glib::g_warning!("fermix", "{url} did not open: {e}");
            self.model.borrow_mut().refusal = Some(words::LINK_FAILED.to_owned());
            self.render();
        }
    }

    fn activate(&self, action: &str) {
        if let Err(e) = self.root.activate_action(action, None) {
            glib::g_warning!("fermix", "{action} could not be sent: {e}");
        }
    }

    // ---- Drawing -----------------------------------------------------------------------------

    fn render(&self) {
        let drawn = self.drawn_now();
        self.render_fields(&drawn);
        self.present_dialogs();
        if self.drawn.borrow().as_ref() == Some(&drawn) {
            return;
        }
        self.show_page(&drawn);
        *self.drawn.borrow_mut() = Some(drawn);
        self.show_sections();
    }

    fn drawn_now(&self) -> Drawn {
        let model = self.model.borrow();
        let page = model.page();
        let screen = page.clone().screen();
        let offer = model.pick().and_then(|pick| {
            let options = model.loaded_options()?;
            Some((options.offer(&pick)?.clone(), options.currency.clone()))
        });
        let code = model.handout.as_ref().and_then(|code| {
            let line = words::own_server_line(&local_time(&code.expires_at));
            Some((line, own_server_command(code)?))
        });
        Drawn {
            sentence: screen
                .as_ref()
                .map(|s| model.choose_sentence(s))
                .unwrap_or_default(),
            note: model.read_note(),
            readiness: screen
                .as_ref()
                .map(|s| model.readiness(s))
                .unwrap_or_default(),
            facts: screen
                .as_ref()
                .map(|s| details(s, offer.as_ref().map(|(o, c)| (o, c.as_str()))))
                .unwrap_or_default(),
            page,
            working: model.working,
            refusal: model.refusal.clone(),
            notice: model.notice.clone(),
            hetzner: model.serves_hetzner(),
            options_read: model.loaded_options().is_some(),
            code,
            code_refusal: model.code_refusal.clone(),
            getting_code: model.getting_code,
        }
    }

    fn show_page(&self, drawn: &Drawn) {
        if let Page::Unreadable(problem) = &drawn.page {
            self.down
                .show(down_view(problem, false, true, words::NEEDS_FERMIX));
            self.root.set_visible_child_name("down");
            return;
        }
        self.down.reset();
        self.root.set_visible_child_name("page");
        let screen = drawn.page.clone().screen();
        self.w
            .screen
            .replace(self.screen_rows(drawn, screen.as_ref()));
        self.w.body.replace(match &screen {
            Some(screen) => self.body_rows(drawn, screen),
            None => Vec::new(),
        });
        let action = screen.as_ref().and_then(|s| s.action.clone());
        self.w
            .action
            .replace(self.action_rows(drawn, action.as_ref()));
        self.w.own.replace(self.own_rows(drawn));
        self.show_details(drawn, screen.as_ref());
    }

    /// The sentence, the bar and a note on the last read; a spinner while the first read is out,
    /// and the update sentence for an engine older than the summary.
    fn screen_rows(&self, drawn: &Drawn, screen: Option<&Screen>) -> Vec<gtk::Widget> {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        match (&drawn.page, screen) {
            (Page::Reading, _) => column.append(&spinner_line(words::READING)),
            (Page::NeedsUpdate, _) => column.append(&text(DAEMON_TOO_OLD)),
            (_, Some(screen)) => {
                column.append(&text(&drawn.sentence));
                if screen.shows_progress {
                    column.append(&pulsing_bar());
                }
            }
            (_, None) => {}
        }
        if let Some(note) = &drawn.note {
            column.append(&attention(note));
        }
        vec![column.upcast()]
    }

    /// The token screen's links, Change on choose, and the review's warnings on ready.
    fn body_rows(&self, drawn: &Drawn, screen: &Screen) -> Vec<gtk::Widget> {
        match screen.kind {
            ScreenKind::Token => screen
                .alternatives
                .iter()
                .map(|a| self.action_row(a, drawn).upcast())
                .collect(),
            ScreenKind::Choose => vec![self.change_row(drawn).upcast()],
            ScreenKind::Ready => drawn.readiness.iter().map(item_row).collect(),
            _ => Vec::new(),
        }
    }

    /// The one main action, its spinner while it waits on the daemon, and the daemon's refusal
    /// or notice under it.
    fn action_rows(&self, drawn: &Drawn, action: Option<&Action>) -> Vec<gtk::Widget> {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        if let Some(action) = action {
            column.append(&self.main_button(action, drawn));
        }
        if let Some(refusal) = &drawn.refusal {
            column.append(&attention(refusal));
        }
        if let Some(notice) = &drawn.notice {
            column.append(&text(notice));
        }
        match column.first_child() {
            Some(_) => vec![column.upcast()],
            None => Vec::new(),
        }
    }

    fn main_button(&self, action: &Action, drawn: &Drawn) -> gtk::Box {
        let button = gtk::Button::builder()
            .label(&action.label)
            .css_classes(["pill", "suggested-action"])
            .sensitive(drawn.working.is_none())
            .build();
        let (me, id) = (self.me.clone(), action.id);
        button.connect_clicked(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.act(id);
            }
        });
        let line = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Start)
            .build();
        line.append(&button);
        if drawn.working == Some(action.id) {
            line.append(&adw::Spinner::new());
        }
        line
    }

    /// An action offered as a row: the token screen's own-server link, and Details' others.
    fn action_row(&self, action: &Action, drawn: &Drawn) -> adw::ButtonRow {
        let busy = drawn.working == Some(action.id)
            || (action.id == ActionId::OwnServer && drawn.getting_code);
        let row = adw::ButtonRow::builder()
            .title(glib::markup_escape_text(&action.label))
            .sensitive(drawn.working.is_none() && !drawn.getting_code)
            .build();
        if busy {
            row.set_end_icon_name(Some("content-loading-symbolic"));
        }
        let (me, id) = (self.me.clone(), action.id);
        row.connect_activated(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.act(id);
            }
        });
        row
    }

    fn change_row(&self, drawn: &Drawn) -> adw::ButtonRow {
        let row = adw::ButtonRow::builder()
            .title(words::CHANGE)
            .sensitive(drawn.options_read)
            .build();
        let me = self.me.clone();
        row.connect_activated(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.open_change();
            }
        });
        row
    }

    /// The command for a server of the owner's own, with its code inside, shown once and copied
    /// for this computer only.
    fn own_rows(&self, drawn: &Drawn) -> Vec<gtk::Widget> {
        let mut rows = Vec::new();
        if let Some((line, command)) = &drawn.code {
            rows.push(note_row(line).upcast());
            rows.push(self.command_row(command).upcast());
        }
        if let Some(refusal) = &drawn.code_refusal {
            rows.push(refused_row(refusal).upcast());
        }
        rows
    }

    fn command_row(&self, command: &str) -> adw::ActionRow {
        let shown = gtk::Label::builder()
            .label(command)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["monospace"])
            .build();
        let copy = gtk::Button::builder()
            .label(words::COPY_COMMAND)
            .valign(gtk::Align::Center)
            .build();
        let (board, mine, value) = (
            self.page.clipboard(),
            self.copied.clone(),
            command.to_owned(),
        );
        copy.connect_clicked(move |_| copy_secret(&board, &mine, &value));
        let row = adw::ActionRow::new();
        row.add_prefix(&shown);
        row.add_suffix(&copy);
        row
    }

    fn show_details(&self, drawn: &Drawn, screen: Option<&Screen>) {
        let shows = screen.is_some_and(Screen::has_details);
        self.w.details.set_visible(shows);
        for row in self.w.details_rows.borrow_mut().drain(..) {
            self.w.details_row.remove(&row);
        }
        let rows: Vec<gtk::Widget> = drawn
            .facts
            .iter()
            .map(|fact| self.fact_row(fact, drawn))
            .collect();
        for row in &rows {
            self.w.details_row.add_row(row);
        }
        *self.w.details_rows.borrow_mut() = rows;
    }

    fn fact_row(&self, fact: &Fact, drawn: &Drawn) -> gtk::Widget {
        match fact {
            Fact::Line(line) => note_row(line).upcast(),
            Fact::Value(label, value) => property_row(label, value).upcast(),
            Fact::Time(label, at) => property_row(label, &local_time(at)).upcast(),
            Fact::Bytes(label, bytes) => property_row(label, &glib::format_size(*bytes)).upcast(),
            Fact::Share(label, used) => property_row(label, &percent(*used)).upcast(),
            Fact::WaitsUntil(at) => note_row(&words::waits_until(&local_time(at))).upcast(),
            Fact::Seen(at) => note_row(&words::seen_line(&local_time(at))).upcast(),
            Fact::Item(item) => item_row(item),
            Fact::Act(action) => self.action_row(action, drawn).upcast(),
        }
    }

    /// The daemon's sections show while Details is open on a screen that has it.
    fn show_sections(&self) {
        let open = self.w.details.is_visible() && self.w.details_row.is_expanded();
        for group in self.sections.borrow().iter() {
            group.set_visible(open);
        }
    }

    /// The token field, the passphrase fields and their words, set on every draw: they stay
    /// for the pane's life so nothing typed in them is lost.
    fn render_fields(&self, drawn: &Drawn) {
        let model = self.model.borrow();
        let screen = drawn.page.clone().screen();
        let field = screen.as_ref().is_some_and(|s| s.token_field);
        let t = &self.w.token;
        t.group.set_visible(field);
        t.entry.set_visible(drawn.hetzner);
        t.newer.set_visible(!drawn.hetzner);
        t.entry.set_sensitive(!model.checking_token);
        t.checking.set_visible(model.checking_token);
        show_refusal(&t.refusal, model.token_refusal.as_deref());
        // How to get one is the field's: an engine without the provisioner gets neither.
        let steps = screen
            .as_ref()
            .map(|s| s.steps.as_slice())
            .unwrap_or_default();
        self.show_steps(if drawn.hetzner { steps } else { &[] });
        let p = &self.w.passphrase;
        p.group
            .set_visible(screen.as_ref().is_some_and(|s| s.asks_passphrase));
        p.set.set_sensitive(!model.setting_passphrase);
        p.spinner.set_visible(model.setting_passphrase);
        show_refusal(&p.refusal, model.passphrase_refusal.as_deref());
    }

    fn show_steps(&self, steps: &[String]) {
        let how = &self.w.token.how;
        how.set_visible(!steps.is_empty());
        let mut shown = self.w.token.steps.borrow_mut();
        if shown.0 == steps {
            return;
        }
        for row in shown.1.drain(..) {
            how.remove(&row);
        }
        how.remove(&self.w.token.open);
        let rows: Vec<gtk::Widget> = steps
            .iter()
            .enumerate()
            .map(|(at, step)| note_row(&format!("{}. {step}", at + 1)).upcast())
            .collect();
        for row in &rows {
            how.add_row(row);
        }
        how.add_row(&self.w.token.open);
        *shown = (steps.to_vec(), rows);
    }

    // ---- Dialogs -------------------------------------------------------------------------------

    /// The dialogs the model asks for: Replace, and the passphrase Move now needs.
    fn present_dialogs(&self) {
        if self.dialog.borrow().is_some() {
            return;
        }
        let (asks, replacing, options) = {
            let model = self.model.borrow();
            let options = model.loaded_options().cloned();
            (model.asks_move_passphrase, model.replacing, options)
        };
        if asks {
            self.present(self.passphrase_page(), None);
        } else if let (true, Some(options)) = (replacing, options) {
            let pick = self.model.borrow().pick();
            self.present(self.replace_page(&options, pick), Some(DIALOG_HEIGHT));
        }
    }

    /// A step the app keeps itself was taken. Replace reads what Hetzner sells before its
    /// dialog opens.
    async fn step_taken(&self) {
        let wants = {
            let model = self.model.borrow();
            model.replacing && model.loaded_options().is_none()
        };
        if wants {
            self.read_options().await;
        }
        let failed = match &self.model.borrow().options {
            Some(Err(sentence)) => Some(sentence.clone()),
            _ => None,
        };
        if let Some(sentence) = failed.filter(|_| self.model.borrow().replacing) {
            let mut model = self.model.borrow_mut();
            model.close_replace();
            model.refusal = Some(sentence);
        }
        self.render();
    }

    fn open_change(&self) {
        let Some(options) = self.model.borrow().loaded_options().cloned() else {
            return;
        };
        let pick = self.model.borrow().pick();
        let me = self.me.clone();
        let chosen: Chosen = Rc::new(move |pick, from| {
            if let Some(pane) = me.upgrade() {
                pane.model.borrow_mut().choose(pick);
                pane.render();
            }
            leave(from);
        });
        let body = offers(&options, pick.as_ref(), &chosen);
        let page = dialog_page(words::CHANGE_TITLE, &body, words::DONE, None);
        self.present(page, Some(DIALOG_HEIGHT));
    }

    fn replace_page(&self, options: &Options, pick: Option<Pick>) -> adw::NavigationPage {
        let choice = Rc::new(RefCell::new(pick.clone()));
        let keep = choice.clone();
        let chosen: Chosen = Rc::new(move |pick, _| *keep.borrow_mut() = Some(pick));
        let body = offers(options, pick.as_ref(), &chosen);
        let lead = adw::PreferencesGroup::builder()
            .description(words::REPLACE_BODY)
            .build();
        body.insert(&lead, 0);
        let create = gtk::Button::builder()
            .label(words::REPLACE_CREATE)
            .css_classes(["suggested-action"])
            .build();
        let me = self.me.clone();
        create.connect_clicked(move |button| {
            let (Some(pane), Some(pick)) = (me.upgrade(), choice.borrow().clone()) else {
                return;
            };
            let context = pane.model.borrow_mut().replace_with(pick);
            leave(button);
            if let Some(context) = context {
                pane.spawn(|pane| async move { pane.run(context).await });
            }
        });
        dialog_page(words::REPLACE_TITLE, &body, words::CANCEL, Some(&create))
    }

    fn passphrase_page(&self) -> adw::NavigationPage {
        let first = adw::PasswordEntryRow::builder()
            .title(words::PASSPHRASE)
            .build();
        let second = adw::PasswordEntryRow::builder()
            .title(words::PASSPHRASE_AGAIN)
            .build();
        let refusal = refused_row("");
        refusal.set_visible(false);
        let group = adw::PreferencesGroup::builder()
            .description(words::PASSPHRASE_MOVE_BODY)
            .build();
        for row in [
            first.upcast_ref::<gtk::Widget>(),
            second.upcast_ref(),
            refusal.upcast_ref(),
        ] {
            group.add(row);
        }
        let body = adw::PreferencesPage::new();
        body.add(&group);
        let go = gtk::Button::builder()
            .label(words::MOVE_NOW)
            .css_classes(["suggested-action"])
            .build();
        let me = self.me.clone();
        let (first, second, refusal) = (first.downgrade(), second.downgrade(), refusal.downgrade());
        go.connect_clicked(move |button| {
            let (Some(pane), Some(first), Some(second), Some(refusal)) = (
                me.upgrade(),
                first.upgrade(),
                second.upgrade(),
                refusal.upgrade(),
            ) else {
                return;
            };
            let context = pane
                .model
                .borrow_mut()
                .move_with_passphrase(&first.text(), &second.text());
            let Some(context) = context else {
                show_refusal(&refusal, pane.model.borrow().passphrase_refusal.as_deref());
                return;
            };
            leave(button);
            pane.spawn(|pane| async move { pane.run(context).await });
        });
        dialog_page(words::PASSPHRASE_TITLE, &body, words::CANCEL, Some(&go))
    }

    /// Presents one dialog at a time; closing it lets go of what the model asked it for.
    fn present(&self, page: adw::NavigationPage, height: Option<i32>) {
        let dialog = form_dialog(&page, DIALOG_WIDTH, height);
        let me = self.me.clone();
        dialog.connect_closed(move |_| {
            let Some(pane) = me.upgrade() else { return };
            pane.dialog.replace(None);
            let mut model = pane.model.borrow_mut();
            model.close_replace();
            model.dismiss_move_passphrase();
            drop(model);
            pane.render();
        });
        dialog.present(Some(&self.root));
        self.dialog.replace(Some(dialog));
    }
}

fn widgets() -> Widgets {
    let learn = gtk::Button::builder()
        .label(words::LEARN_MORE)
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .build();
    let screen = adw::PreferencesGroup::new();
    screen.set_header_suffix(Some(&learn));
    let details_row = adw::ExpanderRow::builder().title(words::DETAILS).build();
    let details = adw::PreferencesGroup::new();
    details.add(&details_row);
    Widgets {
        screen: Rows::new(screen),
        learn,
        token: token_field(),
        body: Rows::new(adw::PreferencesGroup::new()),
        action: Rows::new(adw::PreferencesGroup::new()),
        passphrase: passphrase_fields(),
        own: Rows::new(adw::PreferencesGroup::new()),
        details,
        details_row,
        details_rows: RefCell::default(),
    }
}

fn token_field() -> TokenField {
    let entry = adw::PasswordEntryRow::builder()
        .title(words::TOKEN_FIELD)
        .build();
    let checking = adw::ActionRow::builder()
        .title(words::TOKEN_CHECKING)
        .visible(false)
        .build();
    checking.add_suffix(&adw::Spinner::new());
    let refusal = refused_row("");
    let newer = note_row(DAEMON_TOO_OLD);
    let how = adw::ExpanderRow::builder()
        .title(words::HOW_TO_GET_ONE)
        .build();
    let open = adw::ButtonRow::builder().title(words::OPEN_HETZNER).build();
    how.add_row(&open);
    let group = adw::PreferencesGroup::builder().visible(false).build();
    for row in [
        entry.upcast_ref::<gtk::Widget>(),
        checking.upcast_ref(),
        refusal.upcast_ref(),
        newer.upcast_ref(),
        how.upcast_ref(),
    ] {
        group.add(row);
    }
    TokenField {
        group,
        entry,
        checking,
        refusal,
        newer,
        how,
        steps: RefCell::default(),
        open,
        length: Cell::new(0),
    }
}

fn passphrase_fields() -> PassphraseFields {
    let first = adw::PasswordEntryRow::builder()
        .title(words::PASSPHRASE)
        .build();
    let second = adw::PasswordEntryRow::builder()
        .title(words::PASSPHRASE_AGAIN)
        .build();
    // Not the default button: the screen's own action, Cancel, keeps Return.
    let set = gtk::Button::builder()
        .label(words::SET_PASSPHRASE)
        .valign(gtk::Align::Center)
        .build();
    let spinner = adw::Spinner::builder().visible(false).build();
    let setting = adw::ActionRow::new();
    setting.add_suffix(&spinner);
    setting.add_suffix(&set);
    let refusal = refused_row("");
    let group = adw::PreferencesGroup::builder()
        .title(words::PASSPHRASE_TITLE)
        .description(words::PASSPHRASE_BODY)
        .visible(false)
        .build();
    for row in [
        first.upcast_ref::<gtk::Widget>(),
        second.upcast_ref(),
        setting.upcast_ref(),
        refusal.upcast_ref(),
    ] {
        group.add(row);
    }
    PassphraseFields {
        group,
        first,
        second,
        set,
        spinner,
        refusal,
    }
}

/// The offers, by location, cheapest first, the one selected checked. `chosen` hears each pick
/// with the widget it came from.
fn offers(options: &Options, selected: Option<&Pick>, chosen: &Chosen) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let mut first: Option<gtk::CheckButton> = None;
    for location in &options.locations {
        let group = adw::PreferencesGroup::builder()
            .title(glib::markup_escape_text(&location.city))
            .build();
        for offer in options.offers.iter().filter(|o| o.location == location.id) {
            let pick = Pick {
                size: offer.size.clone(),
                location: offer.location,
            };
            let radio = gtk::CheckButton::builder()
                .active(selected == Some(&pick))
                .valign(gtk::Align::Center)
                .build();
            radio.set_group(first.as_ref());
            first.get_or_insert_with(|| radio.clone());
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&offer.description))
                .subtitle(glib::markup_escape_text(&words::shape(offer)))
                .build();
            row.add_prefix(&radio);
            row.add_suffix(&gtk::Label::new(Some(&words::monthly(
                offer,
                &options.currency,
            ))));
            row.set_activatable_widget(Some(&radio));
            let chosen = chosen.clone();
            radio.connect_toggled(move |radio| {
                if radio.is_active() {
                    chosen(pick.clone(), radio.upcast_ref());
                }
            });
            group.add(&row);
        }
        page.add(&group);
    }
    page
}

/// A dialog's page: its body, with the way out and, where there is one, its answer in the
/// header.
fn dialog_page(
    title: &str,
    body: &adw::PreferencesPage,
    out: &str,
    answer: Option<&gtk::Button>,
) -> adw::NavigationPage {
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    let close = gtk::Button::with_label(out);
    close.connect_clicked(leave);
    header.pack_start(&close);
    if let Some(answer) = answer {
        header.pack_end(answer);
    }
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(body));
    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
}

fn text(line: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(line)
        .wrap(true)
        .xalign(0.0)
        .build()
}

fn attention(line: &str) -> gtk::Label {
    let label = text(line);
    label.add_css_class("error");
    label
}

fn spinner_line(line: &str) -> gtk::Box {
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&adw::Spinner::new());
    row.append(&text(line));
    row
}

/// The bar that claims no percentage: it moves while it is on screen, and stops once drawn away.
fn pulsing_bar() -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::builder().pulse_step(0.05).build();
    let weak = bar.downgrade();
    glib::timeout_add_local(PULSE, move || match weak.upgrade() {
        Some(bar) if bar.parent().is_some() => {
            bar.pulse();
            glib::ControlFlow::Continue
        }
        _ => glib::ControlFlow::Break,
    });
    bar
}

fn note_row(line: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(line))
        .build()
}

fn refused_row(line: &str) -> adw::ActionRow {
    let row = note_row(line);
    row.add_css_class("error");
    row
}

fn show_refusal(row: &adw::ActionRow, refusal: Option<&str>) {
    row.set_visible(refusal.is_some());
    row.set_title(&glib::markup_escape_text(refusal.unwrap_or_default()));
}

fn property_row(label: &str, value: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(label))
        .subtitle(glib::markup_escape_text(value))
        .subtitle_selectable(true)
        .css_classes(["property"])
        .build()
}

/// One warning or block of the review; a block stands out.
fn item_row(item: &ReadinessItem) -> gtk::Widget {
    let row = note_row(&item.sentence);
    if item.status == ItemStatus::Block {
        row.add_css_class("error");
    }
    row.upcast()
}

/// The engine's RFC 3339 time in this computer's own style; the engine's words where it is in a
/// shape this app cannot read.
fn local_time(at: &str) -> String {
    let shown = glib::DateTime::from_iso8601(at, None)
        .and_then(|time| time.to_local())
        .and_then(|time| time.format("%-d %b %Y, %H:%M"));
    match shown {
        Ok(text) => text.into(),
        Err(e) => {
            glib::g_warning!("fermix", "could not show the time {at}: {e}");
            at.to_owned()
        }
    }
}

fn percent(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

fn unix_now() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970");
    i64::try_from(since.as_secs()).expect("the clock fits an i64")
}
