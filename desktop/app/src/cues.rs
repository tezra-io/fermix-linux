//! Cues (M60 cues §4): what can wake a task from outside, and what a task may do on its own. The
//! pane owns its data and its daemon calls, as Integrations does: it reads `cues.status`, the
//! sources, the watchers and the mandates as it opens, re-reads a list when the status counts
//! move, and writes through its own buttons, never through a window action. Its switches, filter
//! model and door port are the daemon's `cues` section, drawn by `descriptor.rs` with the pane's
//! own rows under and in place of some.

use crate::cues_filter::{FilterHooks, FilterRows};
use crate::cues_mandates::{MandateDeps, MandateDialog};
use crate::cues_rows::{
    group_header, line_row, note_row, written, Drawn, QuestionActions, Redrawn, RowActions,
    RowState, StatusLine,
};
use crate::cues_sources::{SourceDeps, SourceDialog};
use crate::daemon::Daemon;
use crate::descriptor::{Extra, Place, SectionView};
use adw::prelude::*;
use fermix_client::cues::view::{
    available, door_in_the_open, door_tone, failure, mandate_line, opening, shut, source_line,
    status_tone, waiting_lines, watcher_line, Asking, Failure, Question, Tone, WaitingLine,
    DOOR_PORT_KEY, ENABLED_KEY, FILTER_MODEL_KEY, FILTER_PROVIDER_KEY, SECTION, WAITING_ICON,
};
use fermix_client::cues::wire::{
    CueStatus, MandateList, ProposalFilter, ProposalList, SourceList, StatusCounts, WatcherList,
};
use fermix_client::cues::words;
use fermix_client::management::CallError;
use fermix_client::model::Capabilities;
use fermix_client::settings::{Section, SectionRows};
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

/// What the pane reaches outside itself: a line in the window, a fresh read of a settings
/// section, and a page of the window shown by name.
pub struct CueHooks {
    pub toast: Rc<dyn Fn(&str)>,
    pub read_section: Rc<dyn Fn(&str)>,
    pub page: Rc<dyn Fn(&str)>,
}

/// One list as last read: `None` until read, `Err` the sentence drawn in its place.
type Read<T> = Option<Result<T, String>>;

/// Opens Chat at a proposal's card, by its id. It never decides.
pub type Reveal = Rc<dyn Fn(&str)>;

#[derive(Default)]
struct CueData {
    status: Option<CueStatus>,
    /// A cue method answered that the daemon has no such method: there is no pane.
    missing: bool,
    sources: Read<SourceList>,
    watchers: Read<WatcherList>,
    mandates: Read<MandateList>,
    waiting: Read<ProposalList>,
    asking: Asking,
    locked: bool,
}

/// A list as drawn: one line in its place, or its rows.
#[derive(Debug, Clone, PartialEq)]
enum ListView {
    Note {
        text: String,
        spinning: bool,
        attention: bool,
    },
    Rows(Vec<Drawn>),
}

/// Waiting for you as drawn: hidden while nothing waits, one line in place of its rows, or
/// its rows.
#[derive(Debug, Clone, PartialEq)]
enum Waiting {
    Hidden,
    Note { text: String, attention: bool },
    Rows(Vec<WaitingLine>),
}

/// The group a list is drawn in, its count in the header, and its Add button where it has one.
struct List {
    rows: Redrawn<ListView>,
    count: gtk::Label,
    add: Option<gtk::Button>,
}

pub struct CuesPane {
    pub page: adw::PreferencesPage,
    /// The `cues` section, which Settings draws with the other sections.
    pub view: Rc<SectionView>,
    daemon: Daemon,
    hooks: CueHooks,
    status_line: StatusLine,
    /// The door's sentence under its port, folded with it.
    door_line: StatusLine,
    /// The door's sentence in the open, as a warning, while the door could not open.
    door_open: StatusLine,
    filter: Rc<FilterRows>,
    waiting: Redrawn<Waiting>,
    reveal: RefCell<Option<Reveal>>,
    sources: List,
    watchers: List,
    mandates: List,
    data: RefCell<CueData>,
}

impl CuesPane {
    pub fn new(daemon: Daemon, hooks: CueHooks) -> Rc<CuesPane> {
        let filter = FilterRows::new(
            daemon.clone(),
            FilterHooks {
                toast: hooks.toast.clone(),
                read_section: hooks.read_section.clone(),
            },
        );
        let (status_line, door_line, door_open) =
            (StatusLine::new(), StatusLine::new(), StatusLine::new());
        let view = SectionView::keeping_with_extras(
            SECTION,
            Box::new(|_| true),
            vec![
                extra(ENABLED_KEY, Place::Under, &status_line.row),
                extra(ENABLED_KEY, Place::Under, &door_open.row),
                extra(FILTER_PROVIDER_KEY, Place::Instead, &filter.provider),
                extra(FILTER_MODEL_KEY, Place::Instead, &filter.model),
                extra(DOOR_PORT_KEY, Place::Under, &door_line.row),
            ],
        );
        let page = adw::PreferencesPage::new();
        let (sources, watchers, mandates) = (
            list(words::SOURCES_TITLE, None, Some(words::ADD_SOURCE)),
            list(words::WATCHERS_TITLE, Some(words::WATCHERS_FOOTER), None),
            list(words::MANDATES_TITLE, None, Some(words::ADD_MANDATE)),
        );
        page.add(&lead_group(hooks.toast.clone()));
        page.add(&view.group);
        let waiting = adw::PreferencesGroup::builder()
            .title(words::WAITING_TITLE)
            .visible(false)
            .build();
        page.add(&waiting);
        for list in [&sources, &watchers, &mandates] {
            page.add(&list.rows.group);
        }
        let pane = Rc::new(CuesPane {
            page,
            view,
            daemon,
            hooks,
            status_line,
            door_line,
            door_open,
            filter,
            waiting: Redrawn::new(waiting),
            reveal: RefCell::default(),
            sources,
            watchers,
            mandates,
            data: RefCell::default(),
        });
        pane.leave_drops_questions();
        pane.wire_adds();
        pane.render();
        pane
    }

    /// Add source and Add mandate each open their dialog over the pane.
    fn wire_adds(self: &Rc<Self>) {
        if let Some(add) = &self.sources.add {
            let weak = Rc::downgrade(self);
            add.connect_clicked(move |_| {
                if let Some(pane) = weak.upgrade() {
                    SourceDialog::open_add(&pane.page, pane.source_deps());
                }
            });
        }
        if let Some(add) = &self.mandates.add {
            let weak = Rc::downgrade(self);
            add.connect_clicked(move |_| {
                if let Some(pane) = weak.upgrade() {
                    MandateDialog::open_add(&pane.page, pane.mandate_deps());
                }
            });
        }
    }

    /// Leaving the pane withdraws a question asked and drops a filter provider held.
    fn leave_drops_questions(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.page.connect_unmap(move |_| {
            let Some(pane) = weak.upgrade() else { return };
            pane.filter.drop_held();
            pane.data.borrow_mut().asking.withdraw();
            pane.render();
        });
    }

    /// Where Show in chat goes: the window's Chat, set once the window is built.
    pub fn set_reveal(&self, reveal: Reveal) {
        *self.reveal.borrow_mut() = Some(reveal);
    }

    /// The proposals waiting, as the last status read counted them.
    pub fn proposals_waiting(&self) -> u64 {
        let data = self.data.borrow();
        data.status
            .as_ref()
            .map_or(0, |s| s.counts.proposals_waiting)
    }

    /// The oldest proposal waiting, read now: where the tray's line opens Chat.
    pub async fn oldest_waiting(&self) -> Option<String> {
        let answer = self
            .daemon
            .call(|m| m.cue_proposals(ProposalFilter::Waiting))
            .await;
        match answer {
            Ok(list) => list.proposals.into_iter().next().map(|p| p.proposal_id),
            Err(e) => {
                glib::g_warning!("fermix", "cues.proposals.list failed: {e:?}");
                None
            }
        }
    }

    // ---- the gate ----

    /// Whether the pane exists now (§4.1).
    pub fn available(&self, capabilities: &Capabilities, sections: Option<&[Section]>) -> bool {
        let data = self.data.borrow();
        !data.missing && available(Some(capabilities), sections, data.status.as_ref())
    }

    /// Whether the gate has said no, so a route aimed at the pane goes elsewhere.
    pub fn shut(&self, capabilities: &Capabilities, sections: Option<&[Section]>) -> bool {
        shut(Some(capabilities), sections, self.data.borrow().missing)
    }

    // ---- reading (§4.9) ----

    /// Call whenever the pane comes into view: the status and each list, each read on its own.
    pub fn shown(self: &Rc<Self>) {
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_status_only().await });
        self.read_lists();
    }

    fn read_lists(self: &Rc<Self>) {
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_sources().await });
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_watchers().await });
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_mandates().await });
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_waiting().await });
    }

    /// The idle refresh's read: `cues.status`, and every list again when its counts moved while
    /// the pane is on screen.
    pub async fn read_status(self: &Rc<Self>) {
        let before = self.counts();
        self.read_status_only().await;
        let after = self.counts();
        if before.is_some() && after != before && self.page.is_mapped() {
            self.read_lists();
        }
    }

    fn counts(&self) -> Option<StatusCounts> {
        self.data.borrow().status.as_ref().map(|s| s.counts)
    }

    async fn read_status_only(self: &Rc<Self>) {
        let answer = self.daemon.call(|m| m.cues_status()).await;
        // A status that could not be read keeps the last one; the pane stays as it was.
        if let Some(Ok(status)) = self.settle(answer, "cues.status") {
            let mut data = self.data.borrow_mut();
            data.missing = false;
            data.status = Some(status);
        }
        self.render();
    }

    async fn read_sources(self: &Rc<Self>) {
        let answer = self.daemon.call(|m| m.cue_sources()).await;
        let read = self.settle(answer, "cues.sources.list");
        self.data.borrow_mut().sources = read;
        self.render();
    }

    async fn read_watchers(self: &Rc<Self>) {
        let answer = self.daemon.call(|m| m.cue_watchers()).await;
        let read = self.settle(answer, "cues.watchers.list");
        self.data.borrow_mut().watchers = read;
        self.render();
    }

    async fn read_mandates(self: &Rc<Self>) {
        let answer = self.daemon.call(|m| m.cue_mandates()).await;
        let read = self.settle(answer, "cues.mandates.list");
        self.data.borrow_mut().mandates = read;
        self.render();
    }

    async fn read_waiting(self: &Rc<Self>) {
        let answer = self
            .daemon
            .call(|m| m.cue_proposals(ProposalFilter::Waiting))
            .await;
        let read = self.settle(answer, "cues.proposals.list");
        self.data.borrow_mut().waiting = read;
        self.render();
    }

    /// A read's answer, or the sentence drawn in its place. A method the daemon does not have
    /// takes the pane away; nothing is drawn for it.
    fn settle<T>(&self, answer: Result<T, CallError>, what: &str) -> Read<T> {
        match answer {
            Ok(value) => Some(Ok(value)),
            Err(e) => self.failed(what, &e).map(Err),
        }
    }

    // ---- drawing ----

    /// The `cues` section as Settings last read it, and whether writes are blocked.
    pub fn render_settings(self: &Rc<Self>, section: Option<&SectionRows>, locked: bool) {
        self.filter.show(section, locked);
        let changed = self.data.borrow().locked != locked;
        if changed {
            self.data.borrow_mut().locked = locked;
            self.render();
        }
    }

    fn render(self: &Rc<Self>) {
        self.render_status();
        let (sources, watchers, mandates) = {
            let data = self.data.borrow();
            (
                sources_view(&data),
                watchers_view(&data),
                mandates_view(&data),
            )
        };
        show_count(&self.sources.count, &sources);
        show_count(&self.watchers.count, &watchers);
        show_count(&self.mandates.count, &mandates);
        self.render_adds();
        self.render_waiting();
        self.sources
            .rows
            .show(sources, |view| self.list_rows(view, Self::source_actions));
        self.watchers
            .rows
            .show(watchers, |view| self.list_rows(view, Self::watcher_actions));
        self.mandates
            .rows
            .show(mandates, |view| self.list_rows(view, Self::mandate_actions));
    }

    /// Waiting for you, only while a proposal waits (§4.7).
    fn render_waiting(self: &Rc<Self>) {
        let view = waiting_view(&self.data.borrow());
        self.waiting.group.set_visible(view != Waiting::Hidden);
        self.waiting.show(view, |view| match view {
            Waiting::Hidden => Vec::new(),
            Waiting::Note { text, attention } => vec![note_row(text, false, *attention).upcast()],
            Waiting::Rows(lines) => lines
                .iter()
                .map(|line| self.waiting_row(line).upcast())
                .collect(),
        });
    }

    /// Who wants to run what and until when, with Show in chat, which opens Chat at the card.
    /// Deciding happens on the card only.
    fn waiting_row(self: &Rc<Self>, line: &WaitingLine) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&line.title))
            .subtitle(glib::markup_escape_text(
                line.waits.as_deref().unwrap_or(""),
            ))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(WAITING_ICON));
        let show = gtk::Button::builder()
            .label(words::SHOW_IN_CHAT)
            .valign(gtk::Align::Center)
            .build();
        let spoken = words::middot(&[words::SHOW_IN_CHAT.to_owned(), line.title.clone()]);
        show.update_property(&[gtk::accessible::Property::Label(&spoken)]);
        let (weak, id) = (Rc::downgrade(self), line.proposal_id.clone());
        show.connect_clicked(move |_| {
            if let Some(pane) = weak.upgrade() {
                pane.show_in_chat(&id);
            }
        });
        row.add_suffix(&show);
        row.set_activatable_widget(Some(&show));
        row
    }

    fn show_in_chat(&self, proposal_id: &str) {
        let reveal = self.reveal.borrow().clone();
        match reveal {
            Some(reveal) => reveal(proposal_id),
            None => glib::g_warning!("fermix", "Show in chat has nowhere to go yet"),
        }
    }

    /// Each Add where the daemon says one can be added, and pressable while writes are not
    /// blocked.
    fn render_adds(&self) {
        let data = self.data.borrow();
        let sources = matches!(&data.sources, Some(Ok(list)) if list.can_add);
        let mandates = matches!(&data.mandates, Some(Ok(list)) if list.can_add);
        for (add, can) in [(&self.sources.add, sources), (&self.mandates.add, mandates)] {
            if let Some(add) = add {
                add.set_visible(can);
                add.set_sensitive(!data.locked);
            }
        }
    }

    /// The daemon's status sentence under Run cues, and the door's under its port, or in the
    /// open while the door could not open.
    fn render_status(&self) {
        let data = self.data.borrow();
        let Some(status) = &data.status else {
            for line in [&self.status_line, &self.door_line, &self.door_open] {
                line.show(None);
            }
            return;
        };
        let tone = status_tone(status);
        let warn = tone == Tone::Warn;
        self.status_line
            .show(Some((&status.status_sentence, tone, warn)));
        let (door, open) = (status.door.sentence.as_str(), door_in_the_open(status));
        self.door_open
            .show(open.then_some((door, door_tone(status), true)));
        self.door_line
            .show((!open).then_some((door, door_tone(status), false)));
    }

    fn list_rows(
        self: &Rc<Self>,
        view: &ListView,
        actions: fn(&Rc<Self>, &Drawn) -> RowActions,
    ) -> Vec<gtk::Widget> {
        let drawn = match view {
            ListView::Note {
                text,
                spinning,
                attention,
            } => return vec![note_row(text, *spinning, *attention).upcast()],
            ListView::Rows(drawn) => drawn,
        };
        drawn
            .iter()
            .map(|row| line_row(row, &actions(self, row), &self.answers()).upcast())
            .collect()
    }

    /// Cancel and the verb of a question asked in a row.
    fn answers(self: &Rc<Self>) -> QuestionActions {
        let (cancel, confirm) = (Rc::downgrade(self), Rc::downgrade(self));
        QuestionActions {
            cancel: Rc::new(move || {
                if let Some(pane) = cancel.upgrade() {
                    pane.withdraw();
                }
            }),
            confirm: Rc::new(move || {
                if let Some(pane) = confirm.upgrade() {
                    pane.confirm();
                }
            }),
        }
    }

    fn source_actions(self: &Rc<Self>, row: &Drawn) -> RowActions {
        let (weak, name) = (Rc::downgrade(self), row.line.name.clone());
        let (opens, opened) = (Rc::downgrade(self), name.clone());
        RowActions {
            switch: Some(Rc::new(move |on| {
                if let Some(pane) = weak.upgrade() {
                    pane.switch_source(name.clone(), on);
                }
            })),
            open: Some(Rc::new(move || {
                if let Some(pane) = opens.upgrade() {
                    pane.open_source(&opened);
                }
            })),
            ..RowActions::default()
        }
    }

    fn watcher_actions(self: &Rc<Self>, row: &Drawn) -> RowActions {
        let (weak, name) = (Rc::downgrade(self), row.line.name.clone());
        RowActions {
            open: Some(Rc::new(move || {
                if let Some(pane) = weak.upgrade() {
                    pane.open_watcher(&name);
                }
            })),
            ..RowActions::default()
        }
    }

    fn mandate_actions(self: &Rc<Self>, row: &Drawn) -> RowActions {
        let name = row.line.name.clone();
        let (weak, switched) = (Rc::downgrade(self), name.clone());
        let switch: Rc<dyn Fn(bool)> = Rc::new(move |on| {
            if let Some(pane) = weak.upgrade() {
                pane.switch_mandate(switched.clone(), on);
            }
        });
        let (weak, asked) = (Rc::downgrade(self), mandate_key(&name));
        let approve: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(pane) = weak.upgrade() {
                pane.ask(&asked, Question::Approve);
            }
        });
        let (weak, opened) = (Rc::downgrade(self), name);
        let open: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(pane) = weak.upgrade() {
                pane.open_mandate(&opened);
            }
        });
        RowActions {
            switch: Some(switch),
            approve: Some(approve),
            open: Some(open),
        }
    }

    /// A source's page, opened at the secret its row's action points at.
    fn open_source(self: &Rc<Self>, name: &str) {
        let at = {
            let data = self.data.borrow();
            let Some(Ok(list)) = &data.sources else {
                return;
            };
            list.sources
                .iter()
                .find(|s| s.name == name)
                .and_then(opening)
        };
        SourceDialog::open_source(&self.page, self.source_deps(), name, at);
    }

    fn open_mandate(self: &Rc<Self>, name: &str) {
        let row = {
            let data = self.data.borrow();
            let Some(Ok(list)) = &data.mandates else {
                return;
            };
            list.mandates.iter().find(|m| m.name == name).cloned()
        };
        let Some(row) = row else { return };
        MandateDialog::open_mandate(&self.page, self.mandate_deps(), row);
    }

    fn source_deps(self: &Rc<Self>) -> SourceDeps {
        let (changed, port) = (Rc::downgrade(self), Rc::downgrade(self));
        SourceDeps {
            daemon: self.daemon.clone(),
            toast: self.hooks.toast.clone(),
            changed: Rc::new(move || {
                if let Some(pane) = changed.upgrade() {
                    pane.reread_sources();
                }
            }),
            port: Rc::new(move || {
                let pane = port.upgrade()?;
                let data = pane.data.borrow();
                data.status.as_ref().map(|s| s.door.port)
            }),
        }
    }

    fn mandate_deps(self: &Rc<Self>) -> MandateDeps {
        let changed = Rc::downgrade(self);
        MandateDeps {
            daemon: self.daemon.clone(),
            changed: Rc::new(move || {
                if let Some(pane) = changed.upgrade() {
                    pane.reread_mandates();
                }
            }),
        }
    }

    /// A dialog changed a source: the sources and the status are read again.
    fn reread_sources(self: &Rc<Self>) {
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_sources().await });
        self.refresh_status();
    }

    fn reread_mandates(self: &Rc<Self>) {
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_mandates().await });
        self.refresh_status();
    }

    fn open_watcher(self: &Rc<Self>, name: &str) {
        let page = self.hooks.page.clone();
        let open_chat: Rc<dyn Fn()> = Rc::new(move || page("chat"));
        crate::cues_watchers::present(&self.page, self.daemon.clone(), name, open_chat);
    }

    // ---- questions asked in place ----

    fn ask(self: &Rc<Self>, key: &str, question: Question) {
        self.data.borrow_mut().asking.ask(key, question);
        self.render();
    }

    fn withdraw(self: &Rc<Self>) {
        self.data.borrow_mut().asking.withdraw();
        self.render();
    }

    /// The second press: does what was asked.
    fn confirm(self: &Rc<Self>) {
        let asked = self.data.borrow_mut().asking.confirm();
        let Some((key, question)) = asked else {
            return;
        };
        self.render();
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let refusal = match (question, key.strip_prefix(MANDATE)) {
                (Question::Approve, Some(name)) => pane.approve(name.to_owned()).await,
                _ => {
                    glib::g_warning!("fermix", "a {question:?} was asked where it has no answer");
                    None
                }
            };
            pane.data.borrow_mut().asking.finished(&key, refusal);
            pane.mandates.rows.forget();
            pane.render();
        });
    }

    // ---- writing ----

    /// Approves a mandate as it is now. The answer's row replaces the old one.
    async fn approve(self: &Rc<Self>, name: String) -> Option<String> {
        let target = name.clone();
        let answer = self
            .daemon
            .call(move |m| m.cue_mandate_approve(&target))
            .await;
        match answer {
            Ok(answer) => {
                self.replace_mandate(answer.mandate);
                self.refresh_status();
                None
            }
            Err(e) => self.failed(&format!("cues.mandates.approve {name}"), &e),
        }
    }

    fn switch_source(self: &Rc<Self>, name: String, on: bool) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let target = name.clone();
            let answer = pane
                .daemon
                .call(move |m| m.cue_source_switch(&target, on))
                .await;
            let key = source_key(&name);
            let what = format!("switching source {name}");
            let refusal = answer.as_ref().err().and_then(|e| pane.failed(&what, e));
            pane.data
                .borrow_mut()
                .asking
                .answered(&key, refusal.as_deref());
            if let Ok(answer) = answer {
                pane.replace_source(answer.source);
            }
            pane.sources.rows.forget();
            pane.render();
            pane.refresh_status();
        });
    }

    fn switch_mandate(self: &Rc<Self>, name: String, on: bool) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let target = name.clone();
            let answer = pane
                .daemon
                .call(move |m| m.cue_mandate_switch(&target, on))
                .await;
            let key = mandate_key(&name);
            let what = format!("switching mandate {name}");
            let refusal = answer.as_ref().err().and_then(|e| pane.failed(&what, e));
            pane.data
                .borrow_mut()
                .asking
                .answered(&key, refusal.as_deref());
            if let Ok(answer) = answer {
                pane.replace_mandate(answer.mandate);
            }
            pane.mandates.rows.forget();
            pane.render();
            pane.refresh_status();
        });
    }

    fn replace_source(&self, row: fermix_client::cues::wire::SourceRow) {
        let mut data = self.data.borrow_mut();
        let Some(Ok(list)) = data.sources.as_mut() else {
            return;
        };
        if let Some(old) = list.sources.iter_mut().find(|s| s.name == row.name) {
            *old = row;
        }
    }

    fn replace_mandate(&self, row: fermix_client::cues::wire::MandateRow) {
        let mut data = self.data.borrow_mut();
        let Some(Ok(list)) = data.mandates.as_mut() else {
            return;
        };
        if let Some(old) = list.mandates.iter_mut().find(|m| m.name == row.name) {
            *old = row;
        }
    }

    /// A write moves the counts the status carries; read it again.
    fn refresh_status(self: &Rc<Self>) {
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.read_status_only().await });
    }

    /// The sentence a failed call leaves where it was asked for, logged as `what`. A method the
    /// daemon lacks takes the pane away and leaves none.
    fn failed(&self, what: &str, e: &CallError) -> Option<String> {
        glib::g_warning!("fermix", "{what} failed: {e:?}");
        match failure(e) {
            Failure::Sentence(sentence) => Some(sentence),
            Failure::Missing => {
                self.data.borrow_mut().missing = true;
                None
            }
        }
    }
}

const SOURCE: &str = "source:";
const MANDATE: &str = "mandate:";

/// Rows are named apart per list, so a source and a mandate of one name never share a question.
fn source_key(name: &str) -> String {
    format!("{SOURCE}{name}")
}

fn mandate_key(name: &str) -> String {
    format!("{MANDATE}{name}")
}

fn extra(key: &str, place: Place, row: &impl IsA<gtk::Widget>) -> Extra {
    Extra {
        key: key.to_owned(),
        place,
        row: row.clone().upcast(),
    }
}

fn list(title: &str, info: Option<&str>, add: Option<&str>) -> List {
    let group = adw::PreferencesGroup::builder().title(title).build();
    let add = add.map(|label| {
        gtk::Button::builder()
            .label(label)
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(false)
            .build()
    });
    let count = group_header(&group, info, add.as_ref());
    List {
        rows: Redrawn::new(group),
        count,
        add,
    }
}

fn show_count(count: &gtk::Label, view: &ListView) {
    match view {
        ListView::Rows(rows) => {
            count.set_label(&rows.len().to_string());
            count.set_visible(true);
        }
        ListView::Note { .. } => count.set_visible(false),
    }
}

/// The pane's one quiet line, with the guide one press away.
fn lead_group(toast: Rc<dyn Fn(&str)>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .description(words::SUBTITLE)
        .build();
    let guide = gtk::Button::builder()
        .label(words::LEARN_MORE)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    guide.connect_clicked(move |button| {
        let window = button.root().and_downcast::<gtk::Window>();
        let (launcher, toast) = (gtk::UriLauncher::new(words::GUIDE_URL), toast.clone());
        glib::spawn_future_local(async move {
            let Err(e) = launcher.launch_future(window.as_ref()).await else {
                return;
            };
            glib::g_warning!("fermix", "the cues guide did not open: {e}");
            toast(words::GUIDE_FAILED);
        });
    });
    group.set_header_suffix(Some(&guide));
    group
}

/// A list's line in place of its rows: Reading while unread, the daemon's sentence where it
/// refused, the empty line where it holds nothing.
fn note_or<T>(read: &Read<T>, empty: &str, rows: impl Fn(&T) -> Vec<Drawn>) -> ListView {
    let note = |text: &str, spinning, attention| ListView::Note {
        text: text.to_owned(),
        spinning,
        attention,
    };
    match read {
        None => note(words::READING, true, false),
        Some(Err(sentence)) => note(sentence, false, true),
        Some(Ok(list)) => {
            let drawn = rows(list);
            if drawn.is_empty() {
                return note(empty, false, false);
            }
            ListView::Rows(drawn)
        }
    }
}

fn state(data: &CueData, key: &str) -> RowState {
    RowState {
        asking: data.asking.shown(key),
        acting: data.asking.acting(key),
        refusal: data.asking.refusal(key).map(str::to_owned),
        locked: data.locked,
    }
}

fn waiting_view(data: &CueData) -> Waiting {
    let count = data
        .status
        .as_ref()
        .map_or(0, |s| s.counts.proposals_waiting);
    if count == 0 {
        return Waiting::Hidden;
    }
    match &data.waiting {
        None => Waiting::Note {
            text: words::READING.to_owned(),
            attention: false,
        },
        Some(Err(sentence)) => Waiting::Note {
            text: sentence.clone(),
            attention: true,
        },
        Some(Ok(list)) => {
            let lines = waiting_lines(list, written);
            if lines.is_empty() {
                return Waiting::Hidden;
            }
            Waiting::Rows(lines)
        }
    }
}

fn sources_view(data: &CueData) -> ListView {
    note_or(&data.sources, words::SOURCES_EMPTY, |list| {
        list.sources
            .iter()
            .map(|s| Drawn {
                line: source_line(s),
                state: state(data, &source_key(&s.name)),
                opens: true,
            })
            .collect()
    })
}

fn watchers_view(data: &CueData) -> ListView {
    note_or(&data.watchers, words::WATCHERS_EMPTY, |list| {
        list.watchers
            .iter()
            .map(|w| Drawn {
                line: watcher_line(w),
                state: RowState::default(),
                opens: true,
            })
            .collect()
    })
}

fn mandates_view(data: &CueData) -> ListView {
    note_or(&data.mandates, words::MANDATES_EMPTY, |list| {
        list.mandates
            .iter()
            .map(|m| Drawn {
                line: mandate_line(m),
                state: state(data, &mandate_key(&m.name)),
                opens: true,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fermix_client::cues::view::Line;

    /// The pane's own files, which hold every cue write. Only what ships is scanned: these tests
    /// name what they look for.
    const FILES: [(&str, &str); 10] = [
        ("cues.rs", include_str!("cues.rs")),
        ("cues_dialog.rs", include_str!("cues_dialog.rs")),
        ("cues_filter.rs", include_str!("cues_filter.rs")),
        ("cues_flow.rs", include_str!("cues_flow.rs")),
        ("cues_mandates.rs", include_str!("cues_mandates.rs")),
        ("cues_rows.rs", include_str!("cues_rows.rs")),
        ("cues_source_page.rs", include_str!("cues_source_page.rs")),
        ("cues_sources.rs", include_str!("cues_sources.rs")),
        ("cues_watchers.rs", include_str!("cues_watchers.rs")),
        ("clipboard.rs", include_str!("clipboard.rs")),
    ];

    fn shipped(source: &str) -> &str {
        source
            .split("#[cfg(test)]")
            .next()
            .expect("split yields one part")
    }

    /// Each `g_warning!` call in `source`, from its name to its closing parenthesis.
    fn log_calls(source: &str) -> Vec<&str> {
        source
            .match_indices("g_warning!(")
            .map(|(start, _)| {
                let rest = &source[start..];
                let end = rest.find(");").expect("a log call ends");
                &rest[..end]
            })
            .collect()
    }

    fn names_a_token(call: &str) -> bool {
        call.to_lowercase().contains("token")
    }

    /// A pushed token is shown once and never logged (M60 cues §6): no log line of the pane's
    /// says the word, so none can carry one. The checker catches a planted line, and the scan
    /// reads files that log.
    #[test]
    fn no_log_line_of_the_pane_names_a_token() {
        let planted = "glib::g_warning!(\"fermix\", \"the new token {t:?}\");";
        assert!(log_calls(planted).iter().any(|c| names_a_token(c)));
        let mut calls = 0;
        for (file, source) in FILES {
            for call in log_calls(shipped(source)) {
                calls += 1;
                assert!(!names_a_token(call), "{file}: {call}");
            }
        }
        assert!(calls >= 5, "the scan reads the pane's log lines ({calls})");
    }

    /// A registered application exports every `win.*` action on the session bus, so no cue
    /// write is one: the pane's buttons write through its own daemon handle (M60 cues §6). Its
    /// reads and pages may go through the window; nothing else does.
    #[test]
    fn no_cue_write_is_a_window_action() {
        let (_, pane) = FILES[0];
        assert!(pane.contains("cue_source_switch") && pane.contains("cue_mandate_approve"));
        let reached = ["\"win.toast\"", "\"win.section-read\"", "\"win.page\""];
        for (file, source) in FILES {
            let source = shipped(source);
            assert!(!source.contains("action_name("), "{file}");
            assert!(!source.contains("activate_action("), "{file}");
            let named = source.matches("\"win.").count();
            let allowed: usize = reached.iter().map(|a| source.matches(a).count()).sum();
            assert_eq!(named, allowed, "{file} names a window action of its own");
        }
        let app = shipped(include_str!("app.rs"));
        assert!(
            app.contains("on(app, \"page\""),
            "the scan reads the registrations"
        );
        assert!(
            !app.contains("on(app, \"cue"),
            "app.rs registers a cue action"
        );
    }

    #[test]
    fn a_source_and_a_mandate_of_one_name_never_share_a_question() {
        assert_ne!(source_key("alerts"), mandate_key("alerts"));
        assert_eq!(mandate_key("alerts").strip_prefix(MANDATE), Some("alerts"));
    }

    fn note(text: &str, spinning: bool, attention: bool) -> ListView {
        ListView::Note {
            text: text.to_owned(),
            spinning,
            attention,
        }
    }

    #[test]
    fn a_list_reads_then_draws_its_rows_its_empty_line_or_the_daemons_sentence() {
        let none: Read<Vec<u8>> = None;
        let rows = |list: &Vec<u8>| -> Vec<Drawn> {
            list.iter()
                .map(|n| Drawn {
                    line: Line {
                        name: n.to_string(),
                        icon: "x",
                        qualifier: None,
                        tag: None,
                        sentence: String::new(),
                        tone: Tone::Good,
                        attention: false,
                        switch: None,
                        approve: false,
                    },
                    state: RowState::default(),
                    opens: false,
                })
                .collect()
        };
        assert_eq!(
            note_or(&none, "Empty.", rows),
            note(words::READING, true, false)
        );
        let refused: Read<Vec<u8>> = Some(Err("Memory is off.".into()));
        assert_eq!(
            note_or(&refused, "Empty.", rows),
            note("Memory is off.", false, true)
        );
        let empty: Read<Vec<u8>> = Some(Ok(Vec::new()));
        assert_eq!(
            note_or(&empty, "Empty.", rows),
            note("Empty.", false, false)
        );
        let two: Read<Vec<u8>> = Some(Ok(vec![1, 2]));
        let ListView::Rows(drawn) = note_or(&two, "Empty.", rows) else {
            panic!("two rows");
        };
        assert_eq!(drawn.len(), 2);
    }
}
