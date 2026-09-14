//! One pane's descriptor sections, as one form.
//!
//! A pane is one group per section the daemon publishes under it, and one row
//! per row in each. The form is built from the shape the daemon sent and then
//! updated in place: the rows are rebuilt only when the *set* of keys changes,
//! because rebuilding a control takes the focus out of it and this surface
//! refreshes underneath the person using it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::management::types::SettingsPane;
use crate::models::{spawn, Change, SettingsModel};

use super::descriptor_row::{DescriptorRow, OpenChoice, Placement};

/// One pane's form.
pub struct DescriptorForm {
    root: gtk::Widget,
    column: gtk::Box,
    settings: Rc<SettingsModel>,
    pane: SettingsPane,
    /// One group per section, in published order.
    groups: RefCell<Vec<(String, Vec<gtk::Widget>)>>,
    /// One row per key, by section.
    rows: RefCell<BTreeMap<String, Vec<Rc<DescriptorRow>>>>,
    /// The key set each section was built from.
    shapes: RefCell<BTreeMap<String, Vec<String>>>,
    /// The sections of this pane this form draws, where a hand-built pane asked
    /// for some of them rather than all. Empty means every section of the pane.
    only: Vec<String>,
    /// The heading a hand-built pane gave its one group, where it gave one.
    heading: Option<crate::copy::Key>,
    /// How this form answers a listing the daemon could not inline.
    picker: RefCell<Option<OpenChoice>>,
    /// How a hand-built pane arranges the rows it was given. The rows are the
    /// daemon's and so is their order; a pane may group them, and the one
    /// grouping the design asks for is credentials first.
    #[allow(clippy::type_complexity)]
    order: RefCell<Option<fn(&mut [crate::management::types::SettingsRow])>>,
    /// Which of a section's rows this form draws, where a hand-built pane draws
    /// one section in more than one group. Every row a section publishes is
    /// still drawn: the pane that filters is the pane that draws the rest.
    #[allow(clippy::type_complexity)]
    keep: RefCell<Option<fn(&crate::management::types::SettingsRow) -> bool>>,
    /// What sits at the trailing edge of this form's group heading, where a
    /// hand-built pane gave it something: the vendor mark of the platform a
    /// group is about.
    suffix: RefCell<Option<gtk::Widget>>,
}

impl DescriptorForm {
    /// Build the form for one pane: every section the daemon assigned to it.
    pub fn new(settings: Rc<SettingsModel>, pane: SettingsPane) -> Rc<Self> {
        Self::build(settings, pane, Vec::new(), None, true)
    }

    /// Build the form for named sections of one pane, for a hand-built pane
    /// that composes the daemon's rows into a group of its own.
    ///
    /// `heading` replaces the daemon's section title, which is the one thing a
    /// hand-built pane owns about a group it composed: the design names those
    /// headings, and the rows inside them are still the daemon's.
    pub fn restricted(
        settings: Rc<SettingsModel>,
        pane: SettingsPane,
        sections: Vec<String>,
        heading: Option<crate::copy::Key>,
    ) -> Rc<Self> {
        Self::build(settings, pane, sections, heading, false)
    }

    fn build(
        settings: Rc<SettingsModel>,
        pane: SettingsPane,
        only: Vec<String>,
        heading: Option<crate::copy::Key>,
        own_scroller: bool,
    ) -> Rc<Self> {
        let column = super::super::column();
        if own_scroller {
            column.add_css_class("fermix-gutter");
        }

        // A form inside a hand-built pane is one group among several, so the
        // pane owns the one scroll container and this form is a plain column.
        let root: gtk::Widget = if own_scroller {
            super::super::scrolled(&super::super::clamp(&column)).upcast()
        } else {
            column.clone().upcast()
        };

        let form = Rc::new(Self {
            root,
            column,
            settings,
            pane,
            groups: RefCell::new(Vec::new()),
            rows: RefCell::new(BTreeMap::new()),
            shapes: RefCell::new(BTreeMap::new()),
            only,
            heading,
            picker: RefCell::new(None),
            order: RefCell::new(None),
            keep: RefCell::new(None),
            suffix: RefCell::new(None),
        });

        form.connect();
        form.draw();
        form
    }

    /// How this form answers a listing the daemon could not inline.
    ///
    /// The rows are built again: a picker set after the form was drawn would
    /// otherwise reach only the rows a later refresh happens to rebuild.
    pub fn set_picker(self: &Rc<Self>, picker: OpenChoice) {
        self.picker.replace(Some(picker));
        self.clear();
        self.draw();
    }

    /// How this form arranges the rows one section published.
    pub fn set_order(self: &Rc<Self>, order: fn(&mut [crate::management::types::SettingsRow])) {
        self.order.replace(Some(order));
        self.clear();
        self.draw();
    }

    /// What sits at the trailing edge of this form's group heading.
    pub fn set_header_suffix(self: &Rc<Self>, suffix: impl IsA<gtk::Widget>) {
        self.suffix.replace(Some(suffix.upcast()));
        self.clear();
        self.draw();
    }

    /// Which of a section's rows this form draws.
    pub fn set_filter(self: &Rc<Self>, keep: fn(&crate::management::types::SettingsRow) -> bool) {
        self.keep.replace(Some(keep));
        self.clear();
        self.draw();
    }

    /// One section's rows, as this form arranges them.
    fn rows_of(&self, section: &str) -> Vec<crate::management::types::SettingsRow> {
        let mut rows = self.settings.state().rows(section);
        if let Some(keep) = *self.keep.borrow() {
            rows.retain(keep);
        }
        if let Some(order) = *self.order.borrow() {
            order(&mut rows);
        }
        rows
    }

    /// The sections this form draws, in published order.
    fn sections(&self) -> Vec<(String, String)> {
        let state = self.settings.state();
        state
            .sections_of(self.pane)
            .iter()
            .filter(|section| self.only.is_empty() || self.only.contains(&section.id))
            .map(|section| (section.id.clone(), section.title.clone()))
            .collect()
    }

    /// The widget to put in the pane stack.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// The pane this form draws.
    pub fn pane(&self) -> SettingsPane {
        self.pane
    }

    /// Read every section this form draws.
    pub fn load(self: &Rc<Self>) {
        let settings = Rc::clone(&self.settings);
        let pane = self.pane;
        let only = self.only.clone();
        spawn(async move {
            if only.is_empty() {
                settings.refresh_pane(pane).await;
                return;
            }
            for section in only {
                settings.refresh_section(&section).await;
            }
        });
    }

    /// The rows this form is showing, for the tests and the pane search.
    pub fn row_labels(&self) -> Vec<String> {
        let sections = self.sections();
        let state = self.settings.state();
        sections
            .iter()
            .flat_map(|(id, _)| state.rows(id))
            .map(|row| row.label)
            .collect()
    }

    fn connect(self: &Rc<Self>) {
        let form = Rc::downgrade(self);
        self.settings.observe(move |change| {
            let Some(form) = form.upgrade() else {
                return;
            };

            match change {
                Change::Sections => form.draw(),
                Change::Section(id) if form.owns(id) => form.draw(),
                Change::Row((section, key)) if form.owns(section) => form.draw_row(section, key),
                _ => {}
            }
        });
    }

    fn owns(&self, section: &str) -> bool {
        self.sections().iter().any(|(id, _)| id == section)
    }

    /// Draw every section of this pane, rebuilding only what changed shape.
    pub fn draw(self: &Rc<Self>) {
        let sections = self.sections();

        let drawn: Vec<String> = self
            .groups
            .borrow()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        let wanted: Vec<String> = sections.iter().map(|(id, _)| id.clone()).collect();

        if drawn != wanted {
            self.clear();
        }

        for (id, title) in sections {
            self.draw_section(&id, &title);
        }
    }

    fn clear(&self) {
        for (_, widgets) in self.groups.borrow_mut().drain(..) {
            for widget in widgets {
                self.column.remove(&widget);
            }
        }
        self.rows.borrow_mut().clear();
        self.shapes.borrow_mut().clear();
    }

    /// One section: its group, its rows, and the values in them.
    fn draw_section(self: &Rc<Self>, id: &str, title: &str) {
        let rows = self.rows_of(id);
        let shape: Vec<String> = rows.iter().map(|row| row.key.clone()).collect();

        let unchanged = self
            .shapes
            .borrow()
            .get(id)
            .map(|drawn| *drawn == shape)
            .unwrap_or(false);

        if unchanged {
            let held = self.rows.borrow();
            let Some(built) = held.get(id) else {
                return;
            };
            for (row, data) in built.iter().zip(rows.iter()) {
                row.update(data);
            }
            return;
        }

        self.remove_section(id);
        if rows.is_empty() {
            return;
        }

        let heading = self
            .heading
            .map(crate::copy::text)
            .unwrap_or_else(|| title.to_string());
        let group = adw::PreferencesGroup::builder().title(heading).build();
        if let Some(suffix) = self.suffix.borrow().as_ref() {
            group.set_header_suffix(Some(suffix));
        }
        let mut widgets: Vec<gtk::Widget> = vec![group.clone().upcast()];
        let mut built = Vec::with_capacity(rows.len());

        self.column.append(&group);

        for data in &rows {
            let row = DescriptorRow::build(
                Rc::clone(&self.settings),
                id,
                data,
                self.picker.borrow().clone(),
            );
            match row.placement() {
                Placement::InGroup(children) => {
                    for child in children {
                        add_child(&group, &child);
                    }
                }
                Placement::OwnGroup(own) => {
                    self.column.append(&own);
                    widgets.push(own.upcast());
                }
            }
            built.push(row);
        }

        self.groups.borrow_mut().push((id.to_string(), widgets));
        self.rows.borrow_mut().insert(id.to_string(), built);
        self.shapes.borrow_mut().insert(id.to_string(), shape);
    }

    fn remove_section(&self, id: &str) {
        let mut groups = self.groups.borrow_mut();
        let Some(at) = groups.iter().position(|(held, _)| held == id) else {
            return;
        };
        let (_, widgets) = groups.remove(at);
        for widget in widgets {
            self.column.remove(&widget);
        }

        self.rows.borrow_mut().remove(id);
        self.shapes.borrow_mut().remove(id);
    }

    /// One row's own state: its refusal, its busy mark, its optimistic value.
    fn draw_row(self: &Rc<Self>, section: &str, key: &str) {
        let rows = self.rows_of(section);
        let Some(data) = rows.iter().find(|row| row.key == key) else {
            return;
        };

        let held = self.rows.borrow();
        let Some(built) = held.get(section) else {
            return;
        };
        let Some(row) = built.iter().find(|row| row.key() == key) else {
            return;
        };

        row.update(data);
    }
}

/// Add one child to a group: a preferences row joins the list, and anything
/// else sits under it, which is what the toolkit does with each.
fn add_child(group: &adw::PreferencesGroup, child: &gtk::Widget) {
    group.add(child);
}
