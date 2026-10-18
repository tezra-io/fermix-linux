//! Replies arrive as Markdown and are drawn as blocks of Pango markup. Every byte
//! of model text is escaped before it becomes markup, and only web and mail links
//! stay clickable, so a reply can never inject markup or open a local file. A
//! bare web address in the text is a link too, as in any messenger.

mod draft;

pub use draft::Draft;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd, TextMergeStream};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Pango markup: a paragraph.
    Text(String),
    /// A list, one row per item, nested items included.
    List(Vec<Item>),
    Table(Table),
    /// Heading level (1 to 6) and its markup.
    Heading(u8, String),
    /// Markup, drawn indented and dim.
    Quote(String),
    /// Raw text for a monospace view: never markup.
    Code(String),
    Rule,
}

/// One list item: drawn with its marker beside it, so wrapped lines hang under
/// the text rather than the marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// 0 for the outer list.
    pub depth: u8,
    /// "•", "3." or a task box; empty for text that follows a nested list.
    pub marker: String,
    pub markup: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub head: Vec<Cell>,
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub markup: String,
    /// The cell's words without markup, for copying.
    pub text: String,
}

impl Table {
    /// One line per row, cells separated by tabs, as a spreadsheet pastes it.
    pub fn tab_text(&self) -> String {
        std::iter::once(&self.head)
            .chain(&self.rows)
            .map(|row| {
                let cells: Vec<&str> = row.iter().map(|cell| cell.text.as_str()).collect();
                cells.join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn render(markdown: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut renderer = Renderer::default();
    // Merged, so an address the parser split into pieces is linked whole.
    for event in TextMergeStream::new(Parser::new_ext(markdown, options)) {
        renderer.event(event);
    }
    renderer.flush_text();
    renderer.blocks
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attribute(text: &str) -> String {
    escape(text).replace('"', "&quot;").replace('\'', "&apos;")
}

fn clickable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

#[derive(Default)]
struct Renderer {
    blocks: Vec<Block>,
    /// Markup of the block being built.
    text: String,
    /// Raw text of the code block being built.
    raw: Option<String>,
    table: Option<TableDraft>,
    /// One entry per open list: the next number, or None for bullets.
    lists: Vec<Option<u64>>,
    /// The items of an outer list drawn as rows; None outside one, and for a
    /// list inside a quote, which stays inline in the quote's markup.
    items: Option<Vec<Item>>,
    /// The depth and marker of the item whose text is being built.
    item: Option<(u8, String)>,
    quote_depth: usize,
    /// One entry per open link: whether it was opened as markup.
    links: Vec<bool>,
}

#[derive(Default)]
struct TableDraft {
    head: Vec<Cell>,
    rows: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    /// The open cell's words without markup.
    text: String,
}

impl Renderer {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => self.push_code_span(&code),
            Event::SoftBreak | Event::HardBreak => self.push_break(),
            Event::Rule => {
                self.flush_text();
                self.blocks.push(Block::Rule);
            }
            Event::Html(html) | Event::InlineHtml(html) => self.push_text(&html),
            Event::TaskListMarker(done) => self.task_marker(done),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::CodeBlock(_) => {
                self.flush_text();
                self.raw = Some(String::new());
            }
            Tag::Table(_) => {
                self.flush_text();
                self.table = Some(TableDraft::default());
            }
            Tag::Paragraph if self.in_container() && !self.text.is_empty() => {
                self.text.push('\n');
            }
            Tag::Heading { .. } => self.flush_text(),
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::List(first) => self.start_list(first),
            Tag::Item => self.start_item(),
            Tag::Emphasis => self.text.push_str("<i>"),
            Tag::Strong => self.text.push_str("<b>"),
            Tag::Strikethrough => self.text.push_str("<s>"),
            Tag::Link { dest_url, .. } => self.start_link(&dest_url),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph if !self.in_container() => self.flush_text(),
            TagEnd::Heading(level) => {
                let markup = std::mem::take(&mut self.text);
                self.blocks
                    .push(Block::Heading(heading_number(level), markup));
            }
            TagEnd::BlockQuote(_) => self.end_quote(),
            TagEnd::List(_) => self.end_list(),
            TagEnd::Emphasis => self.text.push_str("</i>"),
            TagEnd::Strong => self.text.push_str("</b>"),
            TagEnd::Strikethrough => self.text.push_str("</s>"),
            TagEnd::Link => self.end_link(),
            TagEnd::CodeBlock => self.end_code(),
            TagEnd::TableCell => self.end_cell(),
            TagEnd::TableHead => self.end_head(),
            TagEnd::TableRow => self.end_row(),
            TagEnd::Table => self.end_table(),
            _ => {}
        }
    }

    fn in_container(&self) -> bool {
        !self.lists.is_empty() || self.quote_depth > 0
    }

    fn push_text(&mut self, text: &str) {
        if let Some(table) = self.table.as_mut() {
            table.text.push_str(text);
        }
        match self.raw.as_mut() {
            Some(raw) => raw.push_str(text),
            None if !self.links.is_empty() => self.text.push_str(&escape(text)),
            None => self.text.push_str(&linkify(text)),
        }
    }

    fn push_code_span(&mut self, code: &str) {
        if let Some(table) = self.table.as_mut() {
            table.text.push_str(code);
        }
        match self.raw.as_mut() {
            Some(raw) => raw.push_str(code),
            None => self.text.push_str(&format!("<tt>{}</tt>", escape(code))),
        }
    }

    fn push_break(&mut self) {
        match self.raw.as_mut() {
            Some(raw) => raw.push(' '),
            None => self.text.push('\n'),
        }
    }

    fn start_list(&mut self, first: Option<u64>) {
        if self.lists.is_empty() && self.quote_depth == 0 {
            self.items = Some(Vec::new());
        }
        self.lists.push(first);
    }

    fn start_item(&mut self) {
        let depth = self.lists.len().saturating_sub(1);
        let marker = match self.lists.last_mut() {
            Some(Some(number)) => {
                let marker = format!("{number}.");
                *number += 1;
                marker
            }
            _ => "•".to_owned(),
        };
        if self.items.is_some() {
            self.end_item();
            self.item = Some((u8::try_from(depth).unwrap_or(u8::MAX), marker));
            return;
        }
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(&"    ".repeat(depth));
        self.text.push_str(&marker);
        self.text.push_str("  ");
    }

    /// Moves the item being built, if it has a marker or words, into the list.
    fn end_item(&mut self) {
        let markup = std::mem::take(&mut self.text);
        let (depth, marker) = match self.item.take() {
            Some(item) => item,
            None if markup.trim().is_empty() => return,
            None => {
                let depth = self.lists.len().saturating_sub(1);
                (u8::try_from(depth).unwrap_or(u8::MAX), String::new())
            }
        };
        if let Some(items) = self.items.as_mut() {
            items.push(Item {
                depth,
                marker,
                markup: markup.trim_end().to_owned(),
            });
        }
    }

    fn task_marker(&mut self, done: bool) {
        let mark = if done { "☑" } else { "☐" };
        match self.item.as_mut() {
            Some((_, marker)) => *marker = mark.to_owned(),
            None => {
                self.text.push_str(mark);
                self.text.push(' ');
            }
        }
    }

    fn start_link(&mut self, url: &str) {
        let open = clickable(url);
        if open {
            self.text
                .push_str(&format!("<a href=\"{}\">", escape_attribute(url)));
        }
        self.links.push(open);
    }

    fn end_link(&mut self) {
        if self.links.pop() == Some(true) {
            self.text.push_str("</a>");
        }
    }

    fn end_quote(&mut self) {
        self.quote_depth = self.quote_depth.saturating_sub(1);
        if self.quote_depth == 0 && self.lists.is_empty() {
            let markup = std::mem::take(&mut self.text);
            self.blocks.push(Block::Quote(markup));
        }
    }

    fn end_list(&mut self) {
        self.lists.pop();
        if self.lists.is_empty() && self.items.is_some() {
            self.flush_text();
            self.items = None;
        } else if !self.in_container() {
            self.flush_text();
        }
    }

    fn end_cell(&mut self) {
        let markup = std::mem::take(&mut self.text);
        if let Some(table) = self.table.as_mut() {
            let text = std::mem::take(&mut table.text);
            table.row.push(Cell {
                markup: markup.trim().to_owned(),
                text: text.trim().to_owned(),
            });
        }
    }

    fn end_head(&mut self) {
        if let Some(table) = self.table.as_mut() {
            table.head = std::mem::take(&mut table.row);
        }
    }

    fn end_row(&mut self) {
        if let Some(table) = self.table.as_mut() {
            let row = std::mem::take(&mut table.row);
            table.rows.push(row);
        }
    }

    fn end_table(&mut self) {
        if let Some(table) = self.table.take() {
            self.blocks.push(Block::Table(Table {
                head: table.head,
                rows: table.rows,
            }));
        }
    }

    fn end_code(&mut self) {
        let raw = self.raw.take().unwrap_or_default();
        let body = raw.strip_suffix('\n').unwrap_or(&raw).to_owned();
        self.blocks.push(Block::Code(body));
    }

    /// Ends the text being built: a list's rows so far inside a list drawn as
    /// rows, a paragraph anywhere else.
    fn flush_text(&mut self) {
        if self.items.is_some() {
            self.end_item();
            let items = self.items.as_mut().map(std::mem::take).unwrap_or_default();
            if !items.is_empty() {
                self.blocks.push(Block::List(items));
            }
            return;
        }
        let markup = std::mem::take(&mut self.text);
        if !markup.trim().is_empty() {
            self.blocks.push(Block::Text(markup));
        }
    }
}

/// Escaped text with each bare `http(s)://` address made a link. The address
/// ends at whitespace; trailing punctuation, and a closing bracket it did not
/// open, belong to the sentence.
fn linkify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // Each pass consumes at least the scheme it found, so this ends.
    while let Some((start, scheme)) = next_address(rest) {
        out.push_str(&escape(&rest[..start]));
        let tail = &rest[start..];
        let address = &tail[..address_length(tail)];
        if address.len() > scheme {
            let url = escape_attribute(address);
            out.push_str(&format!("<a href=\"{url}\">{}</a>", escape(address)));
        } else {
            out.push_str(&escape(address));
        }
        rest = &tail[address.len().max(scheme)..];
    }
    out.push_str(&escape(rest));
    out
}

/// Where the next web address starts, and the length of its scheme.
fn next_address(text: &str) -> Option<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    ["https://", "http://"]
        .iter()
        .filter_map(|scheme| lower.find(scheme).map(|start| (start, scheme.len())))
        .min()
}

fn address_length(text: &str) -> usize {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let mut address = &text[..end];
    // Each pass drops a character or stops, so the address's length bounds it.
    for _ in 0..=end {
        let trimmed = trim_sentence_end(address);
        if trimmed.len() == address.len() {
            break;
        }
        address = trimmed;
    }
    address.len()
}

fn trim_sentence_end(address: &str) -> &str {
    let trimmed = address.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
    let unmatched = trimmed.matches('(').count() < trimmed.matches(')').count();
    match trimmed.strip_suffix(')') {
        Some(inner) if unmatched => inner,
        _ => trimmed,
    }
}

fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}
