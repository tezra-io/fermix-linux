//! A mandate's page and its editor (M60 cues §4.6), in one dialog of pages. The page says what
//! the mandate lets its task do and how, with its switch, Approve, Edit and Remove mandate…,
//! each asked in place. The editor is written from the mandate's value keys (`mandate.rs`) and
//! fed by `cues.mandates.choices`, drawn in the descriptor's form mode with the mode as a toggle.

use crate::cues_dialog::{
    first_page, hidden_line, named_button, nav_page, page_dialog, show_line, Soon,
};
use crate::cues_rows::{
    fact_row, note_row, question_buttons, set_class, tone_dot, QuestionActions,
};
use crate::cues_sources::refusal_of;
use crate::daemon::Daemon;
use crate::descriptor::{Drawn, Extra, FormSink, Place, SectionView};
use crate::dialogs::keep_with;
use adw::prelude::*;
use fermix_client::cues::mandate::{MandateForm, APPROVE, AUTO};
use fermix_client::cues::view::{
    approval_line, mandate_switch, mandate_tone, mode_tag, offers_approve, question_text,
    task_name, Asking, Question, MANDATE_ICON,
};
use fermix_client::cues::wire::{MandateChoices, MandateMode, MandateRow};
use fermix_client::cues::words;
use fermix_client::settings::SectionRows;
use gtk::glib;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The id the editor's rows are drawn under; nothing is read or written by it.
const FORM_ID: &str = "cues.mandate.form";

/// What the dialog reaches outside itself.
pub struct MandateDeps {
    pub daemon: Daemon,
    /// A mandate was added, changed or removed: the pane reads its list and status again.
    pub changed: Rc<dyn Fn()>,
}

pub struct MandateDialog {
    deps: MandateDeps,
    dialog: adw::Dialog,
    nav: adw::NavigationView,
}

impl MandateDialog {
    fn new(deps: MandateDeps, title: &str) -> Rc<MandateDialog> {
        let (dialog, nav) = page_dialog(title);
        Rc::new(MandateDialog { deps, dialog, nav })
    }

    /// The editor for a new mandate, over `parent`. Once added, its page takes the editor's place.
    pub fn open_add(parent: &impl IsA<gtk::Widget>, deps: MandateDeps) {
        let dialog = MandateDialog::new(deps, words::ADD_MANDATE_TITLE);
        let weak = Rc::downgrade(&dialog);
        let took: Took = Box::new(move |row| {
            if let Some(dialog) = weak.upgrade() {
                (dialog.deps.changed)();
                dialog.dialog.set_title(&row.name);
                first_page(&dialog.nav, &MandatePage::open(&dialog, row));
            }
        });
        let page = Editor::open(&dialog, MandateForm::add(), took);
        first_page(&dialog.nav, &page);
        dialog.dialog.present(Some(parent));
    }

    /// A mandate's page, over `parent`.
    pub fn open_mandate(parent: &impl IsA<gtk::Widget>, deps: MandateDeps, row: MandateRow) {
        let dialog = MandateDialog::new(deps, &row.name);
        let page = MandatePage::open(&dialog, row);
        first_page(&dialog.nav, &page);
        dialog.dialog.present(Some(parent));
    }
}

/// What the editor does with the row the daemon took.
type Took = Box<dyn Fn(MandateRow)>;

// ---- a mandate's page ----

struct MandatePage {
    dialog: Rc<MandateDialog>,
    name: String,
    holder: adw::Bin,
    /// The mandate as the daemon last answered. A removal closes the dialog, so a page never
    /// outlives its mandate.
    row: RefCell<MandateRow>,
    /// What the page asks in place, and the daemon's last refusal on it.
    asking: RefCell<Asking>,
}

impl MandatePage {
    fn open(dialog: &Rc<MandateDialog>, row: MandateRow) -> adw::NavigationPage {
        assert!(
            !row.name.is_empty(),
            "a mandate's page is about a named mandate"
        );
        let holder = adw::Bin::new();
        let page = nav_page(&row.name, &holder, None);
        let mandate = Rc::new(MandatePage {
            dialog: dialog.clone(),
            name: row.name.clone(),
            holder,
            row: RefCell::new(row),
            asking: RefCell::default(),
        });
        let weak = Rc::downgrade(&mandate);
        page.connect_hidden(move |_| {
            let Some(mandate) = weak.upgrade() else {
                return;
            };
            mandate.asking.borrow_mut().withdraw();
            mandate.render();
        });
        mandate.render();
        keep_with(&page, mandate);
        page
    }

    /// The daemon's answer to a write replaces the row drawn.
    fn took(self: &Rc<Self>, row: MandateRow) {
        assert_eq!(row.name, self.name, "an answer about this mandate");
        self.row.replace(row);
        (self.dialog.deps.changed)();
        self.render();
    }

    fn render(self: &Rc<Self>) {
        let row = self.row.borrow().clone();
        let page = adw::PreferencesPage::new();
        page.add(&self.heading(&row));
        page.add(&what_it_allows(&row));
        page.add(&what_it_did(&row));
        page.add(&self.footer(&row));
        self.holder.set_child(Some(&page));
    }

    /// Its icon, its name, the daemon's sentence with its dot, and its switch where published.
    fn heading(self: &Rc<Self>, row: &MandateRow) -> adw::PreferencesGroup {
        let line = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&row.name))
            .subtitle(glib::markup_escape_text(&row.status_sentence))
            .activatable(false)
            .build();
        line.set_subtitle_lines(0);
        line.add_prefix(&gtk::Image::from_icon_name(MANDATE_ICON));
        line.add_suffix(&tone_dot(mandate_tone(row)));
        if mandate_switch(row) {
            line.add_suffix(&self.switch(row));
        }
        let group = adw::PreferencesGroup::new();
        group.add(&line);
        group
    }

    fn switch(self: &Rc<Self>, row: &MandateRow) -> gtk::Switch {
        let switch = gtk::Switch::builder()
            .active(row.enabled)
            .valign(gtk::Align::Center)
            .build();
        let spoken = words::switch_label(&row.name);
        switch.update_property(&[gtk::accessible::Property::Label(&spoken)]);
        let weak = Rc::downgrade(self);
        switch.connect_active_notify(move |switch| {
            switch.set_sensitive(false);
            if let Some(mandate) = weak.upgrade() {
                mandate.set_enabled(switch.is_active());
            }
        });
        switch
    }

    /// Approve, Edit and Remove mandate…, or the question asked in their place, and the daemon's
    /// last refusal on the page.
    fn footer(self: &Rc<Self>, row: &MandateRow) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        let line = adw::ActionRow::builder().activatable(false).build();
        let asking = self.asking.borrow().clone();
        if let Some(question) = asking.shown(&self.name) {
            let asked = question_text(question, &self.name, &[]);
            line.set_title(&glib::markup_escape_text(&asked));
            line.set_title_lines(0);
            let acting = asking.acting(&self.name);
            for button in question_buttons(question, acting, &self.answers()) {
                line.add_suffix(&button);
            }
        } else {
            self.buttons(&line, row);
        }
        group.add(&line);
        if let Some(refusal) = asking.refusal(&self.name) {
            group.add(&note_row(refusal, false, true));
        }
        group
    }

    fn buttons(self: &Rc<Self>, line: &adw::ActionRow, row: &MandateRow) {
        if offers_approve(row) {
            line.add_suffix(&self.asks(words::APPROVE, Question::Approve));
        }
        let edit = named_button(words::EDIT, &self.name);
        let (weak, opened) = (Rc::downgrade(self), row.clone());
        edit.connect_clicked(move |_| {
            if let Some(mandate) = weak.upgrade() {
                mandate.edit(&opened);
            }
        });
        line.add_suffix(&edit);
        let remove = self.asks(words::REMOVE_MANDATE, Question::RemoveMandate);
        set_class(&remove, "destructive-action", false);
        line.add_suffix(&remove);
    }

    /// A button whose first press asks `question` in the footer's place.
    fn asks(self: &Rc<Self>, label: &str, question: Question) -> gtk::Button {
        let ask = named_button(label, &self.name);
        let weak = Rc::downgrade(self);
        ask.connect_clicked(move |_| {
            if let Some(mandate) = weak.upgrade() {
                mandate.asking.borrow_mut().ask(&mandate.name, question);
                mandate.render();
            }
        });
        ask
    }

    fn edit(self: &Rc<Self>, row: &MandateRow) {
        let weak = Rc::downgrade(self);
        let took: Took = Box::new(move |row| {
            let Some(mandate) = weak.upgrade() else {
                return;
            };
            mandate.took(row);
            mandate.dialog.nav.pop();
        });
        let page = Editor::open(&self.dialog, MandateForm::editing(row), took);
        self.dialog.nav.push(&page);
    }

    // ---- questions asked in place ----

    fn answers(self: &Rc<Self>) -> QuestionActions {
        let (cancel, confirm) = (Rc::downgrade(self), Rc::downgrade(self));
        QuestionActions {
            cancel: Rc::new(move || {
                if let Some(mandate) = cancel.upgrade() {
                    mandate.asking.borrow_mut().withdraw();
                    mandate.render();
                }
            }),
            confirm: Rc::new(move || {
                if let Some(mandate) = confirm.upgrade() {
                    mandate.confirm();
                }
            }),
        }
    }

    fn confirm(self: &Rc<Self>) {
        let asked = self.asking.borrow_mut().confirm();
        let Some((_, question)) = asked else { return };
        self.render();
        let mandate = self.clone();
        glib::spawn_future_local(async move {
            let refusal = match question {
                Question::Approve => mandate.approve().await,
                Question::RemoveMandate => mandate.remove().await,
                Question::RemoveSource | Question::NewToken => {
                    unreachable!("a mandate's page asks only to approve or remove it")
                }
            };
            mandate.asking.borrow_mut().finished(&mandate.name, refusal);
            mandate.render();
        });
    }

    // ---- writing ----

    async fn approve(self: &Rc<Self>) -> Option<String> {
        let name = self.name.clone();
        let daemon = self.dialog.deps.daemon.clone();
        match daemon.call(move |m| m.cue_mandate_approve(&name)).await {
            Ok(answer) => {
                self.took(answer.mandate);
                None
            }
            Err(e) => Some(refusal_of(&e, "cues.mandates.approve").1),
        }
    }

    async fn remove(self: &Rc<Self>) -> Option<String> {
        let name = self.name.clone();
        let daemon = self.dialog.deps.daemon.clone();
        match daemon.call(move |m| m.cue_mandate_remove(&name)).await {
            Ok(_) => {
                (self.dialog.deps.changed)();
                self.dialog.dialog.close();
                None
            }
            Err(e) => Some(refusal_of(&e, "cues.mandates.remove").1),
        }
    }

    fn set_enabled(self: &Rc<Self>, on: bool) {
        let mandate = self.clone();
        glib::spawn_future_local(async move {
            let name = mandate.name.clone();
            let daemon = mandate.dialog.deps.daemon.clone();
            let answer = daemon.call(move |m| m.cue_mandate_switch(&name, on)).await;
            let refusal = answer
                .as_ref()
                .err()
                .map(|e| refusal_of(e, "switching a mandate").1);
            let target = mandate.name.clone();
            mandate
                .asking
                .borrow_mut()
                .answered(&target, refusal.as_deref());
            match answer {
                Ok(answer) => mandate.took(answer.mandate),
                Err(_) => mandate.render(),
            }
        });
    }
}

/// Its task, its mode with what the mode lets the task do, its tools, and its read tools and
/// validator where it has them.
fn what_it_allows(row: &MandateRow) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.add(&fact_row(words::TASK, &task_name(row)));
    let mode = fact_row(words::MODE, &row.consent_sentence);
    if let Some((tag, _)) = mode_tag(&row.mode) {
        mode.set_title(&glib::markup_escape_text(&words::middot(&[
            words::MODE.to_owned(),
            tag.to_owned(),
        ])));
    }
    // Acting without asking is a consequence the owner must read.
    set_class(&mode, "warning", row.mode.is(MandateMode::Auto));
    group.add(&mode);
    group.add(&fact_row(words::TOOLS, &words::list(&row.tools)));
    if !row.read_tools.is_empty() {
        group.add(&fact_row(words::READ_TOOLS, &words::list(&row.read_tools)));
    }
    if let Some(validator) = &row.validator {
        group.add(&fact_row(words::VALIDATOR, &validator.path));
    }
    group
}

/// How long it waits and what it acts on, its approval where the heading does not already say
/// it, and its last day.
fn what_it_did(row: &MandateRow) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.add(&fact_row(words::WAITS, &words::waits(row.approve_within_s)));
    group.add(&fact_row(
        words::ACTS_ON,
        &words::acts_on(row.max_cue_age_s),
    ));
    let approval = approval_line(row);
    if approval != row.status_sentence {
        group.add(&fact_row(words::APPROVAL, &approval));
    }
    group.add(&fact_row(
        words::LAST_DAY,
        &words::mandate_last_day(&row.counts),
    ));
    group
}

// ---- the editor ----

struct Editor {
    dialog: Rc<MandateDialog>,
    form: RefCell<MandateForm>,
    choices: RefCell<Option<Result<MandateChoices, String>>>,
    view: RefCell<Option<Rc<SectionView>>>,
    /// The mode as a toggle, drawn in place of the mode row.
    mode: adw::ActionRow,
    modes: adw::ToggleGroup,
    /// Why the choices could not be read, and a refusal naming no field.
    choices_line: adw::ActionRow,
    under: adw::ActionRow,
    send: gtk::Button,
    sending: Cell<bool>,
    soon: Soon,
    took: Took,
}

impl Editor {
    fn open(dialog: &Rc<MandateDialog>, form: MandateForm, took: Took) -> adw::NavigationPage {
        let adding = form.original().is_none();
        let send = gtk::Button::builder()
            .label(if adding {
                words::ADD_MANDATE
            } else {
                words::DONE
            })
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        let editor = Rc::new(Editor {
            dialog: dialog.clone(),
            form: RefCell::new(form),
            choices: RefCell::default(),
            view: RefCell::default(),
            mode: adw::ActionRow::builder().title(words::MODE).build(),
            modes: mode_toggles(),
            choices_line: hidden_line(),
            under: hidden_line(),
            send,
            sending: Cell::new(false),
            soon: Soon::default(),
            took,
        });
        editor.mode.add_suffix(&editor.modes);
        let view = editor_view(Rc::downgrade(&editor), &editor.mode);
        if editor.form.borrow().more_options {
            view.open_folded();
        }
        *editor.view.borrow_mut() = Some(view);
        let title = if adding {
            words::ADD_MANDATE_TITLE
        } else {
            words::EDIT
        };
        let page = nav_page(title, &editor.content(), Some(&editor.send));
        editor.wire();
        editor.redraw();
        editor.read_choices();
        keep_with(&page, editor);
        page
    }

    fn view(&self) -> Rc<SectionView> {
        self.view.borrow().clone().expect("built with its view")
    }

    fn content(&self) -> adw::PreferencesPage {
        let under = adw::PreferencesGroup::new();
        under.add(&self.choices_line);
        under.add(&self.under);
        let content = adw::PreferencesPage::new();
        content.add(&self.view().group);
        content.add(&under);
        content
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.modes.connect_active_name_notify(move |group| {
            let (Some(editor), Some(mode)) = (weak.upgrade(), group.active_name()) else {
                return;
            };
            if editor.form.borrow().mode != mode.as_str() {
                editor.set("mode", Value::from(mode.as_str()));
            }
        });
        let weak = Rc::downgrade(self);
        self.send.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.send();
            }
        });
    }

    fn read_choices(self: &Rc<Self>) {
        let editor = self.clone();
        glib::spawn_future_local(async move {
            let daemon = editor.dialog.deps.daemon.clone();
            let answer = daemon.call(|m| m.cue_mandate_choices()).await;
            let read = answer.map_err(|e| refusal_of(&e, "cues.mandates.choices").1);
            *editor.choices.borrow_mut() = Some(read);
            editor.redraw();
        });
    }

    /// One row changed. What changes the rows (a tool, a list, a refusal cleared) redraws after
    /// the signal that said so is over.
    fn set(self: &Rc<Self>, key: &str, value: Value) {
        let mut form = self.form.borrow_mut();
        let refused = form.row_refusal(key).is_some() || form.form_refusal().is_some();
        let listed = value.is_array() || key.starts_with("tool:");
        if !form.set(key, &value) {
            glib::g_warning!("fermix", "the mandate editor has no row {key}");
        }
        drop(form);
        if refused || listed {
            self.redraw_soon();
        }
        self.draw_send();
    }

    fn redraw_soon(self: &Rc<Self>) {
        self.soon.redraw(self, Editor::redraw);
    }

    fn redraw(&self) {
        let choices = self.choices.borrow();
        let listed = choices.as_ref().and_then(|c| c.as_ref().ok());
        let form = self.form.borrow();
        let rows = form.rows(listed);
        let errors = rows
            .iter()
            .filter_map(|r| Some((r.key.clone(), form.row_refusal(&r.key)?.to_owned())))
            .collect();
        let section = SectionRows {
            id: FORM_ID.into(),
            title: String::new(),
            rows,
        };
        let drawn = Drawn {
            rows: Some(section),
            unread: None,
            errors,
            listing_failed: false,
            locked: false,
        };
        self.view().show(drawn);
        self.draw_mode(&form);
        let unread = choices.as_ref().and_then(|c| c.as_ref().err());
        show_line(&self.choices_line, unread.map(String::as_str));
        show_line(&self.under, form.form_refusal());
        drop(form);
        self.draw_send();
    }

    /// The mode's toggle shows the form's mode, none for a word this build does not know, and
    /// a refusal of the mode under its title.
    fn draw_mode(&self, form: &MandateForm) {
        let known = [APPROVE, AUTO].contains(&form.mode.as_str());
        if !known {
            self.modes.set_active(gtk::INVALID_LIST_POSITION);
        } else if self.modes.active_name().as_deref() != Some(form.mode.as_str()) {
            self.modes.set_active_name(Some(&form.mode));
        }
        let refused = form.row_refusal("mode");
        self.mode
            .set_subtitle(&glib::markup_escape_text(refused.unwrap_or("")));
        set_class(&self.mode, "setting-refused", refused.is_some());
    }

    fn draw_send(&self) {
        let ready = self.form.borrow().is_complete();
        self.send.set_sensitive(ready && !self.sending.get());
    }

    // ---- sending ----

    /// An add sends every field; an edit sends what changed, and nothing at all goes back.
    fn send(self: &Rc<Self>) {
        let form = self.form.borrow();
        let choices = self.choices.borrow();
        let values = form.values(choices.as_ref().and_then(|c| c.as_ref().ok()));
        let (name, add) = (form.name.trim().to_owned(), form.original().is_none());
        drop((form, choices));
        if values.is_empty() && !add {
            self.dialog.nav.pop();
            return;
        }
        self.sending.set(true);
        self.draw_send();
        let editor = self.clone();
        glib::spawn_future_local(async move {
            let daemon = editor.dialog.deps.daemon.clone();
            let target = name.clone();
            let answer = daemon
                .call(move |m| m.cue_mandate_write(&target, values, add))
                .await;
            editor.sending.set(false);
            match answer {
                Ok(answer) => (editor.took)(answer.mandate),
                Err(e) => editor.refused(&e, add),
            }
        });
    }

    fn refused(&self, e: &fermix_client::management::CallError, add: bool) {
        let what = if add {
            "cues.mandates.add"
        } else {
            "cues.mandates.update"
        };
        let (field, sentence) = refusal_of(e, what);
        self.form.borrow_mut().refuse(field.as_deref(), &sentence);
        self.redraw();
    }
}

fn mode_toggles() -> adw::ToggleGroup {
    let group = adw::ToggleGroup::builder()
        .valign(gtk::Align::Center)
        .build();
    for (name, label) in [(APPROVE, words::MODE_APPROVE), (AUTO, words::MODE_AUTO)] {
        group.add(adw::Toggle::builder().name(name).label(label).build());
    }
    group
}

/// The editor's rows, writing into its form as they change. A row of an editor already gone
/// writes nowhere.
fn editor_view(editor: Weak<Editor>, mode: &adw::ActionRow) -> Rc<SectionView> {
    let mode = Extra {
        key: "mode".to_owned(),
        place: Place::Instead,
        row: mode.clone().upcast(),
    };
    SectionView::form(
        FORM_ID,
        FormSink {
            value: Box::new(move |key, value| {
                if let Some(editor) = editor.upgrade() {
                    editor.set(key, value);
                }
            }),
            secret: Box::new(|key, _| {
                glib::g_warning!("fermix", "the mandate editor has no secret row {key}");
            }),
            folded: words::MORE_OPTIONS,
        },
        vec![mode],
    )
}
