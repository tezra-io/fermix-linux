//! A reply's blocks as widgets. A block still streaming is updated in place:
//! a paragraph's label gets the longer text, and a list or table keeps the rows
//! that did not change and redraws only from the first one that did. A code
//! block is coloured in the theme's own hues, again whenever it turns light or
//! dark.

use super::entries::{copy_text, message_menu};
use super::table_scroller::TableScroller;
use adw::prelude::*;
use fermix_client::markdown::highlight::{self, Inks};
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
    Code(CodeText),
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
            Block::Code { language, text } => code_block(language.as_deref(), text),
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
            (
                Part::Code(code),
                Block::Code { language: was, .. },
                Block::Code { language, text },
            ) if was == language => {
                code.text.replace(text.clone());
                let dark = adw::StyleManager::default().is_dark();
                paint_code(&code.label, text, language.as_deref(), dark);
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

/// A code block's code, and the text it draws, which Copy copies as written.
struct CodeText {
    label: gtk::Label,
    text: Rc<RefCell<String>>,
}

/// Monospace and wrapped at the reply's width, coloured, its language named
/// above it where its fence named one, with its own copy button.
fn code_block(language: Option<&str>, text: &str) -> Shown {
    let label = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .hexpand(true)
        .selectable(true)
        .extra_menu(&message_menu())
        .css_classes(["monospace"])
        .build();
    let text = Rc::new(RefCell::new(text.to_owned()));
    let dark = adw::StyleManager::default().is_dark();
    paint_code(&label, &text.borrow(), language, dark);
    follow_theme(&label, &text, language);
    let name = language.map(highlight::language_name);
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    let spoken = name
        .as_ref()
        .map_or("Code".to_owned(), |name| format!("Code, {name}"));
    column.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    if let Some(name) = name {
        column.append(&code_caption(&name));
    }
    column.append(&label);
    let source = text.clone();
    let panel = panel(&column, "Copy code", move || source.borrow().clone());
    Shown {
        widget: panel.upcast(),
        part: Part::Code(CodeText { label, text }),
    }
}

/// The language's name over its code, small and quiet; the block's own name
/// says it to a screen reader instead.
fn code_caption(name: &str) -> gtk::Label {
    let caption = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dimmed"])
        .build();
    caption.set_accessible_role(gtk::AccessibleRole::Presentation);
    caption
}

fn paint_code(label: &gtk::Label, text: &str, language: Option<&str>, dark: bool) {
    label.set_markup(&highlight::markup(text, language, &code_inks(dark)));
}

/// Colours `label` again whenever the theme turns light or dark, for as long
/// as it lives.
fn follow_theme(label: &gtk::Label, text: &Rc<RefCell<String>>, language: Option<&str>) {
    let style = adw::StyleManager::default();
    let weak = label.downgrade();
    let text = text.clone();
    let language = language.map(str::to_owned);
    let handler = style.connect_dark_notify(move |style| {
        if let Some(label) = weak.upgrade() {
            paint_code(&label, &text.borrow(), language.as_deref(), style.is_dark());
        }
    });
    // The style manager outlives every reply, so the handler goes with the label.
    let handler = std::cell::Cell::new(Some(handler));
    label.connect_destroy(move |_| {
        if let Some(handler) = handler.take() {
            adw::StyleManager::default().disconnect(handler);
        }
    });
}

/// The theme's own purple, green and yellow, as libadwaita draws each as text
/// on a light or a dark page: muted enough to read as code without competing
/// with the reply, and never the blue a link or the accent is. They are the
/// hues the macOS app's code takes.
fn code_inks(dark: bool) -> Inks {
    let hex = |accent: adw::AccentColor| {
        let ink = accent.to_standalone_rgba(dark);
        let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            byte(ink.red()),
            byte(ink.green()),
            byte(ink.blue())
        )
    };
    Inks {
        keyword: hex(adw::AccentColor::Purple),
        string: hex(adw::AccentColor::Green),
        number: hex(adw::AccentColor::Yellow),
    }
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
