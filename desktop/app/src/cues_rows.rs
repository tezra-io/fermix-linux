//! The rows the Cues pane and its pages are drawn with (M60 cues §4.2): a list row from the
//! `Line` core decides, the dot before a status sentence, a status line a pane keeps current, a
//! fact, a question asked in place of a row's buttons, and a group whose rows are rebuilt only
//! when what they show changed. Stock libadwaita rows and classes only.

use crate::descriptor::info_button;
use adw::prelude::*;
use fermix_client::chat::{written_time, LocalTime};
use fermix_client::cues::view::{question_text, question_verb, Line, Question, Tone};
use fermix_client::cues::words;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

/// A daemon's RFC 3339 time as Chat writes it (`chat::written_time`), on this computer's clock
/// and in its zone at that moment. None, and nothing drawn, for a time GLib cannot read.
pub fn written(at: &str) -> Option<String> {
    let (at, now) = on_this_clock(at)?;
    Some(written_time(at, now))
}

/// A daemon's time as the day alone (an instruction's day added).
pub fn written_day(at: &str) -> Option<String> {
    let (at, now) = on_this_clock(at)?;
    Some(words::day(at, now))
}

/// `at` and now, read as Chat reads them.
fn on_this_clock(at: &str) -> Option<(LocalTime, LocalTime)> {
    let time = glib::DateTime::from_iso8601(at, None).and_then(|time| time.to_local());
    match (time, glib::DateTime::now_local()) {
        (Ok(time), Ok(now)) => Some((local_time(&time), local_time(&now))),
        (Err(e), _) | (_, Err(e)) => {
            glib::g_warning!("fermix", "could not show the time {at}: {e}");
            None
        }
    }
}

fn local_time(time: &glib::DateTime) -> LocalTime {
    LocalTime {
        year: time.year(),
        month: time.month(),
        day: time.day_of_month(),
        hour: time.hour(),
        minute: time.minute(),
    }
}

/// The class a tone is drawn in.
fn tone_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Good => "success",
        Tone::Warn => "warning",
        Tone::Fail => "error",
        Tone::Quiet => "dimmed",
    }
}

/// The dot before a status sentence. Decoration: the sentence beside it is the state, so a
/// screen reader skips it.
pub fn tone_dot(tone: Tone) -> gtk::Image {
    gtk::Image::builder()
        .icon_name("media-record-symbolic")
        .pixel_size(8)
        .valign(gtk::Align::Center)
        .css_classes([tone_class(tone)])
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build()
}

/// One line of the daemon's under a row of a section, with its dot: drawn again in place when
/// its words change, hidden while it has none.
pub struct StatusLine {
    pub row: adw::ActionRow,
    dot: gtk::Image,
}

impl StatusLine {
    pub fn new() -> StatusLine {
        let row = adw::ActionRow::builder()
            .activatable(false)
            .visible(false)
            .build();
        row.set_title_lines(0);
        let dot = tone_dot(Tone::Quiet);
        row.add_prefix(&dot);
        StatusLine { row, dot }
    }

    /// The sentence and its tone, and whether it is set in the attention style.
    pub fn show(&self, line: Option<(&str, Tone, bool)>) {
        let Some((text, tone, attention)) = line else {
            self.row.set_visible(false);
            return;
        };
        self.row.set_title(&glib::markup_escape_text(text));
        for tone in [Tone::Good, Tone::Warn, Tone::Fail, Tone::Quiet] {
            self.dot.remove_css_class(tone_class(tone));
        }
        self.dot.add_css_class(tone_class(tone));
        set_class(&self.row, "warning", attention);
        set_class(&self.row, "dimmed", !attention);
        self.row.set_visible(true);
    }
}

pub fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// What a list row can do, each bound to the row it was built for. None draws no control.
#[derive(Default)]
pub struct RowActions {
    pub switch: Option<Rc<dyn Fn(bool)>>,
    pub approve: Option<Rc<dyn Fn()>>,
    pub open: Option<Rc<dyn Fn()>>,
}

/// Where a list row stands beyond its `Line`: a question it asks in place of its buttons, an
/// action running, the daemon's last refusal on it, and whether writes are blocked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowState {
    pub asking: Option<Question>,
    pub acting: bool,
    pub refusal: Option<String>,
    pub locked: bool,
}

/// A row as drawn, so a list is rebuilt only when one of its rows changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    pub line: Line,
    pub state: RowState,
    pub opens: bool,
}

/// One row of a list: its icon, its name, the daemon's sentence after the dot, then its tag, its
/// Approve and its switch where the daemon publishes them, and a chevron where it opens a page.
/// A question asked replaces its buttons; a refusal replaces its sentence.
pub fn line_row(drawn: &Drawn, actions: &RowActions, answer: &QuestionActions) -> adw::ActionRow {
    let line = &drawn.line;
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title(line)))
        .subtitle(glib::markup_escape_text(&line.sentence))
        .subtitle_lines(2)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(line.icon));
    if let Some(question) = drawn.state.asking {
        ask_in_place(&row, question, &line.name, drawn.state.acting, answer);
        return row;
    }
    if let Some(sentence) = &drawn.state.refusal {
        row.set_subtitle(&glib::markup_escape_text(sentence));
        row.add_css_class("setting-refused");
    } else if line.attention {
        row.add_css_class("setting-refused");
    }
    if let Some((tag, warn)) = line.tag {
        row.add_suffix(&tag_label(tag, warn));
    }
    row.add_suffix(&tone_dot(line.tone));
    add_controls(&row, drawn, actions);
    row
}

/// The name, and what qualifies it after a middle dot.
fn title(line: &Line) -> String {
    match &line.qualifier {
        Some(qualifier) => words::middot(&[line.name.clone(), qualifier.clone()]),
        None => line.name.clone(),
    }
}

fn tag_label(text: &str, warn: bool) -> gtk::Label {
    let tone = if warn { "warning" } else { "dimmed" };
    gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .css_classes(["caption", tone])
        .build()
}

fn add_controls(row: &adw::ActionRow, drawn: &Drawn, actions: &RowActions) {
    let busy = drawn.state.acting || drawn.state.locked;
    if let (true, Some(approve)) = (drawn.line.approve, &actions.approve) {
        let button = gtk::Button::builder()
            .label(words::APPROVE)
            .valign(gtk::Align::Center)
            .sensitive(!busy)
            .build();
        let label = words::middot(&[words::APPROVE.to_owned(), drawn.line.name.clone()]);
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
        let approve = approve.clone();
        button.connect_clicked(move |_| approve());
        row.add_suffix(&button);
    }
    if drawn.state.acting {
        row.add_suffix(&adw::Spinner::new());
    }
    if let (Some(on), Some(switched)) = (drawn.line.switch, &actions.switch) {
        row.add_suffix(&row_switch(&drawn.line.name, on, !busy, switched.clone()));
    }
    if let (true, Some(open)) = (drawn.opens, &actions.open) {
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_activatable(true);
        let open = open.clone();
        row.connect_activated(move |_| open());
    }
}

/// The row's own switch: throwing it never opens the row's page, and it waits for the daemon.
fn row_switch(name: &str, on: bool, sensitive: bool, switched: Rc<dyn Fn(bool)>) -> gtk::Switch {
    let switch = gtk::Switch::builder()
        .active(on)
        .valign(gtk::Align::Center)
        .sensitive(sensitive)
        .build();
    switch.update_property(&[gtk::accessible::Property::Label(&words::switch_label(name))]);
    switch.connect_active_notify(move |switch| {
        switch.set_sensitive(false);
        switched(switch.is_active());
    });
    switch
}

/// What the buttons of a question asked in place do: Cancel withdraws it, the verb answers it.
pub struct QuestionActions {
    pub cancel: Rc<dyn Fn()>,
    pub confirm: Rc<dyn Fn()>,
}

/// The question in the row's sentence's place, with Cancel and its one verb, as the phone's
/// Forget asks: nothing is raised over the pane.
fn ask_in_place(
    row: &adw::ActionRow,
    question: Question,
    target: &str,
    acting: bool,
    answer: &QuestionActions,
) {
    row.set_subtitle(&glib::markup_escape_text(&question_text(
        question,
        target,
        &[],
    )));
    row.set_subtitle_lines(0);
    for button in question_buttons(question, acting, answer) {
        row.add_suffix(&button);
    }
}

/// Cancel and the question's verb. A removal's verb is drawn as destructive; neither is a
/// default, so Enter answers nothing.
pub fn question_buttons(
    question: Question,
    acting: bool,
    answer: &QuestionActions,
) -> Vec<gtk::Widget> {
    let cancel = gtk::Button::builder()
        .label(words::CANCEL)
        .valign(gtk::Align::Center)
        .sensitive(!acting)
        .build();
    let on_cancel = answer.cancel.clone();
    cancel.connect_clicked(move |_| on_cancel());
    let style = match question {
        Question::RemoveSource | Question::RemoveMandate => "destructive-action",
        Question::Approve | Question::NewToken => "suggested-action",
    };
    let verb = gtk::Button::builder()
        .label(question_verb(question))
        .valign(gtk::Align::Center)
        .sensitive(!acting)
        .css_classes([style])
        .build();
    let on_confirm = answer.confirm.clone();
    verb.connect_clicked(move |button| {
        button.set_sensitive(false);
        on_confirm();
    });
    let mut buttons: Vec<gtk::Widget> = vec![cancel.upcast(), verb.upcast()];
    if acting {
        buttons.push(adw::Spinner::new().upcast());
    }
    buttons
}

/// One line standing in for a list: Reading, the daemon's sentence where it refused, or the
/// list's own empty line.
pub fn note_row(text: &str, spinning: bool, attention: bool) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(text))
        .activatable(false)
        .build();
    row.set_title_lines(0);
    if spinning {
        row.add_suffix(&adw::Spinner::new());
    }
    set_class(&row, if attention { "warning" } else { "dimmed" }, true);
    row
}

/// A fact in the daemon's words or the owner's: the label small, the value large.
pub fn fact_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(value))
        .subtitle_selectable(true)
        .css_classes(["property"])
        .build();
    row.set_subtitle_lines(0);
    row
}

/// A list group's header: how many rows it holds, where that helps, its (i), and its one action.
pub fn group_header(
    group: &adw::PreferencesGroup,
    info: Option<&str>,
    action: Option<&gtk::Button>,
) -> gtk::Label {
    let count = gtk::Label::builder()
        .css_classes(["dimmed", "numeric"])
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    let suffix = gtk::Box::builder().spacing(6).build();
    suffix.append(&count);
    if let Some(info) = info {
        suffix.append(&info_button(info, words::MORE_ABOUT));
    }
    if let Some(action) = action {
        suffix.append(action);
    }
    group.set_header_suffix(Some(&suffix));
    count
}

/// One group whose rows are rebuilt whenever what they show changes.
pub struct Redrawn<T: PartialEq> {
    pub group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
    shown: RefCell<Option<T>>,
}

impl<T: PartialEq> Redrawn<T> {
    pub fn new(group: adw::PreferencesGroup) -> Redrawn<T> {
        Redrawn {
            group,
            rows: RefCell::default(),
            shown: RefCell::default(),
        }
    }

    pub fn show(&self, value: T, build: impl FnOnce(&T) -> Vec<gtk::Widget>) {
        if self.shown.borrow().as_ref() == Some(&value) {
            return;
        }
        for old in self.rows.borrow_mut().drain(..) {
            self.group.remove(&old);
        }
        let fresh = build(&value);
        for row in &fresh {
            self.group.add(row);
        }
        *self.rows.borrow_mut() = fresh;
        *self.shown.borrow_mut() = Some(value);
    }

    /// Rebuilds on the next `show`, so each switch shows the daemon's value again.
    pub fn forget(&self) {
        self.shown.replace(None);
    }
}
