//! A source's page (M60 cues §4.4): its line and switch, its secrets first (the one its row's
//! action points at leading), a token shown once, its facts, what it refused or missed, and Edit
//! and Remove source…, which asks in place. A webhook's address is never drawn: Copy URL asks for
//! it on each press and copies its path. A token is dropped from the page when the page turns.

use crate::clipboard::SecretCopy;
use crate::cues_dialog::{alone, named_button, nav_page, reading};
use crate::cues_rows::{
    fact_row, note_row, question_buttons, set_class, tone_dot, QuestionActions,
};
use crate::cues_sources::{edit_page, refusal_of, SourceDialog};
use crate::descriptor::info_button;
use crate::dialogs::keep_with;
use crate::fields::{secret_row, Live, Slot, Store};
use crate::secret_save::set_secret;
use adw::prelude::*;
use fermix_client::cues::form::{secret_id, Arrival};
use fermix_client::cues::view::{
    has, kind_icon, question_text, read_failure, secrets_first, source_switch, source_tone,
    source_url, Asking, Question,
};
use fermix_client::cues::wire::{
    HandedOut, SecretField, SecretSlot, SourceAction, SourceDetail, SourceRow,
};
use fermix_client::cues::words;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct SourcePage {
    dialog: Rc<SourceDialog>,
    name: String,
    at: Option<SecretField>,
    /// The page's content. The page itself is not held here: the page holds this.
    holder: adw::Bin,
    detail: RefCell<Option<Result<SourceDetail, String>>>,
    /// What the page asks in place, and the daemon's last refusal on it, said under its buttons.
    asking: RefCell<Asking>,
    unstored: RefCell<Option<(SecretField, String)>>,
    /// A token handed out on this page, shown once; it goes when the page turns.
    token: RefCell<Option<HandedOut>>,
    copies: SecretCopy,
    live: RefCell<Live>,
}

impl SourcePage {
    /// The page of the source `name`, opened at `at`, with what an add left for it. The page
    /// owns what it draws from.
    pub fn open(
        dialog: &Rc<SourceDialog>,
        name: &str,
        at: Option<SecretField>,
        arrival: Arrival,
    ) -> adw::NavigationPage {
        assert!(!name.is_empty(), "a source's page is about a named source");
        let holder = adw::Bin::new();
        holder.set_child(Some(&reading()));
        let page = nav_page(name, &holder, None);
        let source = Rc::new(SourcePage {
            dialog: dialog.clone(),
            name: name.to_owned(),
            at,
            holder,
            detail: RefCell::default(),
            asking: RefCell::default(),
            unstored: RefCell::new(arrival.unstored),
            token: RefCell::new(arrival.token),
            copies: SecretCopy::default(),
            live: RefCell::new(Rc::new(Cell::new(true))),
        });
        source.follow(&page);
        keep_with(&page, source.clone());
        source.read();
        page
    }

    /// The page reads its source again as it comes back into view, and lets go of a token and a
    /// copy as it turns.
    fn follow(self: &Rc<Self>, page: &adw::NavigationPage) {
        let weak = Rc::downgrade(self);
        page.connect_hidden(move |_| {
            let Some(source) = weak.upgrade() else { return };
            source.token.replace(None);
            source.copies.take_back();
            source.asking.borrow_mut().withdraw();
            source.render();
        });
        let weak = Rc::downgrade(self);
        page.connect_showing(move |_| {
            let Some(source) = weak.upgrade() else { return };
            if source.detail.borrow().is_some() {
                source.read();
            }
        });
    }

    fn read(self: &Rc<Self>) {
        let source = self.clone();
        glib::spawn_future_local(async move {
            let name = source.name.clone();
            let answer = source
                .dialog
                .deps
                .daemon
                .call(move |m| m.cue_source(&name))
                .await;
            let read = answer.map_err(|e| {
                glib::g_warning!("fermix", "cues.sources.get failed: {e:?}");
                read_failure(&e, "ref", words::SOURCE_GONE)
            });
            *source.detail.borrow_mut() = Some(read);
            source.render();
        });
    }

    /// Reads the page and the pane's list again after a change here.
    fn changed(self: &Rc<Self>) {
        self.read();
        (self.dialog.deps.changed)();
    }

    // ---- drawing ----

    fn render(self: &Rc<Self>) {
        self.live.borrow().set(false);
        *self.live.borrow_mut() = Rc::new(Cell::new(true));
        let detail = self.detail.borrow().clone();
        let content = match &detail {
            None => reading(),
            Some(Err(sentence)) => alone(&note_row(sentence, false, true)),
            Some(Ok(detail)) => self.content(detail),
        };
        self.holder.set_child(Some(&content));
    }

    fn content(self: &Rc<Self>, detail: &SourceDetail) -> adw::PreferencesPage {
        let source = &detail.source;
        let page = adw::PreferencesPage::new();
        page.add(&self.heading(source));
        if !source.secrets.is_empty() {
            page.add(&self.secrets(source));
        }
        if let Some(token) = self.token.borrow().clone() {
            page.add(&self.shown_once(token));
        }
        page.add(&facts(source));
        if let Some(missed) = missed(detail) {
            page.add(&missed);
        }
        page.add(&self.footer(detail));
        page
    }

    /// The kind's icon, the name, the daemon's sentence with its dot, and the switch where the
    /// daemon publishes one.
    fn heading(self: &Rc<Self>, source: &SourceRow) -> adw::PreferencesGroup {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&source.name))
            .subtitle(glib::markup_escape_text(&source.status_sentence))
            .activatable(false)
            .build();
        row.set_subtitle_lines(0);
        row.add_prefix(&gtk::Image::from_icon_name(kind_icon(&source.kind)));
        row.add_suffix(&tone_dot(source_tone(source)));
        if source_switch(source) {
            row.add_suffix(&self.switch(source));
        }
        let group = adw::PreferencesGroup::new();
        group.add(&row);
        group
    }

    fn switch(self: &Rc<Self>, source: &SourceRow) -> gtk::Switch {
        let switch = gtk::Switch::builder()
            .active(source.enabled)
            .valign(gtk::Align::Center)
            .build();
        switch.update_property(&[gtk::accessible::Property::Label(&words::switch_label(
            &source.name,
        ))]);
        let weak = Rc::downgrade(self);
        switch.connect_active_notify(move |switch| {
            switch.set_sensitive(false);
            if let Some(source) = weak.upgrade() {
                source.set_enabled(switch.is_active());
            }
        });
        switch
    }

    fn secrets(self: &Rc<Self>, source: &SourceRow) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        for slot in secrets_first(source, self.at) {
            let row = match slot.field.known() {
                Some(field @ (SecretField::Key | SecretField::Secret)) => {
                    self.typed_secret(slot, field).upcast()
                }
                Some(SecretField::Token) => self.token_row(slot, source).upcast(),
                Some(SecretField::PathSecret) => self.address_row(slot, source).upcast(),
                None => fact_row(&slot.label, stored(slot.present)).upcast::<gtk::Widget>(),
            };
            group.add(&row);
        }
        group
    }

    /// A secret the owner types: Stored, with Replace… and Remove, under its cue id. A secret
    /// the add could not store says why under it.
    fn typed_secret(self: &Rc<Self>, slot: &SecretSlot, field: SecretField) -> adw::ActionRow {
        let action = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&slot.label))
            .build();
        let id = secret_id(&self.name, field);
        let (weak, stored_id) = (Rc::downgrade(self), id.clone());
        let store: Store = Rc::new(move |entry, value| {
            if let Some(source) = weak.upgrade() {
                source.store(entry, stored_id.clone(), value);
            }
        });
        let weak = Rc::downgrade(self);
        let remove: Rc<dyn Fn(&gtk::Button)> = Rc::new(move |_| {
            if let Some(source) = weak.upgrade() {
                source.clear(id.clone());
            }
        });
        let slot = Slot {
            label: slot.label.clone(),
            present: slot.present,
            store,
            remove,
        };
        secret_row(&action, slot, &self.live.borrow());
        if let Some((_, sentence)) = self.unstored.borrow().as_ref().filter(|(f, _)| *f == field) {
            action.set_subtitle(&glib::markup_escape_text(sentence));
            action.add_css_class("setting-refused");
        }
        action
    }

    /// The token: whether one is stored, and New token… where the daemon offers one, which asks
    /// in place before the old one stops working.
    fn token_row(self: &Rc<Self>, slot: &SecretSlot, source: &SourceRow) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&slot.label))
            .subtitle(stored(slot.present))
            .activatable(false)
            .build();
        if self.asking.borrow().shown(&self.name) == Some(Question::NewToken) {
            row.set_subtitle(&glib::markup_escape_text(&question_text(
                Question::NewToken,
                &self.name,
                &[],
            )));
            row.set_subtitle_lines(0);
            let acting = self.asking.borrow().acting(&self.name);
            for button in question_buttons(Question::NewToken, acting, &self.answers()) {
                row.add_suffix(&button);
            }
            return row;
        }
        if has(&source.actions, SourceAction::NewToken) {
            let ask = named_button(words::NEW_TOKEN, &source.name);
            let weak = Rc::downgrade(self);
            ask.connect_clicked(move |_| {
                if let Some(source) = weak.upgrade() {
                    source.ask(Question::NewToken);
                }
            });
            row.add_suffix(&ask);
        }
        row
    }

    /// The address: whether it is made, and Copy URL where the daemon offers it, with where it
    /// goes behind the (i). The address itself is never drawn and never kept.
    fn address_row(self: &Rc<Self>, slot: &SecretSlot, source: &SourceRow) -> adw::ActionRow {
        let made = if source.has_url {
            words::ADDRESS_MADE
        } else {
            words::ADDRESS_NOT_MADE
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&slot.label))
            .subtitle(made)
            .activatable(false)
            .build();
        row.add_suffix(&info_button(words::ADDRESS_INFO, words::MORE_ABOUT));
        if has(&source.actions, SourceAction::GetUrl) {
            let copy = named_button(words::COPY_URL, &source.name);
            let weak = Rc::downgrade(self);
            copy.connect_clicked(move |button| {
                if let Some(source) = weak.upgrade() {
                    source.copy_address(button);
                }
            });
            row.add_suffix(&copy);
        }
        row
    }

    /// Edit and Remove source…, or the removal asked in their place, and the daemon's last
    /// refusal on this page.
    fn footer(self: &Rc<Self>, detail: &SourceDetail) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        let row = adw::ActionRow::builder().activatable(false).build();
        if self.asking.borrow().shown(&self.name) == Some(Question::RemoveSource) {
            let asked = question_text(Question::RemoveSource, &self.name, &detail.source.listening);
            row.set_title(&glib::markup_escape_text(&asked));
            row.set_title_lines(0);
            let acting = self.asking.borrow().acting(&self.name);
            for button in question_buttons(Question::RemoveSource, acting, &self.answers()) {
                row.add_suffix(&button);
            }
        } else {
            row.add_suffix(&self.edit_button(detail));
            row.add_suffix(&self.remove_button());
        }
        group.add(&row);
        if let Some(refusal) = self.asking.borrow().refusal(&self.name) {
            group.add(&note_row(refusal, false, true));
        }
        group
    }

    fn edit_button(self: &Rc<Self>, detail: &SourceDetail) -> gtk::Button {
        let edit = named_button(words::EDIT, &self.name);
        let (weak, detail) = (Rc::downgrade(self), detail.clone());
        edit.connect_clicked(move |_| {
            if let Some(source) = weak.upgrade() {
                let page = edit_page(&source.dialog, &detail);
                source.dialog.nav.push(&page);
            }
        });
        edit
    }

    fn remove_button(self: &Rc<Self>) -> gtk::Button {
        let remove = named_button(words::REMOVE_SOURCE, &self.name);
        set_class(&remove, "destructive-action", false);
        let weak = Rc::downgrade(self);
        remove.connect_clicked(move |_| {
            if let Some(source) = weak.upgrade() {
                source.ask(Question::RemoveSource);
            }
        });
        remove
    }

    // ---- questions asked in place ----

    fn answers(self: &Rc<Self>) -> QuestionActions {
        let (cancel, confirm) = (Rc::downgrade(self), Rc::downgrade(self));
        QuestionActions {
            cancel: Rc::new(move || {
                if let Some(source) = cancel.upgrade() {
                    source.asking.borrow_mut().withdraw();
                    source.render();
                }
            }),
            confirm: Rc::new(move || {
                if let Some(source) = confirm.upgrade() {
                    source.confirm();
                }
            }),
        }
    }

    fn ask(self: &Rc<Self>, question: Question) {
        self.asking.borrow_mut().ask(&self.name, question);
        self.render();
    }

    /// The daemon's answer to a write on this page: its refusal said under the buttons, or none.
    fn note(&self, refusal: Option<String>) {
        let mut asking = self.asking.borrow_mut();
        asking.answered(&self.name, refusal.as_deref());
    }

    fn confirm(self: &Rc<Self>) {
        let asked = self.asking.borrow_mut().confirm();
        let Some((_, question)) = asked else {
            return;
        };
        self.render();
        let source = self.clone();
        glib::spawn_future_local(async move {
            let refusal = match question {
                Question::NewToken => source.new_token().await,
                Question::RemoveSource => source.remove().await,
                Question::Approve | Question::RemoveMandate => {
                    unreachable!("a source's page asks only for a token or a removal")
                }
            };
            source.asking.borrow_mut().finished(&source.name, refusal);
            source.render();
        });
    }

    // ---- writing ----

    fn set_enabled(self: &Rc<Self>, on: bool) {
        let source = self.clone();
        glib::spawn_future_local(async move {
            let name = source.name.clone();
            let answer = source
                .dialog
                .deps
                .daemon
                .call(move |m| m.cue_source_switch(&name, on))
                .await;
            let refusal = answer.err().map(|e| refusal_of(&e, "switching a source").1);
            source.note(refusal);
            source.changed();
        });
    }

    fn store(self: &Rc<Self>, entry: &gtk::PasswordEntry, id: String, value: String) {
        let (source, entry) = (self.clone(), entry.clone());
        glib::spawn_future_local(async move {
            let answer = set_secret(&source.dialog.deps.daemon, &entry, id, value).await;
            let refusal = answer
                .err()
                .map(|e| refusal_of(&e, "secret.set for a source").1);
            source.unstored.replace(None);
            source.note(refusal);
            source.changed();
        });
    }

    fn clear(self: &Rc<Self>, id: String) {
        let source = self.clone();
        glib::spawn_future_local(async move {
            let answer = source
                .dialog
                .deps
                .daemon
                .call(move |m| m.secret_clear(&id))
                .await;
            let refusal = answer
                .err()
                .map(|e| refusal_of(&e, "secret.clear for a source").1);
            source.note(refusal);
            source.changed();
        });
    }

    /// A fresh token, shown once on this page.
    async fn new_token(self: &Rc<Self>) -> Option<String> {
        let name = self.name.clone();
        let answer = self
            .dialog
            .deps
            .daemon
            .call(move |m| m.cue_source_token_new(&name))
            .await;
        match answer {
            Ok(made) => {
                self.token.replace(Some(made.token));
                self.changed();
                None
            }
            Err(e) => Some(refusal_of(&e, "cues.sources.token.new").1),
        }
    }

    async fn remove(self: &Rc<Self>) -> Option<String> {
        let name = self.name.clone();
        let answer = self
            .dialog
            .deps
            .daemon
            .call(move |m| m.cue_source_remove(&name))
            .await;
        match answer {
            Ok(removed) => {
                removed
                    .left
                    .iter()
                    .for_each(|left| (self.dialog.deps.toast)(left));
                (self.dialog.deps.changed)();
                self.dialog.dialog.close();
                None
            }
            Err(e) => Some(refusal_of(&e, "cues.sources.remove").1),
        }
    }

    /// Asks for the address and copies its path, by the token's clipboard rules. Nothing of it
    /// is kept here.
    fn copy_address(self: &Rc<Self>, button: &gtk::Button) {
        button.set_sensitive(false);
        let (source, button) = (self.clone(), button.clone());
        glib::spawn_future_local(async move {
            let name = source.name.clone();
            let answer = source
                .dialog
                .deps
                .daemon
                .call(move |m| m.cue_source_address(&name))
                .await;
            let refusal = match answer {
                Ok(made) => source.copy(&button, made.path.reveal(), "a webhook address"),
                Err(e) => Some(refusal_of(&e, "cues.sources.url.get").1),
            };
            button.set_sensitive(true);
            source.note(refusal);
            source.changed();
        });
    }

    /// Copies what the daemon handed out, by the clipboard's secret rules; an empty value is
    /// not copied, and the sentence saying so comes back.
    fn copy(&self, button: &gtk::Button, text: &str, what: &'static str) -> Option<String> {
        if text.is_empty() {
            glib::g_warning!("fermix", "{what} was empty, so nothing was copied");
            return Some(words::NOT_COPIED.to_owned());
        }
        self.copies.copy(&button.clipboard(), text, what);
        None
    }

    // ---- a token shown once ----

    /// `token`, shown once: unselectable and off the accessibility tree, its row read to a
    /// screen reader as what it is and never as its value, with Copy token and the push example
    /// once the door's port is known. Built at each render, so the example comes when the port
    /// does.
    fn shown_once(self: &Rc<Self>, token: HandedOut) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        let shown = gtk::Label::builder()
            .label(token.reveal())
            .selectable(false)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .xalign(0.0)
            .css_classes(["monospace"])
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        shown.update_state(&[gtk::accessible::State::Hidden(true)]);
        let row = adw::PreferencesRow::builder()
            .child(&shown)
            .activatable(false)
            .build();
        let spoken = words::token_label(&self.name);
        row.upcast_ref::<gtk::Widget>()
            .update_property(&[gtk::accessible::Property::Label(&spoken)]);
        group.add(&row);
        group.add(&self.copy_token_row(token));
        if let Some(port) = (self.dialog.deps.port)() {
            let example = fact_row("", &words::push_example(port, &self.name));
            example.add_css_class("monospace");
            group.add(&example);
        }
        group
    }

    fn copy_token_row(self: &Rc<Self>, token: HandedOut) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(words::TOKEN_ONCE)
            .activatable(false)
            .build();
        row.set_title_lines(0);
        let copy = named_button(words::COPY_TOKEN, &self.name);
        let weak = Rc::downgrade(self);
        copy.connect_clicked(move |button| {
            let Some(source) = weak.upgrade() else { return };
            let refusal = source.copy(button, token.reveal(), "a source's push credential");
            if refusal.is_some() {
                source.note(refusal);
                source.render();
            }
        });
        row.add_suffix(&copy);
        row
    }
}

fn stored(present: bool) -> &'static str {
    if present {
        words::STORED
    } else {
        words::NOT_STORED
    }
}

/// What the source is and did: its kind, the feed or stream address it reads from (one line,
/// the middle left out; a webhook's address is never published here), its daily limit, its last
/// day and the watchers listening.
fn facts(source: &SourceRow) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.add(&fact_row(words::KIND, &source.label));
    if let Some(url) = source_url(source) {
        group.add(&url_row(url));
    }
    group.add(&fact_row(
        words::ITEMS_A_DAY,
        &words::items_a_day(source.daily_budget),
    ));
    let day = words::source_last_day(source.counts.received, source.counts.routed);
    group.add(&fact_row(words::LAST_DAY, &day));
    group.add(&fact_row(words::LISTENING, &words::list(&source.listening)));
    group
}

/// The address on one line, its middle left out, selectable, and whole in its tooltip.
fn url_row(url: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(words::URL)
        .activatable(false)
        .build();
    let value = gtk::Label::builder()
        .label(url)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .single_line_mode(true)
        .selectable(true)
        .hexpand(true)
        .xalign(1.0)
        .tooltip_text(url)
        .css_classes(["dimmed"])
        .build();
    row.add_suffix(&value);
    row
}

/// What it refused and what it missed, in the daemon's sentences, only where there is any.
fn missed(detail: &SourceDetail) -> Option<adw::PreferencesGroup> {
    if detail.rejected.is_empty() && detail.gaps.is_empty() {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    let sentences = |all: Vec<String>| words::lines(&all);
    if !detail.rejected.is_empty() {
        let refused = sentences(detail.rejected.iter().map(|r| r.sentence.clone()).collect());
        group.add(&fact_row(words::REFUSED, &refused));
    }
    if !detail.gaps.is_empty() {
        let gaps = sentences(detail.gaps.iter().map(|g| g.sentence.clone()).collect());
        group.add(&fact_row(words::MISSED, &gaps));
    }
    Some(group)
}
