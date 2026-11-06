//! A watcher's page (M60 cues §4.5): read-only, since watchers are made and changed in chat. It
//! shows the owner's own words (purpose, words to listen for, instructions) and Fermix's record
//! of it; nothing a watcher read, no item, reason or example text, is ever drawn. Its follow-up
//! is drawn in the daemon's words, and the one write here is Ask first again, which takes back
//! the follow-up's Always allow; granting it is a card's alone.

use crate::cues_dialog::{
    alone, first_page, hidden_line, nav_page, page_dialog, reading, show_line,
};
use crate::cues_rows::{fact_row, note_row, set_class, tone_dot, written_day};
use crate::cues_sources::refusal_of;
use crate::daemon::Daemon;
use crate::dialogs::{keep_with, leave};
use adw::prelude::*;
use fermix_client::cues::view::{
    follow_up_line, read_failure, watcher_filter, watcher_task, watcher_tone, AskFirstAgain,
};
use fermix_client::cues::wire::{AlwaysAllowRevoked, WatcherDetail};
use fermix_client::cues::words;
use fermix_client::management::CallError;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Opens `name`'s page over `parent` and reads it. `open_chat` brings Chat into view.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    daemon: Daemon,
    name: &str,
    open_chat: Rc<dyn Fn()>,
) {
    let holder = adw::Bin::new();
    holder.set_child(Some(&reading()));
    let (dialog, nav) = page_dialog(name);
    let shown = nav_page(name, &holder, None);
    first_page(&nav, &shown);
    dialog.present(Some(parent));
    let name = name.to_owned();
    glib::spawn_future_local(async move {
        let (reader, target) = (daemon.clone(), name.clone());
        let answer = reader.call(move |m| m.cue_watcher(&target)).await;
        let content = match answer {
            Ok(answer) => WatcherPage::content(&shown, daemon, &name, answer.watcher, open_chat),
            Err(e) => {
                glib::g_warning!("fermix", "cues.watchers.get {name} failed: {e:?}");
                let sentence = read_failure(&e, "name", words::WATCHER_GONE);
                alone(&note_row(&sentence, false, true))
            }
        };
        holder.set_child(Some(&content));
    });
}

/// A watcher's page once read. Its follow-up rows are drawn again from each answer to Ask first
/// again; the rest of the page stays as it was read.
struct WatcherPage {
    daemon: Daemon,
    /// The name the page was opened with, which the read answered to: what a write names. The
    /// answer's own row name is never sent.
    name: String,
    watcher: RefCell<WatcherDetail>,
    ask: RefCell<AskFirstAgain>,
    follow_up: adw::PreferencesGroup,
    /// "Follow-up", its name, and Ask first again while Always allow is on.
    named: adw::ActionRow,
    ask_button: gtk::Button,
    /// The daemon's sentence about it.
    sentence: adw::ActionRow,
    /// The daemon's refusal of the last press.
    refused: adw::ActionRow,
    /// Ask first again held the focus at the press: the answer gives it back to the button, or
    /// to the Follow-up row once the button is hidden.
    refocus: Cell<bool>,
}

impl WatcherPage {
    /// The page's content; the navigation page `shown` keeps what it draws from.
    fn content(
        shown: &adw::NavigationPage,
        daemon: Daemon,
        name: &str,
        watcher: WatcherDetail,
        open_chat: Rc<dyn Fn()>,
    ) -> adw::PreferencesPage {
        let content = adw::PreferencesPage::new();
        content.add(&status_group(&watcher));
        content.add(&words_group(&watcher));
        if let Some(instructions) = instructions_group(&watcher) {
            content.add(&instructions);
        }
        content.add(&record_group(&watcher));
        let page = Rc::new(WatcherPage::new(daemon, name, watcher));
        content.add(&page.follow_up);
        content.add(&chat_group(open_chat));
        page.wire();
        page.draw_follow_up();
        keep_with(shown, page);
        content
    }

    fn new(daemon: Daemon, name: &str, watcher: WatcherDetail) -> WatcherPage {
        assert!(!name.is_empty(), "a watcher's page is opened by its name");
        let ask_button = gtk::Button::builder()
            .label(words::ASK_FIRST_AGAIN)
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let named = fact_row(words::FOLLOW_UP, "");
        named.set_activatable(false);
        named.add_suffix(&ask_button);
        let sentence = adw::ActionRow::new();
        sentence.set_use_markup(false);
        sentence.set_activatable(false);
        sentence.set_title_lines(0);
        let (follow_up, refused) = (adw::PreferencesGroup::new(), hidden_line());
        follow_up.add(&named);
        follow_up.add(&sentence);
        follow_up.add(&refused);
        WatcherPage {
            daemon,
            name: name.to_owned(),
            watcher: RefCell::new(watcher),
            ask: RefCell::default(),
            follow_up,
            named,
            ask_button,
            sentence,
            refused,
            refocus: Cell::new(false),
        }
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.ask_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.ask_first_again();
            }
        });
    }

    /// The follow-up's rows from the watcher held now: its name, with Ask first again while
    /// Always allow is on, off while a revoke is out; the daemon's sentence, in the warning tone
    /// while it is held; and the daemon's refusal of the last press. No follow-up, no group.
    fn draw_follow_up(&self) {
        let line = follow_up_line(&self.watcher.borrow().row);
        self.follow_up.set_visible(line.is_some());
        let Some(line) = line else {
            return;
        };
        self.named
            .set_subtitle(&glib::markup_escape_text(&line.name));
        self.sentence.set_title(&line.sentence);
        set_class(&self.sentence, "warning", line.held);
        let ask = self.ask.borrow();
        self.ask_button.set_visible(line.ask_first_again);
        self.ask_button.set_sensitive(!ask.sending());
        show_line(&self.refused, ask.refusal());
    }

    /// Takes back the follow-up's Always allow: the answer's row is drawn, or the daemon's
    /// refusal said under the follow-up.
    fn ask_first_again(self: &Rc<Self>) {
        if !self.ask.borrow_mut().press() {
            return;
        }
        self.refocus.set(self.ask_button.is_focus());
        self.draw_follow_up();
        let page = self.clone();
        glib::spawn_future_local(async move {
            let name = page.name.clone();
            let answer = page
                .daemon
                .call(move |m| m.cue_watcher_always_allow_revoke(&name))
                .await;
            page.took(answer);
        });
    }

    /// The daemon's answer to Ask first again: its row drawn, or its refusal said. Where the
    /// button held the focus at the press and nothing took it since, the focus goes back to the
    /// button if it stays, else to the Follow-up row before the button is hidden, so GTK has no
    /// hidden focus to move later. An answer with no follow-up hides the whole group, so nothing
    /// in it takes the focus.
    fn took(&self, answer: Result<AlwaysAllowRevoked, CallError>) {
        let refusal = match answer {
            Ok(revoked) => {
                self.watcher.borrow_mut().row = revoked.watcher;
                None
            }
            Err(e) => Some(refusal_of(&e, "cues.watchers.always_allow.revoke").1),
        };
        self.ask.borrow_mut().answered(refusal);
        let refocus = self.refocus.take() && self.focus_unclaimed();
        let offered = follow_up_line(&self.watcher.borrow().row).map(|l| l.ask_first_again);
        if refocus && offered.is_none() {
            glib::g_debug!(
                "fermix",
                "the answer has no follow-up: its group hides and nothing in it takes the focus"
            );
        }
        if refocus && offered == Some(false) {
            self.focus_follow_up();
        }
        self.draw_follow_up();
        if refocus && offered == Some(true) && !self.ask_button.grab_focus() {
            glib::g_warning!("fermix", "Ask first again did not take the focus back");
        }
    }

    /// Whether the focus is where the press left it: on Ask first again, or on nothing.
    fn focus_unclaimed(&self) -> bool {
        let Some(root) = self.ask_button.root() else {
            return false;
        };
        let button: &gtk::Widget = self.ask_button.upcast_ref();
        root.focus().is_none_or(|focus| focus == *button)
    }

    fn focus_follow_up(&self) {
        if !self.named.grab_focus() && !self.sentence.grab_focus() {
            glib::g_warning!("fermix", "neither follow-up row took the focus");
        }
    }
}

/// The daemon's sentence about it, with its dot.
fn status_group(watcher: &WatcherDetail) -> adw::PreferencesGroup {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&watcher.row.status_sentence))
        .activatable(false)
        .build();
    row.set_title_lines(0);
    row.add_prefix(&tone_dot(watcher_tone(&watcher.row)));
    let group = adw::PreferencesGroup::new();
    group.add(&row);
    group
}

/// The owner's own words: its purpose and the words it listens for.
fn words_group(watcher: &WatcherDetail) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    if let Some(purpose) = watcher.purpose.as_deref().filter(|p| !p.is_empty()) {
        group.add(&fact_row(words::PURPOSE, purpose));
    }
    if !watcher.listen_for.is_empty() {
        group.add(&fact_row(
            words::LISTENS_FOR,
            &words::list(&watcher.listen_for),
        ));
    }
    group
}

/// Its numbered instructions, each with the day it was added; none, no group.
fn instructions_group(watcher: &WatcherDetail) -> Option<adw::PreferencesGroup> {
    if watcher.instructions.is_empty() {
        return None;
    }
    let group = adw::PreferencesGroup::builder()
        .title(words::INSTRUCTIONS)
        .build();
    for instruction in &watcher.instructions {
        let added = instruction
            .added_at
            .as_deref()
            .and_then(written_day)
            .map(|day| words::added(&day));
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&words::instruction(
                instruction.number,
                &instruction.text,
            )))
            .subtitle(glib::markup_escape_text(added.as_deref().unwrap_or("")))
            .activatable(false)
            .build();
        row.set_title_lines(0);
        group.add(&row);
    }
    Some(group)
}

/// Fermix's record of it: its filter, sources, task, limit, last day and examples, as counts.
fn record_group(watcher: &WatcherDetail) -> adw::PreferencesGroup {
    let row = &watcher.row;
    let group = adw::PreferencesGroup::new();
    if let Some(filter) = watcher_filter(row) {
        group.add(&fact_row(words::FILTER, &filter));
    }
    group.add(&fact_row(words::SOURCES, &words::list(&row.sources)));
    group.add(&fact_row(words::TASK, &watcher_task(row)));
    group.add(&fact_row(
        words::DAILY_LIMIT,
        &words::daily_limit(row.daily_cap),
    ));
    group.add(&fact_row(
        words::LAST_DAY,
        &words::watcher_last_day(&row.counts),
    ));
    let examples = words::examples(watcher.examples.pass, watcher.examples.drop);
    group.add(&fact_row(words::EXAMPLES, &examples));
    group
}

/// "Watchers are made and changed in chat.", with Open chat, which closes the page.
fn chat_group(open_chat: Rc<dyn Fn()>) -> adw::PreferencesGroup {
    let open = gtk::Button::builder()
        .label(words::OPEN_CHAT)
        .valign(gtk::Align::Center)
        .build();
    open.connect_clicked(move |button| {
        leave(button);
        open_chat();
    });
    let row = adw::ActionRow::builder()
        .title(words::WATCHERS_FOOTER)
        .activatable(false)
        .build();
    row.add_suffix(&open);
    let group = adw::PreferencesGroup::new();
    group.add(&row);
    group
}
