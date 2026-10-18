//! A reply's blocks as widgets. A block still streaming is updated in place:
//! a paragraph's label gets the longer text, and a list or table keeps the rows
//! that did not change and redraws only from the first one that did.

use super::entries::{copy_text, message_menu};
use super::table_scroller::TableScroller;
use adw::prelude::*;
use fermix_client::markdown::{Block, Cell, Item, Table};
use std::cell::RefCell;
use std::rc::Rc;

/// How far each level of a nested list sits in from the one above it.
const LIST_INDENT: i32 = 20;
/// A table cell wraps at this width before the table scrolls sideways.
const CELL_MIN_CHARS: i32 = 8;
const CELL_MAX_CHARS: i32 = 40;

/// One block's widget, and what it needs to grow in place.
pub struct Shown {
    pub widget: gtk::Widget,
    part: Part,
}

enum Part {
    Fixed,
    Label(gtk::Label),
    List(ListRows),
    Table(TableGrid),
}

impl Shown {
    pub fn new(block: &Block) -> Shown {
        match block {
            Block::Text(markup) => label_part(text_label(markup)),
            Block::Heading(level, markup) => {
                let size = match level {
                    1 => "x-large",
                    2 => "large",
                    _ => "medium",
                };
                let label = text_label(&format!(
                    "<span size=\"{size}\" weight=\"bold\">{markup}</span>"
                ));
                fixed(label)
            }
            Block::Quote(markup) => {
                let label = text_label(markup);
                label.add_css_class("chat-quote");
                label.add_css_class("dimmed");
                label_part(label)
            }
            Block::Code(text) => code_block(text),
            Block::List(items) => {
                let rows = ListRows::new(items);
                Shown {
                    widget: rows.body.clone().upcast(),
                    part: Part::List(rows),
                }
            }
            Block::Table(table) => {
                let grid = TableGrid::new(table);
                Shown {
                    widget: grid.panel.clone().upcast(),
                    part: Part::Table(grid),
                }
            }
            Block::Rule => fixed(gtk::Separator::new(gtk::Orientation::Horizontal)),
        }
    }

    /// Shows `new` in place of `old`, the same block with more streamed in;
    /// false when it is a different kind of block and needs a new widget.
    pub fn update(&mut self, old: &Block, new: &Block) -> bool {
        match (&mut self.part, old, new) {
            (Part::Label(label), Block::Text(_), Block::Text(markup))
            | (Part::Label(label), Block::Quote(_), Block::Quote(markup)) => {
                label.set_markup(markup);
                true
            }
            (Part::Label(label), Block::Code(_), Block::Code(text)) => {
                label.set_text(text);
                true
            }
            (Part::List(rows), Block::List(old), Block::List(new)) => {
                rows.update(old, new);
                true
            }
            (Part::Table(grid), Block::Table(old), Block::Table(new)) => grid.update(old, new),
            _ => false,
        }
    }
}

fn label_part(label: gtk::Label) -> Shown {
    Shown {
        widget: label.clone().upcast(),
        part: Part::Label(label),
    }
}

fn fixed(widget: impl IsA<gtk::Widget>) -> Shown {
    Shown {
        widget: widget.upcast(),
        part: Part::Fixed,
    }
}

/// A reply's text, no wider than reads comfortably.
fn text_label(markup: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .max_width_chars(90)
        .extra_menu(&message_menu())
        .build();
    label.set_markup(markup);
    label
}

/// Monospace, scrolled sideways when wide, with its own copy button.
fn code_block(text: &str) -> Shown {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .selectable(true)
        .extra_menu(&message_menu())
        .css_classes(["monospace"])
        .build();
    let scroller = sideways_scroller(&label);
    let source = label.clone();
    let panel = panel(&scroller, "Copy code", move || source.text().to_string());
    Shown {
        widget: panel.upcast(),
        part: Part::Label(label),
    }
}

/// Content that never wraps, scrolled sideways when wider than the reply.
fn sideways_scroller(content: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .propagate_natural_width(true)
        .hexpand(true)
        .child(content)
        .build()
}

/// The soft panel code and tables share: the content, and a copy button
/// copying what `copied` returns.
fn panel(
    content: &impl IsA<gtk::Widget>,
    what: &str,
    copied: impl Fn() -> String + 'static,
) -> gtk::Box {
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(what)
        .valign(gtk::Align::Start)
        .css_classes(["flat", "circular"])
        .build();
    copy.update_property(&[gtk::accessible::Property::Label(what)]);
    copy.connect_clicked(move |button| copy_text(button, &copied()));
    let block = gtk::Box::builder()
        .spacing(6)
        .css_classes(["chat-code"])
        .build();
    block.append(content);
    block.append(&copy);
    block
}

/// A list, one row per item: the marker, faint, beside text that wraps under
/// itself rather than under the marker.
struct ListRows {
    body: gtk::Box,
    rows: Vec<(gtk::Box, gtk::Label)>,
}

impl ListRows {
    fn new(items: &[Item]) -> ListRows {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .accessible_role(gtk::AccessibleRole::List)
            .build();
        let mut rows = ListRows {
            body,
            rows: Vec::new(),
        };
        rows.append(items);
        rows
    }

    fn update(&mut self, old: &[Item], new: &[Item]) {
        let same = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let mut next = same;
        if let (Some(was), Some(item), Some((_, text))) =
            (old.get(same), new.get(same), self.rows.get(same))
        {
            if was.depth == item.depth && was.marker == item.marker {
                text.set_markup(&item.markup);
                next += 1;
            }
        }
        for (row, _) in self.rows.drain(next.min(self.rows.len())..) {
            self.body.remove(&row);
        }
        self.append(&new[next.min(new.len())..]);
    }

    fn append(&mut self, items: &[Item]) {
        for item in items {
            let row = gtk::Box::builder()
                .spacing(6)
                .margin_start(LIST_INDENT * i32::from(item.depth))
                .accessible_role(gtk::AccessibleRole::ListItem)
                .build();
            let marker = gtk::Label::builder()
                .label(&item.marker)
                .width_chars(3)
                .xalign(1.0)
                .yalign(0.0)
                .css_classes(["dimmed"])
                .build();
            marker.set_accessible_role(gtk::AccessibleRole::Presentation);
            let text = text_label(&item.markup);
            text.set_hexpand(true);
            row.append(&marker);
            row.append(&text);
            self.body.append(&row);
            self.rows.push((row, text));
        }
    }
}

/// A table on the code panel: the header row in heading weight, cells that
/// wrap at their natural width when the reply is narrow, and sideways
/// scrolling only past each column's floor.
struct TableGrid {
    panel: gtk::Box,
    grid: gtk::Grid,
    /// What the copy button copies, kept current while the table streams.
    table: Rc<RefCell<Table>>,
}

impl TableGrid {
    fn new(table: &Table) -> TableGrid {
        let grid = gtk::Grid::builder()
            .column_spacing(18)
            .row_spacing(6)
            .accessible_role(gtk::AccessibleRole::Table)
            .build();
        let shared = Rc::new(RefCell::new(table.clone()));
        let source = shared.clone();
        let panel = panel(&TableScroller::new(&grid), "Copy table", move || {
            source.borrow().tab_text()
        });
        let drawn = TableGrid {
            panel,
            grid,
            table: shared,
        };
        drawn.attach_row(0, &table.head, true);
        for (index, row) in table.rows.iter().enumerate() {
            drawn.attach_row(index + 1, row, false);
        }
        drawn
    }

    /// Keeps the rows that did not change and redraws from the first that did;
    /// false when the header changed, which needs a new table.
    fn update(&mut self, old: &Table, new: &Table) -> bool {
        if old.head != new.head {
            return false;
        }
        let same = old
            .rows
            .iter()
            .zip(&new.rows)
            .take_while(|(a, b)| a == b)
            .count();
        for _ in same..old.rows.len() {
            self.grid.remove_row(grid_row(same + 1));
        }
        for (index, row) in new.rows.iter().enumerate().skip(same) {
            self.attach_row(index + 1, row, false);
        }
        self.table.replace(new.clone());
        true
    }

    fn attach_row(&self, row: usize, cells: &[Cell], head: bool) {
        for (column, cell) in cells.iter().enumerate() {
            let label = gtk::Label::builder()
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .width_chars(CELL_MIN_CHARS)
                .max_width_chars(CELL_MAX_CHARS)
                .xalign(0.0)
                .yalign(0.0)
                .accessible_role(if head {
                    gtk::AccessibleRole::ColumnHeader
                } else {
                    gtk::AccessibleRole::Cell
                })
                .build();
            label.set_markup(&cell.markup);
            if head {
                label.add_css_class("heading");
            }
            self.grid
                .attach(&label, grid_row(column), grid_row(row), 1, 1);
        }
    }
}

/// A row or column index as GTK takes it; a reply never holds enough rows to
/// reach the limit, and one past it lands on the last place rather than wrapping.
fn grid_row(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}
