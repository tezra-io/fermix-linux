//! Logs.
//!
//! The full detail width, edge to edge: local time to the millisecond, the
//! level as a word, the message. Search, the level filter, Pause and Export sit
//! in the header bar, and one caption line names the file this reads and the
//! command that shows what the service writes before that file exists.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{LogEntry, LogLevel};
use crate::models::logs::{level_word, LogsModel, LEVELS};
use crate::models::{spawn, Change, SettingsModel};

use super::{caption, PageToolbar};

/// The Logs surface.
pub struct LogsPage {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    logs: Rc<LogsModel>,
    store: gio::ListStore,
    stack: gtk::Stack,
    scroller: gtk::ScrolledWindow,
    empty: adw::StatusPage,
    banner: adw::Banner,
    file_caption: gtk::Label,
    search: gtk::SearchEntry,
    level: gtk::DropDown,
    pause: gtk::ToggleButton,
    export: gtk::Button,
    /// Set while the page writes a control's value into it, so a handler can
    /// tell a person's gesture from a refresh.
    updating: RefCell<bool>,
}

impl LogsPage {
    /// Build Logs over the one settings model.
    pub fn new(settings: Rc<SettingsModel>, logs: Rc<LogsModel>) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list_view(&store))
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .title(copy::text(Key::LogsEmpty))
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("entries"));
        stack.add_named(&empty, Some("empty"));
        stack.set_vexpand(true);

        let banner = adw::Banner::new("");
        banner.set_revealed(false);
        let file_caption = file_caption();

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        column.append(&banner);
        column.append(&stack);
        column.append(&file_caption);

        let page = Rc::new(Self {
            root: column.upcast(),
            settings,
            logs,
            store,
            stack,
            scroller,
            empty,
            banner,
            file_caption,
            search: search_entry(),
            level: level_filter(),
            pause: toolbar_toggle(Key::LogsPause),
            export: toolbar_button(Key::ActionExport),
            updating: RefCell::new(false),
        });

        page.connect();
        page.draw();
        page
    }

    /// The widget to put in the window.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// What this page puts in the header bar: search and the level filter
    /// leading, Pause and Export trailing.
    pub fn toolbar(&self) -> PageToolbar {
        PageToolbar {
            start: vec![self.search.clone().upcast(), self.level.clone().upcast()],
            end: vec![self.pause.clone().upcast(), self.export.clone().upcast()],
        }
    }

    /// Put the focus in the search entry, which is what the window's Search
    /// action does on this surface.
    pub fn focus_search(&self) {
        self.search.grab_focus();
    }

    fn connect(self: &Rc<Self>) {
        self.connect_model();
        self.connect_controls();
        self.connect_paging();
    }

    /// The two things that redraw this surface: its own entries, and the
    /// binding the command line reports, which is where the caption's path
    /// comes from.
    fn connect_model(self: &Rc<Self>) {
        {
            let page = Rc::downgrade(self);
            self.logs.observe(move || {
                if let Some(page) = page.upgrade() {
                    page.draw();
                }
            });
        }

        let page = Rc::downgrade(self);
        self.settings.observe(move |change| {
            if !matches!(change, Change::Service) {
                return;
            }
            if let Some(page) = page.upgrade() {
                page.draw_caption();
            }
        });
    }

    /// Search, the level filter, Pause and Export.
    fn connect_controls(self: &Rc<Self>) {
        {
            let page = Rc::clone(self);
            self.export.connect_clicked(move |_| page.export());
        }

        {
            let page = Rc::clone(self);
            self.search.connect_search_changed(move |entry| {
                if *page.updating.borrow() {
                    return;
                }
                page.logs.set_search(Some(entry.text().to_string()));
            });
        }

        {
            let page = Rc::clone(self);
            self.level.connect_selected_notify(move |dropdown| {
                if *page.updating.borrow() {
                    return;
                }
                page.logs.set_level(selected_level(dropdown));
            });
        }

        let page = Rc::clone(self);
        self.pause.connect_toggled(move |button| {
            if *page.updating.borrow() {
                return;
            }
            page.logs.set_paused(button.is_active());
        });
    }

    /// Reading when shown, polling while in front of someone, and the page
    /// behind this one when the list reaches its top.
    fn connect_paging(self: &Rc<Self>) {
        {
            let page = Rc::clone(self);
            self.scroller.connect_edge_reached(move |_, position| {
                if position != gtk::PositionType::Top || !page.logs.has_older() {
                    return;
                }
                let logs = Rc::clone(&page.logs);
                spawn(async move {
                    logs.load_older().await;
                });
            });
        }

        {
            // Shown is when this surface reads; in front of someone is when it
            // keeps reading.
            let page = Rc::clone(self);
            super::on_shown(&self.root, move || {
                let logs = Rc::clone(&page.logs);
                spawn(async move {
                    logs.refresh().await;
                });
            });
        }

        let page = Rc::clone(self);
        super::watch_visibility(&self.root, move |visible| {
            if visible {
                page.logs.start_polling();
            } else {
                page.logs.stop_polling();
            }
        });
    }

    /// Write the entries the model holds into the list, without moving anyone's
    /// place in it.
    pub fn draw(&self) {
        let entries = self.logs.entries();
        let held = self.store.n_items() as usize;

        // Appending leaves the scroll position where it is. A window that has
        // moved past everything held is the only case that replaces the list.
        let appended = entries.len() >= held
            && (0..held).all(|index| self.at(index) == entries.get(index).cloned());

        if appended {
            for entry in entries.iter().skip(held) {
                self.store.append(&glib::BoxedAnyObject::new(entry.clone()));
            }
        } else {
            let objects: Vec<glib::BoxedAnyObject> = entries
                .iter()
                .map(|entry| glib::BoxedAnyObject::new(entry.clone()))
                .collect();
            self.store.splice(0, self.store.n_items(), &objects);
        }

        let empty = entries.is_empty();
        self.stack
            .set_visible_child_name(if empty { "empty" } else { "entries" });
        self.empty
            .set_title(&copy::text(if self.logs.is_unavailable() {
                Key::LogsDaemonUnavailable
            } else {
                Key::LogsEmpty
            }));

        match self.logs.notice() {
            Some(notice) => {
                self.banner.set_title(&notice);
                self.banner.set_revealed(true);
            }
            None => self.banner.set_revealed(false),
        }

        *self.updating.borrow_mut() = true;
        self.pause.set_active(self.logs.is_paused());
        *self.updating.borrow_mut() = false;

        self.draw_caption();
    }

    fn at(&self, index: usize) -> Option<LogEntry> {
        self.store
            .item(index as u32)
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|object| object.borrow::<LogEntry>().clone())
    }

    /// The one always-visible line: the file this surface reads, and the
    /// command that shows what the service writes before that file exists.
    fn draw_caption(&self) {
        let journal = copy::text(Key::LogsCaptionJournal);
        let command = copy::text(Key::LogsJournalCommand);

        let file = self
            .log_file()
            .map(|path| copy::fill(Key::LogsCaptionFile, &[("{path}", &path.to_string_lossy())]));

        self.file_caption.set_label(&match file {
            Some(file) => format!("{file}\n{journal} {command}"),
            None => format!("{journal} {command}"),
        });
    }

    fn log_file(&self) -> Option<std::path::PathBuf> {
        let state = self.settings.state();
        let home = state.service.as_ref()?.bound_home()?;
        Some(std::path::PathBuf::from(home).join("logs/fermix.log"))
    }

    /// Write what is held to a file a person chooses.
    fn export(self: &Rc<Self>) {
        let entries = self.logs.entries();
        let body: String = entries
            .iter()
            .map(|entry| {
                format!(
                    "{} {} {}\n",
                    local_time(&entry.time),
                    copy::text(level_word(entry.level)),
                    entry.message
                )
            })
            .collect();

        let dialog = gtk::FileDialog::builder()
            .title(copy::text(Key::ActionExport))
            .initial_name(EXPORT_NAME)
            .build();
        let window = super::window_of(&self.root);

        spawn(async move {
            let Ok(file) = dialog.save_future(window.as_ref()).await else {
                return;
            };
            if let Err(error) = file
                .replace_contents_future(
                    body.into_bytes(),
                    None,
                    false,
                    gio::FileCreateFlags::REPLACE_DESTINATION,
                )
                .await
            {
                glib::g_warning!("fermix-desktop", "the log was not written: {error:?}");
            }
        });
    }
}

/// The always-visible line under the list.
///
/// Selectable, because one of the two things it names is a command to run in a
/// terminal and the design asks for it to be copyable. The Recovery surface
/// makes the same command selectable in the dialog that shows it.
fn file_caption() -> gtk::Label {
    let caption = caption("");
    caption.set_selectable(true);
    caption.set_margin_start(crate::metrics::SPACE_HEADING);
    caption.set_margin_end(crate::metrics::SPACE_HEADING);
    caption.set_margin_bottom(crate::metrics::SPACE_TIGHT);
    caption
}

fn search_entry() -> gtk::SearchEntry {
    // Eighteen characters is what it asks for and eight is the least it will
    // take: the header bar spans the whole window, so a control in it that
    // cannot shrink is a floor under the window's own minimum width. At the
    // default size there is room for the whole eighteen, which is what the
    // natural width asks for.
    let search = gtk::SearchEntry::builder()
        .placeholder_text(copy::text(Key::ActionSearchAccessible))
        .width_chars(SEARCH_FLOOR_CHARS)
        .max_width_chars(SEARCH_CHARS)
        .build();
    search.update_property(&[gtk::accessible::Property::Label(&copy::text(
        Key::LogsSearchAccessible,
    ))]);
    search
}

/// The name the export is offered under.
const EXPORT_NAME: &str = "fermix-log.txt";

/// The list: one monospace row per entry, three columns.
fn list_view(store: &gio::ListStore) -> gtk::ListView {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&entry_row()));
        }
    });
    factory.connect_bind(|_, item| bind_entry(item));

    gtk::ListView::builder()
        .model(&gtk::NoSelection::new(Some(store.clone())))
        .factory(&factory)
        .single_click_activate(false)
        .build()
}

/// One row of the list: the time, the level and the message.
fn entry_row() -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(crate::metrics::SPACE_HEADING)
        .build();
    row.set_margin_start(crate::metrics::SPACE_HEADING);
    row.set_margin_end(crate::metrics::SPACE_HEADING);
    row.set_margin_top(crate::metrics::SPACE_TIGHT);
    row.set_margin_bottom(crate::metrics::SPACE_TIGHT);

    let time = gtk::Label::builder().xalign(0.0).build();
    time.add_css_class("monospace");
    time.add_css_class("dim-label");

    // One width for every level word, so the messages line up whichever level
    // each entry carries.
    let level = gtk::Label::builder().xalign(0.0).width_chars(9).build();
    level.add_css_class("monospace");
    level.add_css_class("dim-label");

    let message = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .selectable(true)
        .wrap(true)
        .build();
    message.add_css_class("monospace");

    row.append(&time);
    row.append(&level);
    row.append(&message);
    row
}

fn bind_entry(item: &glib::Object) {
    let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
        return;
    };
    let Some(object) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
        return;
    };
    let Some(row) = item.child().and_downcast::<gtk::Box>() else {
        return;
    };

    let entry = object.borrow::<LogEntry>();
    let labels: Vec<gtk::Label> = children(&row);
    if labels.len() != 3 {
        return;
    }

    labels[0].set_label(&local_time(&entry.time));
    labels[1].set_label(&copy::text(level_word(entry.level)));
    labels[2].set_label(&entry.message);
}

fn children(row: &gtk::Box) -> Vec<gtk::Label> {
    let mut found = Vec::new();
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            found.push(label.clone());
        }
        child = widget.next_sibling();
    }
    found
}

/// One instant, in this computer's own time, to the millisecond.
///
/// The daemon writes an offset timestamp; a person reading a log is reading
/// their own clock. A time this build cannot parse is shown exactly as the
/// daemon wrote it rather than replaced with a guess.
pub fn local_time(stamp: &str) -> String {
    let Ok(parsed) = glib::DateTime::from_iso8601(stamp, None) else {
        return stamp.to_string();
    };
    let Ok(local) = parsed.to_local() else {
        return stamp.to_string();
    };

    local
        .format("%H:%M:%S")
        .map(|formatted| format!("{formatted}.{:03}", local.microsecond() / 1000))
        .unwrap_or_else(|_| stamp.to_string())
}

/// The level filter: every level the protocol publishes, and one entry for no
/// filter at all.
fn level_filter() -> gtk::DropDown {
    let mut words: Vec<String> = vec![copy::text(Key::LogsLevelAll)];
    words.extend(LEVELS.iter().map(|level| copy::text(level_word(*level))));

    let borrowed: Vec<&str> = words.iter().map(String::as_str).collect();
    let dropdown = gtk::DropDown::from_strings(&borrowed);
    // The default factory's label asks for the widest word on the list and
    // will not give it back, which in the header bar is width the window can
    // never get under. This is the same label, shortened at the end when there
    // is not room for the whole word.
    dropdown.set_factory(Some(&ellipsizing_factory()));
    dropdown.update_property(&[gtk::accessible::Property::Label(&copy::text(
        Key::LogsLevel,
    ))]);
    dropdown
}

/// The drop-down's own list item: the string list's word, shortened at its end
/// rather than holding the bar open.
fn ellipsizing_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        item.set_child(Some(&label));
    });

    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(label) = item.child().and_downcast::<gtk::Label>() else {
            return;
        };
        let word = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .map(|object| object.string().to_string())
            .unwrap_or_default();
        label.set_label(&word);
    });

    factory
}

/// One toolbar button, at a label that shortens rather than holding the bar
/// open. The whole word stays the accessible name and the tooltip.
fn toolbar_button(key: Key) -> gtk::Button {
    let word = copy::text(key);
    let button = gtk::Button::builder().label(&word).build();
    dress_toolbar_button(button.upcast_ref::<gtk::Button>(), &word);
    button
}

/// The same, for the one toolbar control that is a toggle.
fn toolbar_toggle(key: Key) -> gtk::ToggleButton {
    let word = copy::text(key);
    let button = gtk::ToggleButton::builder().label(&word).build();
    dress_toolbar_button(button.upcast_ref::<gtk::Button>(), &word);
    button
}

fn dress_toolbar_button(button: &gtk::Button, word: &str) {
    super::shorten(button);
    button.set_tooltip_text(Some(word));
    button.update_property(&[gtk::accessible::Property::Label(word)]);
}

/// How many characters of a query the search asks room for, and the least it
/// will take before it starts shortening.
const SEARCH_CHARS: i32 = 18;
const SEARCH_FLOOR_CHARS: i32 = 4;

fn selected_level(dropdown: &gtk::DropDown) -> Option<LogLevel> {
    let index = dropdown.selected() as usize;
    if index == 0 {
        return None;
    }
    LEVELS.get(index - 1).copied()
}
