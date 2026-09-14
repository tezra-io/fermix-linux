//! The searchable pane list.
//!
//! Thirteen rows in four groups, in one list and one focus order. The search
//! entry is the list's key-capture widget, so typing anywhere in the sidebar
//! filters it, and Enter opens the first row still showing. It indexes the pane
//! titles and the row labels of the descriptors that have been read, which is
//! the only index that cannot name a row the daemon does not publish.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::SettingsPane;
use crate::models::pane::{self, PaneRow};
use crate::models::{Change, SettingsModel};

/// The pane list, as the sidebar shows it.
pub struct PaneList {
    root: gtk::Widget,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    rows: Vec<(SettingsPane, gtk::ListBoxRow)>,
    empty: gtk::Label,
    settings: Rc<SettingsModel>,
    /// Set while the list is selecting a row itself, so the handler can tell a
    /// person's gesture from a redraw.
    updating: RefCell<bool>,
}

impl PaneList {
    /// Build the pane list over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("navigation-sidebar");
        list.set_header_func(header);

        let mut rows = Vec::with_capacity(pane::PANES.len());
        for row in pane::PANES {
            let widget = pane_row(row);
            list.append(&widget);
            rows.push((row.pane, widget.upcast::<gtk::ListBoxRow>()));
        }

        let search = gtk::SearchEntry::builder()
            .placeholder_text(copy::text(Key::ActionSearchAccessible))
            .build();
        search.update_property(&[gtk::accessible::Property::Label(&copy::text(
            Key::SettingsSearchAccessible,
        ))]);
        // Typing anywhere in the sidebar goes to the search entry, which is
        // what makes "find a pane" one gesture rather than two.
        search.set_key_capture_widget(Some(&list));

        let empty = super::super::caption(&copy::text(Key::SettingsSearchEmpty));
        empty.set_visible(false);
        empty.set_halign(gtk::Align::Center);

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(crate::metrics::SPACE_TIGHT)
            .build();
        search.set_margin_start(crate::metrics::SPACE_TIGHT);
        search.set_margin_end(crate::metrics::SPACE_TIGHT);
        search.set_margin_top(crate::metrics::SPACE_TIGHT);
        column.append(&search);
        column.append(&super::super::scrolled(&list));
        column.append(&empty);

        let panes = Rc::new(Self {
            root: column.upcast(),
            search: search.clone(),
            list: list.clone(),
            rows,
            empty,
            settings,
            updating: RefCell::new(false),
        });

        panes.connect();
        panes.draw();
        panes
    }

    /// The widget the sidebar shows.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// Put the focus in the search entry.
    pub fn focus_search(&self) {
        self.search.grab_focus();
    }

    /// The panes still showing, in order.
    pub fn visible_panes(&self) -> Vec<SettingsPane> {
        self.rows
            .iter()
            .filter(|(_, row)| row.is_visible())
            .map(|(pane, _)| *pane)
            .collect()
    }

    /// Filter the list. Public so a widget test can type into it without a
    /// keyboard.
    pub fn filter(&self, needle: &str) {
        let needle = needle.trim().to_lowercase();
        let mut showing = 0;

        for (pane, row) in &self.rows {
            let matched = needle.is_empty() || self.matches(*pane, &needle);
            row.set_visible(matched);
            showing += usize::from(matched);
        }

        self.empty.set_visible(showing == 0);
    }

    /// Whether one pane answers a search: its own name, the titles of the
    /// sections under it, or the labels of the rows that have been read.
    fn matches(&self, pane: SettingsPane, needle: &str) -> bool {
        let Some(row) = pane::row(pane) else {
            return false;
        };
        if copy::text(row.title).to_lowercase().contains(needle) {
            return true;
        }

        let state = self.settings.state();
        state.sections_of(pane).iter().any(|section| {
            section.title.to_lowercase().contains(needle)
                || state
                    .rows(&section.id)
                    .iter()
                    .any(|row| row.label.to_lowercase().contains(needle))
        })
    }

    /// Open the first pane still showing, which is what Enter does.
    pub fn open_first_visible(&self) {
        let Some(pane) = self.visible_panes().first().copied() else {
            return;
        };
        self.settings.select_pane(pane);
    }

    fn connect(self: &Rc<Self>) {
        {
            let panes = Rc::clone(self);
            self.search
                .connect_search_changed(move |entry| panes.filter(entry.text().as_str()));
        }

        {
            let panes = Rc::clone(self);
            self.search.connect_activate(move |entry| {
                // The filter is applied from the entry's own text first: a
                // search entry holds its `search-changed` back for a moment, so
                // a person who types and presses Enter straight away would
                // otherwise open the first pane of a list that has not been
                // filtered yet. The redlines fix this task at "type + Enter",
                // and that is one gesture however fast it is made.
                panes.filter(entry.text().as_str());
                panes.open_first_visible();
            });
        }

        {
            let panes = Rc::clone(self);
            self.list.connect_row_activated(move |_, row| {
                if *panes.updating.borrow() {
                    return;
                }
                let Some((pane, _)) = panes.rows.iter().find(|(_, candidate)| candidate == row)
                else {
                    return;
                };
                panes.settings.select_pane(*pane);
            });
        }

        let panes = Rc::downgrade(self);
        self.settings.observe(move |change| {
            let Some(panes) = panes.upgrade() else {
                return;
            };
            if matches!(change, Change::Pane | Change::Sections) {
                panes.draw();
            }
        });
    }

    /// Follow the model's selection.
    pub fn draw(&self) {
        let selected = self.settings.pane();
        let Some((_, row)) = self.rows.iter().find(|(pane, _)| *pane == selected) else {
            return;
        };

        *self.updating.borrow_mut() = true;
        self.list.select_row(Some(row));
        *self.updating.borrow_mut() = false;
    }
}

/// One pane's row, with the chevron that says it opens something.
fn pane_row(row: &PaneRow) -> adw::ActionRow {
    let widget = adw::ActionRow::builder()
        .title(copy::text(row.title))
        .activatable(true)
        .build();
    widget.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    widget
}

/// The four group headings, drawn by the list itself: one heading above the
/// first row of each group and nothing between rows.
fn header(row: &gtk::ListBoxRow, before: Option<&gtk::ListBoxRow>) {
    let index = row.index().max(0) as usize;
    let Some(current) = pane::PANES.get(index) else {
        return;
    };

    let previous = before
        .map(|row| row.index().max(0) as usize)
        .and_then(|index| pane::PANES.get(index));

    if previous.map(|row| row.group) == Some(current.group) {
        row.set_header(None::<&gtk::Widget>);
        return;
    }

    let label = gtk::Label::builder()
        .label(copy::text(current.group))
        .xalign(0.0)
        .build();
    label.add_css_class("heading");
    label.set_margin_start(crate::metrics::SPACE_HEADING);
    label.set_margin_top(crate::metrics::SPACE_HEADING);
    label.set_margin_bottom(crate::metrics::SPACE_TIGHT);

    row.set_header(Some(&label));
}
