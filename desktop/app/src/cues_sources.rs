//! The source dialog (M60 cues §4.4): one dialog whose pages are the add form, a source's page
//! (`cues_source_page.rs`) and the edit form. Nothing is raised over it but the keyring's one
//! question. The forms are the daemon's rows drawn in the descriptor's form mode; what is set is
//! held in `SourceFormState` until Add or Done, and a typed secret goes to `secret.set` once the
//! source exists, never with it.

use crate::cues_dialog::{first_page, hidden_line, nav_page, page_dialog, show_line, Soon};
use crate::cues_source_page::SourcePage;
use crate::daemon::Daemon;
use crate::descriptor::{info_button, Drawn, FormSink, SectionView};
use crate::dialogs::keep_with;
use crate::secret_save::set_secret;
use adw::prelude::*;
use fermix_client::cues::calls::FormChoice;
use fermix_client::cues::form::{
    AddFlow, AddSpec, Arrival, SourceFormState, TypedSecret, VERIFY_KEY,
};
use fermix_client::cues::wire::{SecretField, SourceDetail, SourceForm};
use fermix_client::cues::words;
use fermix_client::management::CallError;
use fermix_client::settings::{Kind, SectionRows};
use fermix_client::view::daemon_problem;
use gtk::glib;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// A form is asked for once the choice has been still this long.
const FORM_PAUSE: Duration = Duration::from_millis(250);
/// The id the form's rows are drawn under; nothing is read or written by it.
const FORM_ID: &str = "cues.source.form";

/// What the dialog reaches outside itself.
pub struct SourceDeps {
    pub daemon: Daemon,
    pub toast: Rc<dyn Fn(&str)>,
    /// A source was added, changed or removed: the pane reads its list and status again.
    pub changed: Rc<dyn Fn()>,
    /// The door's port, once the status gave it, for the local push example.
    pub port: Rc<dyn Fn() -> Option<u16>>,
}

pub struct SourceDialog {
    pub deps: SourceDeps,
    pub dialog: adw::Dialog,
    pub nav: adw::NavigationView,
}

impl SourceDialog {
    fn new(deps: SourceDeps, title: &str) -> Rc<SourceDialog> {
        let (dialog, nav) = page_dialog(title);
        Rc::new(SourceDialog { deps, dialog, nav })
    }

    /// The add form, over `parent`.
    pub fn open_add(parent: &impl IsA<gtk::Widget>, deps: SourceDeps) {
        let dialog = SourceDialog::new(deps, words::ADD_SOURCE_TITLE);
        let form = AddForm::new(&dialog);
        let page = form.page();
        keep_with(&page, form.clone());
        first_page(&dialog.nav, &page);
        dialog.dialog.present(Some(parent));
        form.ask(FormChoice::default());
    }

    /// A source's page, opened at the secret or token its row's action points at.
    pub fn open_source(
        parent: &impl IsA<gtk::Widget>,
        deps: SourceDeps,
        name: &str,
        at: Option<SecretField>,
    ) {
        let dialog = SourceDialog::new(deps, name);
        let page = SourcePage::open(&dialog, name, at, Arrival::default());
        first_page(&dialog.nav, &page);
        dialog.dialog.present(Some(parent));
    }

    /// After an add, the form and what it held go, and the source's page takes their place.
    fn turn_to_source(self: &Rc<Self>, name: &str, arrival: Arrival) {
        let page = SourcePage::open(self, name, arrival.at(), arrival);
        self.dialog.set_title(name);
        first_page(&self.nav, &page);
    }
}

/// The daemon's sentence for a refused call, and the field it names; anything else is logged
/// and said as what became of the daemon.
pub fn refusal_of(e: &CallError, what: &str) -> (Option<String>, String) {
    if let CallError::Refused(r) = e {
        return (r.field.clone(), r.sentence.clone());
    }
    glib::g_warning!("fermix", "{what} failed: {e:?}");
    (None, daemon_problem(e).sentence())
}

/// The form's rows as drawn: each holding what was set in it, and each refusal under its row.
fn form_drawn(state: &SourceFormState) -> Drawn {
    let rows = state
        .rows()
        .iter()
        .map(|row| {
            let mut row = row.clone();
            row.value = state.value_of(&row);
            row
        })
        .collect();
    let errors = state
        .rows()
        .iter()
        .filter_map(|r| Some((r.key.clone(), state.refusal_for(&r.key)?.to_owned())))
        .collect();
    Drawn {
        rows: Some(SectionRows {
            id: FORM_ID.into(),
            title: String::new(),
            rows,
        }),
        unread: None,
        errors,
        listing_failed: false,
        locked: false,
    }
}

/// A form's rows, writing into its state as they change. What changes the rows (a check chosen,
/// a list changed, a refusal cleared) redraws after the signal that said so is over. A row of a
/// form already gone writes nowhere.
fn form_view(core: Weak<FormCore>) -> Rc<SectionView> {
    let secrets = core.clone();
    SectionView::form(
        FORM_ID,
        FormSink {
            value: Box::new(move |key, value| {
                if let Some(core) = core.upgrade() {
                    core.set(key, value);
                }
            }),
            secret: Box::new(move |key, text| {
                if let Some(core) = secrets.upgrade() {
                    core.hold(key, &text);
                }
            }),
            folded: words::MORE_OPTIONS,
        },
        Vec::new(),
    )
}

/// What a form does once a value changed, with the key that changed.
type Changed = Box<dyn Fn(&str)>;

/// What the add and edit forms share: the state, its view, the line under it, and the pause
/// before a redraw.
pub struct FormCore {
    state: RefCell<SourceFormState>,
    view: RefCell<Option<Rc<SectionView>>>,
    under: adw::ActionRow,
    soon: Soon,
    /// Called once a value changed, outside any signal: asks for the form again on a new check.
    changed: RefCell<Option<Changed>>,
}

impl FormCore {
    fn new(state: SourceFormState) -> Rc<FormCore> {
        let core = Rc::new(FormCore {
            state: RefCell::new(state),
            view: RefCell::default(),
            under: hidden_line(),
            soon: Soon::default(),
            changed: RefCell::default(),
        });
        *core.view.borrow_mut() = Some(form_view(Rc::downgrade(&core)));
        core
    }

    fn view(&self) -> Rc<SectionView> {
        self.view.borrow().clone().expect("built with its view")
    }

    fn set(self: &Rc<Self>, key: &str, value: Value) {
        let mut state = self.state.borrow_mut();
        let refused = state.refusal_for(key).is_some();
        let list = state
            .rows()
            .iter()
            .any(|r| r.key == key && r.kind == Kind::List);
        state.set(key, value);
        drop(state);
        if refused || list || key == VERIFY_KEY {
            self.redraw_soon();
        }
        if let Some(changed) = self.changed.borrow().as_ref() {
            changed(key);
        }
    }

    fn hold(self: &Rc<Self>, key: &str, text: &str) {
        let refused = self.state.borrow().refusal_for(key).is_some();
        self.state.borrow_mut().hold_secret(key, text);
        if refused {
            self.redraw_soon();
        }
    }

    fn redraw_soon(self: &Rc<Self>) {
        self.soon.redraw(self, FormCore::redraw);
    }

    fn redraw(&self) {
        let drawn = form_drawn(&self.state.borrow());
        self.view().show(drawn);
        let state = self.state.borrow();
        show_line(&self.under, state.form_refusal());
    }
}

// ---- the add form ----

struct AddForm {
    dialog: Rc<SourceDialog>,
    core: Rc<FormCore>,
    kinds: adw::ToggleGroup,
    kind_line: adw::ActionRow,
    about: RefCell<Option<gtk::MenuButton>>,
    preset: adw::ComboRow,
    /// The preset each combo item stands for; None is "None".
    preset_ids: RefCell<Vec<Option<String>>>,
    pickers_note: adw::ActionRow,
    add: gtk::Button,
    /// The latest choice's ticket: a request superseded during the pause is not sent.
    ticket: Cell<u64>,
    filling: Cell<bool>,
    sending: Cell<bool>,
}

impl AddForm {
    fn new(dialog: &Rc<SourceDialog>) -> Rc<AddForm> {
        let add = gtk::Button::builder()
            .label(words::ADD_SOURCE)
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        let form = Rc::new(AddForm {
            dialog: dialog.clone(),
            core: FormCore::new(SourceFormState::default()),
            kinds: adw::ToggleGroup::builder()
                .halign(gtk::Align::Start)
                .build(),
            kind_line: adw::ActionRow::builder().activatable(false).build(),
            about: RefCell::default(),
            preset: adw::ComboRow::builder().title(words::PRESET).build(),
            preset_ids: RefCell::default(),
            pickers_note: hidden_line(),
            add,
            ticket: Cell::new(0),
            filling: Cell::new(false),
            sending: Cell::new(false),
        });
        form.kind_line.set_title_lines(0);
        form.wire();
        form
    }

    fn page(self: &Rc<Self>) -> adw::NavigationPage {
        // The kinds sit above the list their choice fills, outside it.
        let kinds = adw::PreferencesGroup::new();
        kinds.add(&self.kinds);
        let pickers = adw::PreferencesGroup::new();
        pickers.add(&self.kind_line);
        pickers.add(&self.preset);
        pickers.add(&self.pickers_note);
        let under = adw::PreferencesGroup::new();
        under.add(&self.core.under);
        let content = adw::PreferencesPage::new();
        content.add(&kinds);
        content.add(&pickers);
        content.add(&self.core.view().group);
        content.add(&under);
        nav_page(words::ADD_SOURCE_TITLE, &content, Some(&self.add))
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.kinds.connect_active_name_notify(move |group| {
            let Some(form) = weak.upgrade().filter(|f| !f.filling.get()) else {
                return;
            };
            if let Some(kind) = group.active_name() {
                form.core.state.borrow_mut().choose_kind(&kind);
                form.chose();
            }
        });
        let weak = Rc::downgrade(self);
        self.preset.connect_selected_notify(move |combo| {
            let Some(form) = weak.upgrade().filter(|f| !f.filling.get()) else {
                return;
            };
            let index = usize::try_from(combo.selected()).expect("an index");
            let preset = form.preset_ids.borrow().get(index).cloned().flatten();
            form.core
                .state
                .borrow_mut()
                .choose_preset(preset.as_deref());
            form.chose();
        });
        let weak = Rc::downgrade(self);
        *self.core.changed.borrow_mut() = Some(Box::new(move |key| {
            let Some(form) = weak.upgrade() else { return };
            if key == VERIFY_KEY {
                form.chose();
            }
            form.draw_add();
        }));
        let weak = Rc::downgrade(self);
        self.add.connect_clicked(move |_| {
            if let Some(form) = weak.upgrade() {
                form.send();
            }
        });
    }

    /// A choice changed: draw what is known now, and ask for its form after the pause.
    fn chose(self: &Rc<Self>) {
        let ticket = self.ticket.get() + 1;
        self.ticket.set(ticket);
        let form = self.clone();
        glib::idle_add_local_once(move || form.draw());
        let form = self.clone();
        glib::timeout_add_local_once(FORM_PAUSE, move || {
            if form.ticket.get() == ticket {
                let choice = form.core.state.borrow().request();
                form.ask(choice);
            }
        });
    }

    /// Asks for the form for `choice`. Only the answer to the choice held when it lands is drawn.
    fn ask(self: &Rc<Self>, choice: FormChoice) {
        let form = self.clone();
        glib::spawn_future_local(async move {
            let asked = choice.clone();
            let daemon = form.dialog.deps.daemon.clone();
            let answer = daemon.call(move |m| m.cue_source_form(&asked)).await;
            match answer {
                Ok(answer) => form.took(&answer, &choice),
                Err(e) => {
                    let (field, sentence) = refusal_of(&e, "cues.sources.form");
                    form.core
                        .state
                        .borrow_mut()
                        .refuse(field.as_deref(), &sentence);
                }
            }
            form.draw();
        });
    }

    /// The daemon's form for `asked`. The first answer only names the kinds and presets: the
    /// form for the first kind and its first preset is asked for next.
    fn took(self: &Rc<Self>, answer: &SourceForm, asked: &FormChoice) {
        let mut state = self.core.state.borrow_mut();
        state.take(answer, asked);
        let next = state.request();
        drop(state);
        if next != *asked {
            self.chose();
        }
    }

    fn draw(&self) {
        self.draw_kinds();
        self.draw_presets();
        self.core.redraw();
        let state = self.core.state.borrow();
        let picked = state.refusal_for("kind").or(state.refusal_for("preset"));
        show_line(&self.pickers_note, picked);
        drop(state);
        self.draw_add();
    }

    fn draw_add(&self) {
        let named = !self.core.state.borrow().name().is_empty();
        self.add.set_sensitive(named && !self.sending.get());
    }

    /// The kinds as toggles, the chosen kind's line, and the consent line and notes behind (i).
    fn draw_kinds(&self) {
        let state = self.core.state.borrow();
        self.filling.set(true);
        if self.kinds.n_toggles() as usize != state.kinds().len() {
            for kind in state.kinds() {
                let toggle = adw::Toggle::builder()
                    .name(&kind.id)
                    .label(&kind.label)
                    .build();
                self.kinds.add(toggle);
            }
        }
        self.kinds.set_active_name(state.kind());
        self.filling.set(false);
        let line = state
            .kinds()
            .iter()
            .find(|k| Some(k.id.as_str()) == state.kind())
            .map(|k| k.sentence.clone());
        self.kind_line
            .set_title(&glib::markup_escape_text(line.as_deref().unwrap_or("")));
        self.kind_line.set_visible(line.is_some());
        if let Some(old) = self.about.borrow_mut().take() {
            self.kind_line.remove(&old);
        }
        if let Some(about) = state.about() {
            let more = info_button(&about, words::MORE_ABOUT);
            self.kind_line.add_suffix(&more);
            *self.about.borrow_mut() = Some(more);
        }
    }

    /// The preset combo, with "None" first, where the kind has presets.
    fn draw_presets(&self) {
        let state = self.core.state.borrow();
        let presets = state.kind_presets();
        let mut ids: Vec<Option<String>> = vec![None];
        ids.extend(presets.iter().map(|p| Some(p.id.clone())));
        let mut labels = vec![words::NONE.to_owned()];
        labels.extend(presets.iter().map(|p| p.label.clone()));
        let selected = ids
            .iter()
            .position(|id| id.as_deref() == state.preset())
            .unwrap_or(0);
        self.preset.set_visible(!presets.is_empty());
        self.filling.set(true);
        if *self.preset_ids.borrow() != ids {
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            self.preset.set_model(Some(&gtk::StringList::new(&labels)));
            *self.preset_ids.borrow_mut() = ids;
        }
        self.preset
            .set_selected(u32::try_from(selected).expect("few presets"));
        self.filling.set(false);
    }

    // ---- adding ----

    fn send(self: &Rc<Self>) {
        let prepared = self.core.state.borrow_mut().prepare_add();
        let Some((spec, secrets)) = prepared else {
            self.draw();
            return;
        };
        self.sending.set(true);
        self.draw_add();
        let form = self.clone();
        glib::spawn_future_local(async move {
            form.add(spec, secrets).await;
            form.sending.set(false);
            form.draw_add();
        });
    }

    async fn add(self: &Rc<Self>, spec: AddSpec, secrets: Vec<TypedSecret>) {
        let daemon = self.dialog.deps.daemon.clone();
        let (name, choice, values) = (spec.name.clone(), spec.choice.clone(), spec.values);
        let answer = daemon
            .call(move |m| m.cue_source_add(&name, &choice, values))
            .await;
        let added = match answer {
            Ok(added) => added,
            Err(e) => {
                let (field, sentence) = refusal_of(&e, "cues.sources.add");
                let mut state = self.core.state.borrow_mut();
                state.refuse(field.as_deref(), &sentence);
                drop(state);
                return self.draw();
            }
        };
        // The flow gives out one secret at a time and none after a refusal.
        let mut flow = AddFlow::added(&spec.name, added.token, secrets);
        while let Some((id, value)) = flow.next_secret() {
            let (daemon, parent) = (&self.dialog.deps.daemon, &self.dialog.dialog);
            let answer = set_secret(daemon, parent, id, value.reveal().to_owned()).await;
            flow.stored(
                answer
                    .err()
                    .map(|e| refusal_of(&e, "secret.set for a new source").1),
            );
        }
        (self.dialog.deps.changed)();
        self.dialog.turn_to_source(&spec.name, flow.arrival());
    }
}

// ---- the edit form ----

/// The source's own fields, its name and kind fixed and its secrets left to its page. Done sends
/// only what changed; a refusal stays under its field.
pub fn edit_page(dialog: &Rc<SourceDialog>, detail: &SourceDetail) -> adw::NavigationPage {
    let core = FormCore::new(SourceFormState::editing(detail));
    let done = gtk::Button::builder()
        .label(words::DONE)
        .css_classes(["suggested-action"])
        .build();
    let content = adw::PreferencesPage::new();
    content.add(&core.view().group);
    let under = adw::PreferencesGroup::new();
    under.add(&core.under);
    content.add(&under);
    core.redraw();
    let name = detail.source.name.clone();
    let page = nav_page(&name, &content, Some(&done));
    let (dialog, keep) = (dialog.clone(), core.clone());
    done.connect_clicked(move |button| {
        button.set_sensitive(false);
        let (dialog, core, name, button) =
            (dialog.clone(), keep.clone(), name.clone(), button.clone());
        glib::spawn_future_local(async move {
            update(&dialog, &core, &name).await;
            button.set_sensitive(true);
        });
    });
    page
}

async fn update(dialog: &Rc<SourceDialog>, core: &Rc<FormCore>, name: &str) {
    let values = core.state.borrow().changed_values();
    if values.is_empty() {
        dialog.nav.pop();
        return;
    }
    let target = name.to_owned();
    let answer = dialog
        .deps
        .daemon
        .call(move |m| m.cue_source_update(&target, values))
        .await;
    match answer {
        Ok(_) => {
            (dialog.deps.changed)();
            dialog.nav.pop();
        }
        Err(e) => {
            let (field, sentence) = refusal_of(&e, "cues.sources.update");
            core.state.borrow_mut().refuse(field.as_deref(), &sentence);
            core.redraw();
        }
    }
}
