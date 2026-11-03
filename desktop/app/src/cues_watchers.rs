//! A watcher's page (M60 cues §4.5): read-only, since watchers are made and changed in chat. It
//! shows the owner's own words (purpose, words to listen for, instructions) and Fermix's record
//! of it; nothing a watcher read, no item, reason or example text, is ever drawn.

use crate::cues_dialog::{alone, first_page, nav_page, page_dialog, reading};
use crate::cues_rows::{fact_row, note_row, tone_dot, written_day};
use crate::daemon::Daemon;
use crate::dialogs::leave;
use adw::prelude::*;
use fermix_client::cues::view::{read_failure, watcher_filter, watcher_task, watcher_tone};
use fermix_client::cues::wire::WatcherDetail;
use fermix_client::cues::words;
use gtk::glib;
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
    first_page(&nav, &nav_page(name, &holder, None));
    dialog.present(Some(parent));
    let name = name.to_owned();
    glib::spawn_future_local(async move {
        let target = name.clone();
        let answer = daemon.call(move |m| m.cue_watcher(&target)).await;
        let content = match answer {
            Ok(answer) => page(&answer.watcher, open_chat),
            Err(e) => {
                glib::g_warning!("fermix", "cues.watchers.get {name} failed: {e:?}");
                let sentence = read_failure(&e, "name", words::WATCHER_GONE);
                alone(&note_row(&sentence, false, true))
            }
        };
        holder.set_child(Some(&content));
    });
}

fn page(watcher: &WatcherDetail, open_chat: Rc<dyn Fn()>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.add(&status_group(watcher));
    page.add(&words_group(watcher));
    if let Some(instructions) = instructions_group(watcher) {
        page.add(&instructions);
    }
    page.add(&record_group(watcher));
    page.add(&chat_group(open_chat));
    page
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
