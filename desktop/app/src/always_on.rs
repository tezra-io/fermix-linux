//! The Always On pane (Always On setup §8.4): it draws the engine's summary and sends each press
//! as one `home.act`. Which screen shows, what a press collects and when to read are
//! `fermix_client::always_on`'s; this file draws the model and runs the calls it hands over, each
//! off the window's thread. It keeps only how it asks for a token and a passphrase, how it draws
//! a choice of size and place, and that Fermix restarts after its own Move now.
//!
//! The pane owns its daemon calls, as Integrations does. It reads `home.status` on no timer:
//! when it is shown, after each act, when the engine says the summary changed
//! (`home_status_changed`, which the chat wire hands to `home_status_changed` here), and when a
//! new engine answers while it shows; so while Always On is off and the pane is closed the app
//! asks nothing. The engine's notices are raised as desktop notifications, each once. The
//! `snapshots` section, which the daemon files under this pane, is added below by `SettingsPage`
//! and drawn by the one settings model; it shows while Details is open. The token field stays
//! for the pane's life, so a read never wipes a token being pasted.

use crate::daemon::Daemon;
use crate::dialogs::{form_dialog, leave};
use crate::phone::{copy_secret, withdraw};
use crate::status::{down_view, DownPage};
use adw::prelude::*;
use fermix_client::always_on::model::{Act, Dialog, Model, Press, Read};
use fermix_client::always_on::notices::Posted;
use fermix_client::always_on::{
    self as words, destructive, sentence, Fact, Page, Screen, TokenTarget,
};
use fermix_client::home::{
    Action, ActionId, HomeStatus, ItemStatus, Notice, OwnServer, ReadinessItem,
};
use fermix_client::management::{CallError, Management};
use fermix_client::provision::{Offer, Options, Pick};
use fermix_client::view::{daemon_problem, DAEMON_TOO_OLD};
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// A token is at least this long, so a change that adds this much at once was pasted.
const PASTED: usize = 16;
/// How often the bar that claims no percentage moves.
const PULSE: Duration = Duration::from_millis(120);
const DIALOG_WIDTH: i32 = 480;
const DIALOG_HEIGHT: i32 = 560;
/// The most reads one run of `read_due` makes: what is still due after it waits for the next
/// trigger. A pane's own reads are at most two; the rest are notices that came meanwhile.
const READS_AT_ONCE: usize = 32;

/// Hears each size and place picked in a list of offers, with the widget it came from.
type Chosen = Rc<dyn Fn(Pick, &gtk::Widget)>;

/// One read: the methods `hello` lists, and `home.status` where it lists what the pane needs.
struct HomeRead {
    methods: Vec<String>,
    status: Option<Result<HomeStatus, CallError>>,
}

fn read_home(m: &Management) -> Result<HomeRead, CallError> {
    let methods = m.hello()?.capabilities.methods;
    let serves = words::NEEDED
        .iter()
        .all(|needed| methods.iter().any(|name| name == needed));
    let status = serves.then(|| m.home_status());
    Ok(HomeRead { methods, status })
}

/// What the page was drawn from; a page equal to it draws nothing again.
#[derive(Clone, PartialEq)]
struct Drawn {
    page: Page,
    note: Option<String>,
    working: Option<ActionId>,
    refusal: Option<String>,
    /// Whose token the field takes, while one is asked for.
    target: Option<TokenTarget>,
    options: Option<Result<Options, String>>,
    chosen: Option<Pick>,
    handout: Option<OwnServer>,
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

/// The paste field and what sits with it, in the words of the provider it is for.
struct TokenField {
    group: adw::PreferencesGroup,
    /// The provider's own sentence, where the screen's says something else.
    said: adw::ActionRow,
    entry: adw::PasswordEntryRow,
    checking: adw::ActionRow,
    refusal: adw::ActionRow,
    how: adw::ExpanderRow,
    steps: RefCell<(Vec<String>, Vec<gtk::Widget>)>,
    /// Open the provider's console, after the steps.
    open: adw::ButtonRow,
    /// The Always On docs page at the provider's part.
    learn: adw::ButtonRow,
    length: Cell<usize>,
}

struct Widgets {
    /// The sentence, the step in hand, the bar and a note on the last read.
    screen: Rows,
    /// Where the engine lists more than one provider: their names, to pick one.
    pick: Rows,
    token: TokenField,
    /// The size and place the main action creates, with Change.
    choice: Rows,
    /// The review's warnings and blocks.
    review: Rows,
    /// The one main action, and what the daemon said of the last act.
    action: Rows,
    /// The other actions the screen offers.
    others: Rows,
    /// The own server's command and code.
    own: Rows,
    details: adw::PreferencesGroup,
    details_row: adw::ExpanderRow,
    details_rows: RefCell<Vec<gtk::Widget>>,
}

pub struct AlwaysOnPane {
    me: Weak<AlwaysOnPane>,
    /// Where the raised notices' ids are kept across launches.
    notices_file: PathBuf,
    /// The page, or the down page while Fermix on this computer was never read.
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
    /// The reads due are being run, one after another, so they land in order.
    draining: Cell<bool>,
    /// What this pane copied, taken back when the pane goes.
    copied: Rc<RefCell<Option<gdk::ContentProvider>>>,
    /// The dialog up, and whether it is one the model asked for (else Change's own).
    dialog: RefCell<Option<(adw::Dialog, bool)>>,
}

impl AlwaysOnPane {
    /// `notify` raises one desktop notification; `notices_file` keeps which were raised, read
    /// here once.
    pub fn new(
        daemon: Daemon,
        notify: Rc<dyn Fn(&Notice)>,
        notices_file: PathBuf,
    ) -> Rc<AlwaysOnPane> {
        let posted = Posted::load(&notices_file).unwrap_or_else(|e| {
            glib::g_warning!("fermix", "Always On's raised notices were not read: {e}");
            Posted::default()
        });
        let page = adw::PreferencesPage::new();
        let w = widgets();
        for group in [
            &w.screen.group,
            &w.pick.group,
            &w.token.group,
            &w.choice.group,
            &w.review.group,
            &w.action.group,
            &w.others.group,
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
            notices_file,
            root,
            page,
            down,
            daemon,
            notify,
            model: RefCell::new(Model::with_posted(posted)),
            w,
            drawn: RefCell::default(),
            sections: RefCell::default(),
            draining: Cell::new(false),
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

    /// The engine said Always On's summary changed (companion `home_status_changed`), or the
    /// chat was refused because this desktop's server is out of reach or no longer knows it
    /// (`primary_unreachable`, `desktop_revoked`), where no notice can arrive: one read of it.
    #[expect(
        dead_code,
        reason = "the chat wire calls it once it speaks companion protocol 6"
    )]
    pub fn home_status_changed(&self) {
        self.model.borrow_mut().home_status_changed();
        self.read_due();
    }

    /// A new engine process answers here: it is read again only while the pane shows, and it
    /// ends the restart after this app's own Move now.
    pub fn engine_restarted(&self) {
        self.model.borrow_mut().engine_restarted();
        self.read_due();
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
        self.w.details_row.connect_expanded_notify(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.show_sections();
            }
        });
        self.wire_token();
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
                pane.open_provider(|p| p.console_url.clone());
            }
        });
        let me = self.me.clone();
        self.w.token.learn.connect_activated(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.open_provider(|p| p.docs_url.clone());
            }
        });
    }

    // ---- Coming and going ----------------------------------------------------------------

    fn shown(&self) {
        self.model.borrow_mut().pane_shown();
        self.read_due();
    }

    /// The pane went: the code it showed, and its copy on the clipboard, go with it.
    fn hidden(&self) {
        self.model.borrow_mut().pane_hidden();
        withdraw(&self.page.clipboard(), &self.copied);
        self.render();
    }

    // ---- Reads -----------------------------------------------------------------------------

    /// Runs the reads the model says are due, one after another, until none is; a run already
    /// going takes up what is asked meanwhile.
    fn read_due(&self) {
        if self.draining.replace(true) {
            return;
        }
        self.spawn(|pane| async move {
            let mut made = 0;
            while made < READS_AT_ONCE {
                let next = pane.model.borrow_mut().next_read();
                let Some(read) = next else { break };
                pane.perform_read(read).await;
                made += 1;
            }
            pane.draining.set(false);
            if made == READS_AT_ONCE {
                glib::g_warning!(
                    "fermix",
                    "Always On made {READS_AT_ONCE} reads in one run; the rest wait for the next"
                );
            }
        });
    }

    /// One read, and what it answered handed to the model; a notice the summary carries for the
    /// first time is raised.
    async fn perform_read(&self, read: Read) {
        match read {
            Read::Status => {
                let answer = self.daemon.call(read_home).await;
                let notice = self.apply(answer);
                self.raise(notice);
            }
            Read::Options => {
                let answer = self.daemon.call(|m| m.provision_options()).await;
                let answer = answer.map_err(|e| {
                    glib::g_warning!("fermix", "home.provision.options was not read: {e:?}");
                    sentence(&e)
                });
                self.model.borrow_mut().options_read(answer);
            }
        }
        self.render();
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
            Ok(status) => model.answered(status),
            Err(e) => {
                glib::g_debug!("fermix", "home.status was not read: {e:?}");
                model.unanswered(daemon_problem(&e));
                None
            }
        }
    }

    // ---- Presses -----------------------------------------------------------------------------

    /// One action the screen offers was pressed.
    fn act(&self, action: &Action) {
        let press = self.model.borrow_mut().press(action);
        self.render();
        self.spawn(|pane| async move { pane.pressed(press).await });
    }

    async fn pressed(self: Rc<Self>, press: Press) {
        match press {
            Press::Drawn => self.render(),
            Press::Read => self.read_due(),
            Press::Open(url) => self.open(&url).await,
            Press::Update => self.activate("win.updates-open"),
            Press::Run(act) => self.run(act).await,
            Press::Ignored => {}
        }
    }

    /// The one `home.act`, off the window's thread: its answer is drawn, the summary read
    /// again, and a notice it carries for the first time raised.
    async fn run(&self, act: Act) {
        self.render();
        let id = act.action;
        let answer = self
            .daemon
            .call(move |m| m.home_act(act.action, &act.key, &act.input))
            .await;
        if let Err(e) = &answer {
            glib::g_debug!("fermix", "home.act {id:?} did not run: {e:?}");
        }
        let took_token = id == ActionId::PasteToken && answer.is_ok();
        let notice = self.model.borrow_mut().acted(answer);
        if took_token {
            self.w.token.entry.set_text("");
        }
        self.raise(notice);
        self.render();
        self.read_due();
    }

    /// Raises a notice seen for the first time, and keeps that it was.
    fn raise(&self, notice: Option<Notice>) {
        let Some(notice) = notice else { return };
        (self.notify)(&notice);
        let saved = self.model.borrow().posted().save(&self.notices_file);
        if let Err(e) = saved {
            glib::g_warning!(
                "fermix",
                "Always On's raised notices were not saved to {}: {e}",
                self.notices_file.display()
            );
        }
    }

    /// A pasted token, or Return in the field: the model hands over its act.
    fn check_token(&self) {
        let text = self.w.token.entry.text();
        let press = self.model.borrow_mut().token_entered(&text);
        self.render();
        self.spawn(|pane| async move { pane.pressed(press).await });
    }

    /// Opens a page of the provider the field is for.
    fn open_provider(&self, page: impl Fn(&fermix_client::home::Provider) -> String) {
        let Some(TokenTarget::Provider(provider)) = self.model.borrow().token_target() else {
            return;
        };
        let url = page(&provider);
        self.spawn(|pane| async move { pane.open(&url).await });
    }

    fn pick_provider(&self, id: &str) {
        self.model.borrow_mut().pick_provider(id);
        self.render();
        self.read_due();
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
        self.render_token(&drawn);
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
        Drawn {
            page: model.page(),
            note: model.read_note(),
            working: model.working,
            refusal: model.refusal.clone(),
            target: model.token_target(),
            options: model.options().cloned(),
            chosen: model.chosen().cloned(),
            handout: model.handout.clone(),
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
        self.w.screen.replace(screen_rows(drawn, screen.as_ref()));
        self.w.pick.replace(self.pick_rows(drawn));
        let review = screen.as_ref().map(|s| s.readiness.as_slice());
        self.w
            .review
            .replace(review.unwrap_or_default().iter().map(item_row).collect());
        let choice = if screen.as_ref().is_some_and(|s| s.choice) {
            self.choice_rows(drawn)
        } else {
            Vec::new()
        };
        self.w.choice.replace(choice);
        let action = screen.as_ref().and_then(|s| s.action.clone());
        self.w
            .action
            .replace(self.action_rows(drawn, action.as_ref()));
        let others = screen.as_ref().map(|s| s.others.as_slice());
        let others = others.unwrap_or_default().iter();
        self.w
            .others
            .replace(others.map(|a| self.action_row(a, drawn).upcast()).collect());
        self.w.own.replace(self.own_rows(drawn));
        self.show_details(screen.as_ref());
    }

    /// Where the engine lists more than one provider and none is picked: their names, one of
    /// which the token field is then for.
    fn pick_rows(&self, drawn: &Drawn) -> Vec<gtk::Widget> {
        let Some(TokenTarget::Pick(providers)) = &drawn.target else {
            return Vec::new();
        };
        let mut rows = vec![note_row(words::PROVIDER_CHOOSE).upcast::<gtk::Widget>()];
        for provider in providers {
            let row = adw::ButtonRow::builder()
                .title(glib::markup_escape_text(&provider.name))
                .build();
            let (me, id) = (self.me.clone(), provider.id.clone());
            row.connect_activated(move |_| {
                if let Some(pane) = me.upgrade() {
                    pane.pick_provider(&id);
                }
            });
            rows.push(row.upcast());
        }
        rows
    }

    /// The choice the main action makes: the engine's recommendation, or the owner's own offer
    /// in the engine's words and its city, with Change.
    fn choice_rows(&self, drawn: &Drawn) -> Vec<gtk::Widget> {
        let options = match &drawn.options {
            None => return vec![spinner_row(words::READING).upcast()],
            Some(Err(why)) => return vec![refused_row(why).upcast()],
            Some(Ok(options)) => options,
        };
        let picked = drawn.chosen.as_ref().and_then(|pick| options.offer(pick));
        let said: gtk::Widget = match picked {
            Some(offer) => {
                let row = offer_row(offer);
                let city = options.city(&offer.location).unwrap_or(&offer.location);
                row.add_suffix(&gtk::Label::new(Some(city)));
                row.upcast()
            }
            None => note_row(&options.sentence).upcast(),
        };
        let change = adw::ButtonRow::builder().title(words::CHANGE).build();
        let me = self.me.clone();
        change.connect_activated(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.open_change();
            }
        });
        vec![said, change.upcast()]
    }

    /// The one main action, its spinner while it runs, and the daemon's refusal under it.
    fn action_rows(&self, drawn: &Drawn, action: Option<&Action>) -> Vec<gtk::Widget> {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        if let Some(action) = action {
            column.append(&self.main_button(action, drawn));
        }
        if let Some(refusal) = &drawn.refusal {
            column.append(&attention(refusal));
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
        let (me, pressed) = (self.me.clone(), action.clone());
        button.connect_clicked(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.act(&pressed);
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

    /// Another action the screen offers, as a row.
    fn action_row(&self, action: &Action, drawn: &Drawn) -> adw::ButtonRow {
        let row = adw::ButtonRow::builder()
            .title(glib::markup_escape_text(&action.label))
            .sensitive(drawn.working.is_none())
            .build();
        if drawn.working == Some(action.id) {
            row.set_end_icon_name(Some("content-loading-symbolic"));
        }
        let (me, pressed) = (self.me.clone(), action.clone());
        row.connect_activated(move |_| {
            if let Some(pane) = me.upgrade() {
                pane.act(&pressed);
            }
        });
        row
    }

    /// The command for a server of the owner's own and the code it asks for, shown once and
    /// copied for this computer only.
    fn own_rows(&self, drawn: &Drawn) -> Vec<gtk::Widget> {
        let Some(handout) = &drawn.handout else {
            return Vec::new();
        };
        let line = words::own_server_line(&local_time(&handout.expires_at));
        vec![
            note_row(&line).upcast(),
            self.copy_row(&handout.command, words::COPY_COMMAND)
                .upcast(),
            self.copy_row(handout.code(), words::COPY_CODE).upcast(),
        ]
    }

    fn copy_row(&self, text: &str, label: &str) -> adw::ActionRow {
        let shown = gtk::Label::builder()
            .label(text)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["monospace"])
            .build();
        let copy = gtk::Button::builder()
            .label(label)
            .valign(gtk::Align::Center)
            .build();
        let (board, mine, value) = (self.page.clipboard(), self.copied.clone(), text.to_owned());
        copy.connect_clicked(move |_| copy_secret(&board, &mine, &value));
        let row = adw::ActionRow::new();
        row.add_prefix(&shown);
        row.add_suffix(&copy);
        row
    }

    fn show_details(&self, screen: Option<&Screen>) {
        let facts = screen.map(|s| s.details.as_slice()).unwrap_or_default();
        self.w.details.set_visible(!facts.is_empty());
        for row in self.w.details_rows.borrow_mut().drain(..) {
            self.w.details_row.remove(&row);
        }
        let rows: Vec<gtk::Widget> = facts.iter().map(fact_row).collect();
        for row in &rows {
            self.w.details_row.add_row(row);
        }
        *self.w.details_rows.borrow_mut() = rows;
    }

    /// The daemon's sections show while Details is open on a screen that has it.
    fn show_sections(&self) {
        let open = self.w.details.is_visible() && self.w.details_row.is_expanded();
        for group in self.sections.borrow().iter() {
            group.set_visible(open);
        }
    }

    /// The token field and its words, set on every draw: it stays for the pane's life so
    /// nothing pasted in it is lost. The field, its label, the check's words, How to get one,
    /// the console and Learn more are the provider's the field is for.
    fn render_token(&self, drawn: &Drawn) {
        let t = &self.w.token;
        let provider = match &drawn.target {
            Some(TokenTarget::Provider(provider)) => Some(provider),
            _ => None,
        };
        t.group.set_visible(provider.is_some());
        let Some(provider) = provider else {
            return;
        };
        let screen = drawn.page.clone().screen();
        let repeats = screen.is_some_and(|s| s.sentence == provider.sentence);
        t.said.set_visible(!repeats);
        t.said
            .set_title(&glib::markup_escape_text(&provider.sentence));
        t.entry
            .set_title(&glib::markup_escape_text(&provider.token_label));
        let checking = drawn.working == Some(ActionId::PasteToken);
        t.entry.set_sensitive(!checking);
        t.checking.set_visible(checking);
        t.checking
            .set_title(&glib::markup_escape_text(&words::token_checking(
                &provider.name,
            )));
        let refused = self.model.borrow().token_refusal.clone();
        show_refusal(&t.refusal, refused.as_deref());
        t.open
            .set_title(&glib::markup_escape_text(&words::console_label(
                &provider.name,
            )));
        self.show_steps(&provider.steps);
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

    /// The dialog the model asks for; one it no longer asks for closes.
    fn present_dialogs(&self) {
        let wanted = self.model.borrow().dialog();
        let open = self.dialog.borrow().clone();
        if let Some((dialog, asked)) = open {
            if asked && wanted.is_none() {
                dialog.close();
            }
            return;
        }
        match wanted {
            Some(Dialog::Choice(action)) => {
                let page = self.choice_page(&action);
                self.present(page, Some(DIALOG_HEIGHT), true);
            }
            Some(Dialog::Passphrase { action, twice }) => {
                let page = self.passphrase_page(&action, twice);
                self.present(page, None, true);
            }
            Some(Dialog::Confirm(action)) => self.present_confirm(&action),
            None => {}
        }
    }

    /// Change, on a screen that draws the choice: the offers, the one selected checked.
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
        let body = offers(Some(&options), pick.as_ref(), &chosen);
        let page = dialog_page(words::CHANGE_TITLE, &body, words::DONE, None);
        self.present(page, Some(DIALOG_HEIGHT), false);
    }

    /// An action's choice in its own dialog, with its confirm: answering it is accepting it.
    fn choice_page(&self, action: &Action) -> adw::NavigationPage {
        let (options, pick) = {
            let model = self.model.borrow();
            (model.loaded_options().cloned(), model.pick())
        };
        let me = self.me.clone();
        let chosen: Chosen = Rc::new(move |pick, _| {
            if let Some(pane) = me.upgrade() {
                pane.model.borrow_mut().choose(pick);
            }
        });
        let body = offers(options.as_ref(), pick.as_ref(), &chosen);
        if let Some(confirm) = &action.confirm {
            let lead = adw::PreferencesGroup::builder()
                .description(confirm)
                .build();
            body.insert(&lead, 0);
        }
        let answer = gtk::Button::builder()
            .label(&action.label)
            .css_classes(["suggested-action"])
            .build();
        let me = self.me.clone();
        answer.connect_clicked(move |button| {
            let Some(pane) = me.upgrade() else { return };
            let press = pane.model.borrow_mut().choice_answered();
            leave(button);
            pane.spawn(|pane| async move { pane.pressed(press).await });
        });
        dialog_page(&action.label, &body, words::CANCEL, Some(&answer))
    }

    /// The recovery passphrase, twice where none is set, under the action's confirm: answering
    /// it is accepting it.
    fn passphrase_page(&self, action: &Action, twice: bool) -> adw::NavigationPage {
        let first = adw::PasswordEntryRow::builder()
            .title(words::PASSPHRASE)
            .build();
        let second = adw::PasswordEntryRow::builder()
            .title(words::PASSPHRASE_AGAIN)
            .visible(twice)
            .build();
        let refusal = refused_row("");
        refusal.set_visible(false);
        let group = adw::PreferencesGroup::builder()
            .description(passphrase_said(action, twice))
            .build();
        for row in [
            first.upcast_ref::<gtk::Widget>(),
            second.upcast_ref(),
            refusal.upcast_ref(),
        ] {
            group.add(row);
        }
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let answer = gtk::Button::builder()
            .label(&action.label)
            .css_classes(["suggested-action"])
            .build();
        let me = self.me.clone();
        let (first, second, refusal) = (first.downgrade(), second.downgrade(), refusal.downgrade());
        answer.connect_clicked(move |button| {
            let (Some(pane), Some(first), Some(second), Some(refusal)) = (
                me.upgrade(),
                first.upgrade(),
                second.upgrade(),
                refusal.upgrade(),
            ) else {
                return;
            };
            let again = twice.then(|| second.text());
            let press = pane
                .model
                .borrow_mut()
                .passphrase_entered(&first.text(), again.as_deref());
            if !matches!(press, Press::Run(_)) {
                show_refusal(&refusal, pane.model.borrow().passphrase_refusal.as_deref());
                return;
            }
            leave(button);
            pane.spawn(|pane| async move { pane.pressed(press).await });
        });
        dialog_page(words::PASSPHRASE_TITLE, &page, words::CANCEL, Some(&answer))
    }

    /// The action's confirm, under its label: a step that deletes never takes Return for yes.
    fn present_confirm(&self, action: &Action) {
        let confirm = action.confirm.as_deref().unwrap_or_default();
        let alert = adw::AlertDialog::new(Some(&action.label), Some(confirm));
        alert.add_responses(&[("cancel", words::CANCEL), ("ok", action.label.as_str())]);
        let deletes = destructive(action.id);
        let appearance = if deletes {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        };
        alert.set_response_appearance("ok", appearance);
        alert.set_default_response(Some(if deletes { "cancel" } else { "ok" }));
        alert.set_close_response("cancel");
        let me = self.me.clone();
        alert.connect_response(Some("ok"), move |_, _| {
            let Some(pane) = me.upgrade() else { return };
            let press = pane.model.borrow_mut().confirmed();
            pane.spawn(|pane| async move { pane.pressed(press).await });
        });
        self.watch(alert.upcast_ref(), true);
        alert.present(Some(&self.root));
    }

    /// Presents one dialog at a time.
    fn present(&self, page: adw::NavigationPage, height: Option<i32>, asked: bool) {
        let dialog = form_dialog(&page, DIALOG_WIDTH, height);
        self.watch(&dialog, asked);
        dialog.present(Some(&self.root));
    }

    /// Keeps `dialog` as the one up; closing it lets go of what the model asked it for.
    fn watch(&self, dialog: &adw::Dialog, asked: bool) {
        let me = self.me.clone();
        dialog.connect_closed(move |_| {
            let Some(pane) = me.upgrade() else { return };
            pane.dialog.replace(None);
            pane.model.borrow_mut().dismissed();
            pane.render();
        });
        self.dialog.replace(Some((dialog.clone(), asked)));
    }
}

fn widgets() -> Widgets {
    let details_row = adw::ExpanderRow::builder().title(words::DETAILS).build();
    let details = adw::PreferencesGroup::new();
    details.add(&details_row);
    Widgets {
        screen: Rows::new(adw::PreferencesGroup::new()),
        pick: Rows::new(adw::PreferencesGroup::new()),
        token: token_field(),
        choice: Rows::new(adw::PreferencesGroup::new()),
        review: Rows::new(adw::PreferencesGroup::new()),
        action: Rows::new(adw::PreferencesGroup::new()),
        others: Rows::new(adw::PreferencesGroup::new()),
        own: Rows::new(adw::PreferencesGroup::new()),
        details,
        details_row,
        details_rows: RefCell::default(),
    }
}

/// The paste field and what sits with it; their words are set on each draw, from the provider
/// the field is for.
fn token_field() -> TokenField {
    let said = note_row("");
    let entry = adw::PasswordEntryRow::new();
    let checking = adw::ActionRow::builder().visible(false).build();
    checking.add_suffix(&adw::Spinner::new());
    let refusal = refused_row("");
    let how = adw::ExpanderRow::builder()
        .title(words::HOW_TO_GET_ONE)
        .build();
    let open = adw::ButtonRow::new();
    how.add_row(&open);
    let learn = adw::ButtonRow::builder().title(words::LEARN_MORE).build();
    let group = adw::PreferencesGroup::builder().visible(false).build();
    for row in [
        said.upcast_ref::<gtk::Widget>(),
        entry.upcast_ref(),
        checking.upcast_ref(),
        refusal.upcast_ref(),
        how.upcast_ref(),
        learn.upcast_ref(),
    ] {
        group.add(row);
    }
    TokenField {
        group,
        said,
        entry,
        checking,
        refusal,
        how,
        steps: RefCell::default(),
        open,
        learn,
        length: Cell::new(0),
    }
}

/// The sentence, the step in hand, the bar and a note on the last read; a spinner while the
/// first read is out, and the update sentence for an engine that does not serve the pane.
fn screen_rows(drawn: &Drawn, screen: Option<&Screen>) -> Vec<gtk::Widget> {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    match (&drawn.page, screen) {
        (Page::Reading, _) => column.append(&spinner_line(words::READING)),
        (Page::NeedsUpdate, _) => column.append(&text(DAEMON_TOO_OLD)),
        (_, Some(screen)) => {
            column.append(&text(&screen.sentence));
            if let Some(progress) = &screen.progress {
                column.append(&dim(progress));
            }
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

/// Over the passphrase entries: the action's confirm, which answering accepts, then what the
/// passphrase is for.
fn passphrase_said(action: &Action, twice: bool) -> String {
    let body = if twice {
        words::PASSPHRASE_NEW_BODY
    } else {
        words::PASSPHRASE_BODY
    };
    match &action.confirm {
        Some(confirm) => format!("{confirm}\n\n{body}"),
        None => body.to_owned(),
    }
}

/// The offers grouped by city, cheapest first, the one selected checked; none where they could
/// not be read. `chosen` hears each pick with the widget it came from.
fn offers(
    options: Option<&Options>,
    selected: Option<&Pick>,
    chosen: &Chosen,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let Some(options) = options else {
        return page;
    };
    let mut first: Option<gtk::CheckButton> = None;
    for (city, here) in words::by_city(options) {
        let group = adw::PreferencesGroup::builder()
            .title(glib::markup_escape_text(city))
            .build();
        for offer in here {
            let pick = Pick {
                size: offer.size.clone(),
                location: offer.location.clone(),
            };
            let radio = gtk::CheckButton::builder()
                .active(selected == Some(&pick))
                .valign(gtk::Align::Center)
                .build();
            radio.set_group(first.as_ref());
            first.get_or_insert_with(|| radio.clone());
            let row = offer_row(offer);
            row.add_prefix(&radio);
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

/// One offer as the engine words it: its label, and its price under it.
fn offer_row(offer: &Offer) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(&offer.label))
        .subtitle(glib::markup_escape_text(&offer.price))
        .build()
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

fn fact_row(fact: &Fact) -> gtk::Widget {
    match fact {
        Fact::Sentence(line) => note_row(line).upcast(),
        Fact::Line(line) => property_row(&line.label, &line.value).upcast(),
    }
}

fn text(line: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(line)
        .wrap(true)
        .xalign(0.0)
        .build()
}

fn dim(line: &str) -> gtk::Label {
    let label = text(line);
    label.add_css_class("dim-label");
    label
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

fn spinner_row(line: &str) -> adw::ActionRow {
    let row = note_row(line);
    row.add_suffix(&adw::Spinner::new());
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
