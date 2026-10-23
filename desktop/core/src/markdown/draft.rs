//! A reply while it streams. Parsing the whole draft on every piece costs more
//! the longer the reply grows, so the blocks a blank line has closed are parsed
//! once and kept, and each piece re-parses only the part still being written.
//! A cut is made only where Markdown cannot join what follows to what came
//! before, so the kept blocks and the tail always equal a parse of the whole.

use super::{render, Block};

#[derive(Debug, Default)]
pub struct Draft {
    /// The text the settled blocks were parsed from: everything before the cut.
    prefix: String,
    settled: Vec<Block>,
    /// A reference definition can change a link anywhere above it, and an HTML
    /// block can run on past blank lines, so a draft with either outside code
    /// parses whole from then on.
    whole: bool,
}

impl Draft {
    /// The blocks a blank line has closed, which no later piece changes.
    pub fn settled(&self) -> &[Block] {
        &self.settled
    }

    /// Takes the draft as it stands and returns the blocks after the settled
    /// ones. A text that does not continue the last one starts over.
    pub fn update(&mut self, text: &str) -> Vec<Block> {
        if !text.starts_with(self.prefix.as_str()) {
            *self = Draft::default();
        }
        if self.whole {
            return render(text);
        }
        let start = self.prefix.len();
        let Some(cut) = last_cut(&text[start..]).map(|cut| start + cut) else {
            *self = Draft {
                whole: true,
                ..Draft::default()
            };
            return render(text);
        };
        if cut > start {
            self.settled.extend(render(&text[start..cut]));
            self.prefix.push_str(&text[start..cut]);
        }
        render(&text[cut..])
    }
}

/// The byte offset of the last line in `text` that starts a block nothing
/// above it can continue; 0 when there is none, and None when the text has a
/// line outside code that can reach back past a cut.
///
/// The line has to follow a blank line outside a code fence, and start at
/// the margin with a character that opens no list item, quote or HTML block,
/// all of which can carry on across a blank line.
fn last_cut(text: &str) -> Option<usize> {
    let mut cut = 0;
    let mut fence: Option<(char, usize)> = None;
    let mut after_blank = false;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let complete = line.ends_with('\n');
        let content = line.trim_end_matches(['\n', '\r']);
        if fence.is_none() && reaches_back(content) {
            return None;
        }
        if fence.is_none() && after_blank && opens_unjoinable_block(content) {
            cut = start;
        }
        fence = next_fence(fence, content);
        after_blank = complete && content.trim().is_empty();
    }
    Some(cut)
}

fn opens_unjoinable_block(line: &str) -> bool {
    match line.chars().next() {
        None => false,
        Some(first) => {
            !(first.is_whitespace()
                || first.is_ascii_digit()
                || matches!(first, '-' | '*' | '+' | '>' | '<' | '['))
        }
    }
}

/// The fence open after `line`: a run of three or more backticks or tildes at
/// most three spaces in opens one, and a run of the same mark at least as long
/// closes it.
fn next_fence(open: Option<(char, usize)>, line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return open;
    }
    let body = &line[indent..];
    let Some(mark) = body.chars().next().filter(|c| matches!(c, '`' | '~')) else {
        return open;
    };
    let run = body.len() - body.trim_start_matches(mark).len();
    match open {
        _ if run < 3 => open,
        None => Some((mark, run)),
        Some((open_mark, open_run)) if open_mark == mark && run >= open_run => None,
        Some(_) => open,
    }
}

/// Whether the line could be a link reference definition, `[name]: …`, or
/// open an HTML block.
fn reaches_back(line: &str) -> bool {
    let line = line.trim_start_matches(' ');
    line.starts_with('<') || (line.starts_with('[') && line.contains("]:"))
}
