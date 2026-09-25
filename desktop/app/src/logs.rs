//! Logs: the daemon's log, newest at the bottom, read through `logs.query` every
//! 2 s while the page is shown. The app never opens the log file. Messages hold
//! home paths: they are shown as they are and never logged.

use crate::daemon::Daemon;
use adw::prelude::*;
use fermix_client::logs::{
    copy_text, emphasis, search_term, shown_time, Emphasis, Filter, Level, LogEntry, LogLines,
    LogPage, Merged, MAX_LINES, ROTATED,
};
use fermix_client::management::CallError;
use fermix_client::view::{daemon_problem, DaemonProblem};
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);
/// Past this distance from the bottom, new lines no longer pull the view down.
const STICK_DISTANCE: f64 = 48.0;
/// Pages "Load older" may follow in one click past pages that brought only
/// lines already held (lines appended since its cursor shift the window).
const OLDER_HOPS: u32 = 5;
const WHERE_LOGS_ARE: &str = "Fermix writes its log to ~/.fermix/logs/fermix.log. \
    To see what the service printed, run this in a terminal:";
const JOURNAL_COMMAND: &str = "journalctl --user -u fermix";

enum Notice {
    /// The daemon's refusal; the next answer clears it.
    Refused(String),
    /// Something the page did or found; the reader's next action clears it.
    Said(String),
}

#[derive(Default)]
struct Feed {
    filter: Filter,
    /// `None` until the first page for this filter lands.
    lines: Option<LogLines>,
    /// Bumped when the filter changes; an answer for an older filter is dropped.
    generation: u64,
    /// The generation a newest-page read is on the wire for.
    polling: Option<u64>,
    loading_older: bool,
    notice: Option<Notice>,
    /// Whether the down page is showing, so a lost daemon is logged once.
    down: bool,
}

struct Controls {
    older: gtk::Button,
    level: gtk::DropDown,
    search: gtk::SearchEntry,
    copy: gtk::Button,
}

pub struct LogsPage {
    pub root: gtk::Widget,
    stack: gtk::Stack,
    body: gtk::Stack,
    empty: adw::StatusPage,
    down: adw::StatusPage,
    controls: Controls,
    note: gtk::Label,
    store: gio::ListStore,
    list: gtk::ListView,
    /// Whether the reader is at the bottom, so new lines keep the newest in view.
    stuck: Rc<Cell<bool>>,
    daemon: Daemon,
    feed: RefCell<Feed>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl LogsPage {
    pub fn new(daemon: Daemon) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let (scroller, list) = log_list(&store);
        let stuck = watch_bottom(&scroller);
        let empty = adw::StatusPage::builder()
            .icon_name("text-x-generic-symbolic")
            .build();
        let body = gtk::Stack::builder().vexpand(true).build();
        body.add_named(&scroller, Some("list"));
        body.add_named(&empty, Some("empty"));
        let controls = controls();
        let note = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(6)
            .css_classes(["dim-label"])
            .visible(false)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        content.append(&controls_bar(&controls));
        content.append(&note);
        content.append(&body);
        let down = down_page();
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&content, Some("logs"));
        stack.add_named(&down, Some("down"));
        let page = Rc::new(LogsPage {
            root: stack.clone().upcast(),
            stack,
            body,
            empty,
            down,
            controls,
            note,
            store,
            list,
            stuck,
            daemon,
            feed: RefCell::default(),
            timer: RefCell::default(),
        });
        page.wire_controls();
        page.render();
        page
    }

    /// Starts the 2 s poll and reads now. A second call while shown changes nothing.
    pub fn shown(self: &Rc<Self>) {
        if self.timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local(POLL, move || {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            page.poll();
            glib::ControlFlow::Continue
        });
        self.timer.replace(Some(timer));
        self.poll();
    }

    /// Stops the poll. Calling it while hidden changes nothing.
    pub fn hidden(&self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }

    fn wire_controls(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.controls.level.connect_selected_notify(move |_| {
            if let Some(page) = weak.upgrade() {
                page.filter_changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.controls.search.connect_search_changed(move |_| {
            if let Some(page) = weak.upgrade() {
                page.filter_changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.controls.older.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                glib::spawn_future_local(page.load_older());
            }
        });
        let weak = Rc::downgrade(self);
        self.controls.copy.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.copy_visible();
            }
        });
    }

    /// A new filter starts from its own newest page: a cursor belongs to its query.
    fn filter_changed(self: &Rc<Self>) {
        let search = match search_term(&self.controls.search.text()) {
            Ok(search) => search,
            Err(sentence) => return self.say(sentence),
        };
        let filter = Filter {
            level: level_at(self.controls.level.selected()),
            search,
        };
        let mut feed = self.feed.borrow_mut();
        feed.notice = None;
        if feed.filter == filter {
            drop(feed);
            return self.render();
        }
        feed.filter = filter;
        feed.lines = None;
        feed.generation += 1;
        drop(feed);
        self.store.remove_all();
        self.render();
        self.poll();
    }

    /// Reads the newest page for the current filter, unless one is on the wire.
    fn poll(self: &Rc<Self>) {
        let mut feed = self.feed.borrow_mut();
        if feed.polling == Some(feed.generation) {
            return;
        }
        feed.polling = Some(feed.generation);
        let (generation, filter) = (feed.generation, feed.filter.clone());
        drop(feed);
        let page = self.clone();
        glib::spawn_future_local(async move {
            let answer = page.daemon.call(move |m| m.logs_query(&filter, None)).await;
            page.newest_landed(generation, answer);
        });
    }

    fn newest_landed(self: &Rc<Self>, generation: u64, answer: Result<LogPage, CallError>) {
        let mut feed = self.feed.borrow_mut();
        if feed.polling == Some(generation) {
            feed.polling = None;
        }
        if feed.generation != generation {
            return;
        }
        drop(feed);
        let newest = match answer {
            Ok(newest) => newest,
            Err(e) => return self.failed(e),
        };
        self.came_back();
        let merged = self.take_newest(newest);
        let objects = boxed(merged.added);
        let dropped = u32::try_from(merged.dropped).expect("the view holds at most MAX_LINES");
        self.store.splice(0, dropped, &[] as &[glib::Object]);
        self.store.extend_from_slice(&objects);
        self.follow_tail();
        self.render();
    }

    /// Scrolls to the newest line while the reader is at the bottom. A list
    /// view re-anchors after rows change, so the adjustment alone cannot hold it.
    fn follow_tail(&self) {
        let count = self.store.n_items();
        if self.stuck.get() && count > 0 {
            self.list
                .scroll_to(count - 1, gtk::ListScrollFlags::NONE, None);
        }
    }

    /// The first page for this filter, or a poll appended to the lines held.
    fn take_newest(&self, newest: LogPage) -> Merged {
        let mut feed = self.feed.borrow_mut();
        if matches!(feed.notice, Some(Notice::Refused(_))) {
            feed.notice = None;
        }
        if let Some(lines) = feed.lines.as_mut() {
            return lines.append_newer(newest);
        }
        assert_eq!(
            self.store.n_items(),
            0,
            "a first page lands on an empty list"
        );
        let lines = LogLines::new(newest, MAX_LINES);
        let added = lines.entries().to_vec();
        feed.lines = Some(lines);
        Merged { dropped: 0, added }
    }

    /// Follows the older cursor, and on past pages that brought nothing new.
    async fn load_older(self: Rc<Self>) {
        let Some(generation) = self.begin_older() else {
            return;
        };
        for _ in 0..OLDER_HOPS {
            let Some((filter, cursor)) = self.older_query(generation) else {
                break;
            };
            let answer = self
                .daemon
                .call(move |m| m.logs_query(&filter, Some(&cursor)))
                .await;
            if self.older_landed(generation, answer) {
                break;
            }
        }
        self.feed.borrow_mut().loading_older = false;
        self.render();
    }

    fn begin_older(&self) -> Option<u64> {
        let mut feed = self.feed.borrow_mut();
        if feed.loading_older {
            return None;
        }
        feed.loading_older = true;
        feed.notice = None;
        let generation = feed.generation;
        drop(feed);
        self.render();
        Some(generation)
    }

    fn older_query(&self, generation: u64) -> Option<(Filter, String)> {
        let feed = self.feed.borrow();
        let cursor = feed.lines.as_ref()?.older_cursor()?.to_owned();
        (feed.generation == generation).then(|| (feed.filter.clone(), cursor))
    }

    /// Prepends one older page. True when this click has nothing more to follow.
    fn older_landed(self: &Rc<Self>, generation: u64, answer: Result<LogPage, CallError>) -> bool {
        if self.feed.borrow().generation != generation {
            return true;
        }
        let older = match answer {
            Ok(older) => older,
            Err(CallError::Refused(r)) if r.code == "cursor_expired" => {
                self.restart_after_rotation();
                return true;
            }
            Err(e) => {
                self.failed(e);
                return true;
            }
        };
        let mut feed = self.feed.borrow_mut();
        let Some(lines) = feed.lines.as_mut() else {
            return true;
        };
        let merged = lines.prepend_older(older);
        drop(feed);
        let added = merged.added.len();
        self.store.splice(0, 0, &boxed(merged.added));
        self.follow_tail();
        added > 0
    }

    /// Only "Load older" meets a rotation, since polls carry no cursor. The
    /// newest page is read again and the reader is told why the view jumped.
    fn restart_after_rotation(self: &Rc<Self>) {
        let mut feed = self.feed.borrow_mut();
        feed.lines = None;
        feed.notice = Some(Notice::Said(ROTATED.to_owned()));
        drop(feed);
        self.store.remove_all();
        self.poll();
    }

    /// A refusal is shown in the daemon's words. Losing the daemon shows where
    /// the log lives instead; the poll goes on and brings the lines back.
    fn failed(&self, e: CallError) {
        let problem = daemon_problem(&e);
        match e {
            CallError::Refused(refusal) if !matches!(problem, DaemonProblem::UpdateNeeded(_)) => {
                let mut feed = self.feed.borrow_mut();
                let repeated =
                    matches!(&feed.notice, Some(Notice::Refused(s)) if *s == refusal.sentence);
                if !repeated {
                    glib::g_warning!("fermix", "logs.query was refused: {}", refusal.code);
                }
                feed.notice = Some(Notice::Refused(refusal.sentence));
                drop(feed);
                self.render();
            }
            other => self.went_down(&problem, &other),
        }
    }

    fn went_down(&self, problem: &DaemonProblem, e: &CallError) {
        let was_down = std::mem::replace(&mut self.feed.borrow_mut().down, true);
        if !was_down {
            glib::g_warning!("fermix", "Logs could not reach the daemon: {e:?}");
        }
        let (title, description) = match problem {
            DaemonProblem::UpdateNeeded(sentence) => {
                ("Update needed", format!("{sentence} {WHERE_LOGS_ARE}"))
            }
            DaemonProblem::NotRunning => ("Fermix is not running", WHERE_LOGS_ARE.to_owned()),
            DaemonProblem::NotResponding => ("Fermix is not responding", WHERE_LOGS_ARE.to_owned()),
            DaemonProblem::Broken(_) => (
                "Fermix answered in a way this app does not understand",
                WHERE_LOGS_ARE.to_owned(),
            ),
        };
        self.down.set_title(title);
        let description = glib::markup_escape_text(&description);
        self.down.set_description(Some(description.as_str()));
        self.stack.set_visible_child_name("down");
    }

    fn came_back(&self) {
        self.feed.borrow_mut().down = false;
        self.stack.set_visible_child_name("logs");
    }

    fn say(&self, sentence: &str) {
        self.feed.borrow_mut().notice = Some(Notice::Said(sentence.to_owned()));
        self.render();
    }

    /// Copies every line the view holds: the filter is the daemon's, so what is
    /// held is what matches.
    fn copy_visible(&self) {
        let feed = self.feed.borrow();
        let Some(lines) = feed.lines.as_ref() else {
            return;
        };
        let (text, count) = (copy_text(lines.entries()), lines.entries().len());
        drop(feed);
        self.root.clipboard().set_text(&text);
        if count == 1 {
            return self.say("Copied 1 line.");
        }
        self.say(&format!("Copied {count} lines."));
    }

    fn render(&self) {
        let feed = self.feed.borrow();
        let lines = feed.lines.as_ref();
        let empty = lines.is_some_and(|l| l.entries().is_empty());
        self.body
            .set_visible_child_name(if empty { "empty" } else { "list" });
        self.empty.set_title(if feed.filter == Filter::default() {
            "The log is empty"
        } else {
            "No entries match this filter"
        });
        self.controls
            .older
            .set_visible(lines.is_some_and(LogLines::can_load_older));
        self.controls.older.set_sensitive(!feed.loading_older);
        self.controls
            .copy
            .set_sensitive(lines.is_some_and(|l| !l.entries().is_empty()));
        let capped = lines
            .filter(|l| l.capped())
            .map(|_| format!("This view keeps the newest {MAX_LINES} lines."));
        let note = match &feed.notice {
            Some(Notice::Refused(s) | Notice::Said(s)) => Some(s.clone()),
            None => capped,
        };
        self.note.set_text(note.as_deref().unwrap_or(""));
        self.note.set_visible(note.is_some());
    }
}

/// Index 0 of the level menu is every level; the rest follow `Level::ALL`.
fn level_at(index: u32) -> Option<Level> {
    let index = usize::try_from(index.checked_sub(1)?).ok()?;
    Level::ALL.get(index).copied()
}

fn boxed(entries: Vec<LogEntry>) -> Vec<glib::BoxedAnyObject> {
    entries.into_iter().map(glib::BoxedAnyObject::new).collect()
}

fn controls() -> Controls {
    let mut levels = vec!["All levels"];
    levels.extend(Level::ALL.iter().map(|l| l.label()));
    let level = gtk::DropDown::from_strings(&levels);
    level.set_tooltip_text(Some("The least severe level shown"));
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search")
        .hexpand(true)
        .build();
    let older = gtk::Button::builder()
        .label("Load older")
        .visible(false)
        .build();
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy visible lines")
        .build();
    Controls {
        older,
        level,
        search,
        copy,
    }
}

fn controls_bar(controls: &Controls) -> gtk::Box {
    let bar = gtk::Box::builder()
        .spacing(6)
        .margin_top(12)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    bar.append(&controls.older);
    bar.append(&controls.level);
    bar.append(&controls.search);
    bar.append(&controls.copy);
    bar
}

/// A list that draws only the lines in view, so thousands stay fast.
fn log_list(store: &gio::ListStore) -> (gtk::ScrolledWindow, gtk::ListView) {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("a list view's items are list items");
        item.set_child(Some(&line_widget()));
    });
    factory.connect_bind(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("a list view's items are list items");
        let entry = item
            .item()
            .and_downcast::<glib::BoxedAnyObject>()
            .expect("the log store holds boxed entries");
        let line = item
            .child()
            .and_downcast::<gtk::Box>()
            .expect("each line is the box set up above");
        show_line(&line, &entry.borrow::<LogEntry>());
    });
    let list = gtk::ListView::builder()
        .model(&gtk::NoSelection::new(Some(store.clone())))
        .factory(&factory)
        .css_classes(["log-lines"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    (scroller, list)
}

/// The time and level, then the message, which wraps under itself.
fn line_widget() -> gtk::Box {
    let meta = gtk::Label::builder()
        .xalign(0.0)
        .valign(gtk::Align::Start)
        .build();
    let message = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .hexpand(true)
        .selectable(true)
        .build();
    let line = gtk::Box::builder().spacing(12).build();
    line.append(&meta);
    line.append(&message);
    line
}

fn show_line(line: &gtk::Box, entry: &LogEntry) {
    let meta = line
        .first_child()
        .and_downcast::<gtk::Label>()
        .expect("a line starts with its time and level");
    let message = line
        .last_child()
        .and_downcast::<gtk::Label>()
        .expect("a line ends with its message");
    meta.set_text(&format!("{}  {:<9}", shown_time(&entry.time), entry.level));
    message.set_text(&entry.message);
    let tone = match emphasis(&entry.level) {
        Emphasis::Alarm => Some("error"),
        Emphasis::Warn => Some("warning"),
        Emphasis::Quiet => Some("dim-label"),
        Emphasis::Plain => None,
    };
    let mut classes = vec!["monospace"];
    classes.extend(tone);
    meta.set_css_classes(&classes);
    message.set_css_classes(&classes);
}

/// Tracks whether the reader is within `STICK_DISTANCE` of the bottom; once
/// they scroll up to read, new lines leave the view alone.
fn watch_bottom(scroller: &gtk::ScrolledWindow) -> Rc<Cell<bool>> {
    let stuck = Rc::new(Cell::new(true));
    let watch = stuck.clone();
    scroller.vadjustment().connect_value_changed(move |a| {
        watch.set(a.value() + a.page_size() >= a.upper() - STICK_DISTANCE);
    });
    stuck
}

fn down_page() -> adw::StatusPage {
    let command = gtk::Label::builder()
        .label(JOURNAL_COMMAND)
        .selectable(true)
        .halign(gtk::Align::Center)
        .css_classes(["monospace"])
        .build();
    adw::StatusPage::builder()
        .icon_name("network-offline-symbolic")
        .child(&command)
        .build()
}
