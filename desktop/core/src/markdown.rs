//! Replies arrive as Markdown and are drawn as blocks of Pango markup. Every byte
//! of model text is escaped before it becomes markup, and only web and mail links
//! stay clickable, so a reply can never inject markup or open a local file.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Pango markup: a paragraph, or a whole list with one item per line.
    Text(String),
    /// Heading level (1 to 6) and its markup.
    Heading(u8, String),
    /// Markup, drawn indented and dim.
    Quote(String),
    /// Raw text for a monospace view: never markup.
    Code(String),
    Rule,
}

pub fn render(markdown: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut renderer = Renderer::default();
    for event in Parser::new_ext(markdown, options) {
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
    /// Raw text of the code block or table being built.
    raw: Option<String>,
    table_row: Vec<String>,
    table_rows: Option<Vec<String>>,
    /// One entry per open list: the next number, or None for bullets.
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    /// One entry per open link: whether it was opened as markup.
    links: Vec<bool>,
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
            Event::TaskListMarker(done) => self.text.push_str(if done { "☑ " } else { "☐ " }),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        if matches!(tag, Tag::CodeBlock(_) | Tag::Table(_)) {
            self.flush_text();
            self.raw = Some(String::new());
            return;
        }
        match tag {
            Tag::Paragraph if self.in_container() && !self.text.is_empty() => {
                self.text.push('\n');
            }
            Tag::Heading { .. } => self.flush_text(),
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::List(first) => self.lists.push(first),
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
            TagEnd::CodeBlock => self.end_raw_block(),
            TagEnd::TableCell => self.end_cell(),
            TagEnd::TableHead | TagEnd::TableRow => self.end_row(),
            TagEnd::Table => self.end_raw_block(),
            _ => {}
        }
    }

    fn in_container(&self) -> bool {
        !self.lists.is_empty() || self.quote_depth > 0
    }

    fn push_text(&mut self, text: &str) {
        match self.raw.as_mut() {
            Some(raw) => raw.push_str(text),
            None => self.text.push_str(&escape(text)),
        }
    }

    fn push_code_span(&mut self, code: &str) {
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

    fn start_item(&mut self) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        let depth = self.lists.len().saturating_sub(1);
        self.text.push_str(&"    ".repeat(depth));
        let marker = match self.lists.last_mut() {
            Some(Some(number)) => {
                let marker = format!("{number}.  ");
                *number += 1;
                marker
            }
            _ => "•  ".to_owned(),
        };
        self.text.push_str(&marker);
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
        if !self.in_container() {
            self.flush_text();
        }
    }

    fn end_cell(&mut self) {
        let cell = self.raw.replace(String::new()).unwrap_or_default();
        self.table_row.push(cell.trim().to_owned());
    }

    fn end_row(&mut self) {
        let row = std::mem::take(&mut self.table_row).join(" | ");
        self.text_rows().push(row);
    }

    /// Table rows wait here, in `text`'s place, until the table ends.
    fn text_rows(&mut self) -> &mut Vec<String> {
        self.table_rows.get_or_insert_with(Vec::new)
    }

    fn end_raw_block(&mut self) {
        let raw = self.raw.take().unwrap_or_default();
        let body = match self.table_rows.take() {
            Some(rows) => rows.join("\n"),
            None => raw.strip_suffix('\n').unwrap_or(&raw).to_owned(),
        };
        self.blocks.push(Block::Code(body));
    }

    fn flush_text(&mut self) {
        let markup = std::mem::take(&mut self.text);
        if !markup.trim().is_empty() {
            self.blocks.push(Block::Text(markup));
        }
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
