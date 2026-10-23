//! A code block's restrained colours: the stretches of its code that are keywords, strings,
//! numbers and comments, read by a small scanner rather than a grammar, and the Pango markup
//! that draws them. Everything else is plain. A port of the macOS app's `CodeHighlight` and
//! `CodeLanguage` (eb0c5ff, 76dce85).
//!
//! It never fails, and every byte is looked at a bounded number of times, so odd input (a
//! string never closed, one line of a hundred thousand characters) costs time in proportion to
//! its length. A string that does not close on its own line is not a string, so an apostrophe
//! in a log line colours nothing; a string that may run over lines (three quotes, a backtick)
//! runs to the end when it is never closed, as one does while a reply is still being written.

use super::escape;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    String,
    Number,
    Comment,
}

/// One coloured stretch, as byte offsets into the code, each on a character boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

/// The colours a block draws in, as `#rrggbb`: the theme's own, which the app reads for the
/// current appearance. Comments are the code's own ink, faint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inks {
    pub keyword: String,
    pub string: String,
    pub number: String,
}

/// How faint a comment is drawn.
const COMMENT_ALPHA: &str = "55%";

/// A language a fence names: its spellings (lowercase), the name a block's header says, and
/// the syntax its colours follow.
pub struct Language {
    pub tags: &'static [&'static str],
    pub name: &'static str,
    syntax: &'static Syntax,
}

/// How one family of languages writes the four things the scanner colours.
struct Syntax {
    /// What starts a comment to the end of its line, at a line's start or after a space.
    line_comments: &'static [&'static str],
    block_comments: &'static [(&'static str, &'static str)],
    /// The quotes of a string that ends on its own line.
    quotes: &'static [u8],
    /// The quotes of a string that may run over lines: three quotes, a backtick.
    long_quotes: &'static [&'static str],
    /// The quotes with no escapes inside: a shell's single quote.
    raw_quotes: &'static [u8],
    /// Rust's and Go's single quote: a character closed within a few bytes, or not a string.
    character_quote: bool,
    keywords: &'static [&'static str],
    /// SQL's keywords are any case.
    keywords_ignore_case: bool,
}

const C_BLOCK: &[(&str, &str)] = &[("/*", "*/")];
const QUOTES: &[u8] = b"\"'";

const fn plain() -> Syntax {
    Syntax {
        line_comments: &[],
        block_comments: &[],
        quotes: QUOTES,
        long_quotes: &[],
        raw_quotes: &[],
        character_quote: false,
        keywords: &[],
        keywords_ignore_case: false,
    }
}

static UNKNOWN: Syntax = Syntax {
    line_comments: &["#", "//"],
    block_comments: C_BLOCK,
    ..plain()
};

static SHELL: Syntax = Syntax {
    line_comments: &["#"],
    raw_quotes: b"'",
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
        "in", "function", "return", "export", "local", "readonly", "set", "unset", "source",
        "exit", "break", "continue",
    ],
    ..plain()
};

static PYTHON: Syntax = Syntax {
    line_comments: &["#"],
    long_quotes: &["\"\"\"", "'''"],
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
        "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
        "yield", "True", "False", "None", "self", "match", "case",
    ],
    ..plain()
};

static SWIFT: Syntax = Syntax {
    line_comments: &["//"],
    block_comments: C_BLOCK,
    quotes: b"\"",
    long_quotes: &["\"\"\""],
    keywords: &[
        "actor",
        "any",
        "as",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "continue",
        "default",
        "defer",
        "do",
        "else",
        "enum",
        "extension",
        "fallthrough",
        "false",
        "fileprivate",
        "for",
        "func",
        "guard",
        "if",
        "import",
        "in",
        "init",
        "inout",
        "internal",
        "is",
        "let",
        "nil",
        "open",
        "private",
        "protocol",
        "public",
        "repeat",
        "rethrows",
        "return",
        "self",
        "Self",
        "some",
        "static",
        "struct",
        "subscript",
        "super",
        "switch",
        "throw",
        "throws",
        "true",
        "try",
        "typealias",
        "var",
        "where",
        "while",
    ],
    ..plain()
};

static JAVASCRIPT: Syntax = Syntax {
    line_comments: &["//"],
    block_comments: C_BLOCK,
    long_quotes: &["`"],
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "null",
        "of",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "with",
        "yield",
        "interface",
        "type",
        "enum",
        "implements",
        "private",
        "public",
        "protected",
        "readonly",
        "as",
        "declare",
        "namespace",
        "abstract",
        "keyof",
    ],
    ..plain()
};

static JSON: Syntax = Syntax {
    quotes: b"\"",
    keywords: &["true", "false", "null"],
    ..plain()
};

static ELIXIR: Syntax = Syntax {
    line_comments: &["#"],
    long_quotes: &["\"\"\"", "'''"],
    keywords: &[
        "after",
        "alias",
        "and",
        "case",
        "catch",
        "cond",
        "def",
        "defp",
        "defmodule",
        "defmacro",
        "defmacrop",
        "defstruct",
        "defprotocol",
        "defimpl",
        "defdelegate",
        "defguard",
        "do",
        "else",
        "end",
        "fn",
        "for",
        "if",
        "import",
        "in",
        "not",
        "or",
        "quote",
        "raise",
        "receive",
        "require",
        "rescue",
        "try",
        "unless",
        "unquote",
        "use",
        "when",
        "with",
        "true",
        "false",
        "nil",
    ],
    ..plain()
};

static GO: Syntax = Syntax {
    line_comments: &["//"],
    block_comments: C_BLOCK,
    quotes: b"\"",
    long_quotes: &["`"],
    character_quote: true,
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
        "true",
        "false",
        "nil",
        "iota",
    ],
    ..plain()
};

static RUST: Syntax = Syntax {
    line_comments: &["//"],
    block_comments: C_BLOCK,
    quotes: b"\"",
    character_quote: true,
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while",
    ],
    ..plain()
};

static SQL: Syntax = Syntax {
    line_comments: &["--"],
    block_comments: C_BLOCK,
    keywords: &[
        "select",
        "from",
        "where",
        "and",
        "or",
        "not",
        "insert",
        "into",
        "values",
        "update",
        "set",
        "delete",
        "create",
        "table",
        "drop",
        "alter",
        "add",
        "index",
        "on",
        "join",
        "left",
        "right",
        "inner",
        "outer",
        "full",
        "cross",
        "group",
        "by",
        "order",
        "having",
        "limit",
        "offset",
        "as",
        "distinct",
        "union",
        "all",
        "null",
        "is",
        "in",
        "like",
        "between",
        "case",
        "when",
        "then",
        "else",
        "end",
        "exists",
        "primary",
        "key",
        "foreign",
        "references",
        "default",
        "unique",
        "with",
        "returning",
        "true",
        "false",
        "asc",
        "desc",
        "begin",
        "commit",
        "rollback",
        "view",
        "if",
    ],
    keywords_ignore_case: true,
    ..plain()
};

static YAML: Syntax = Syntax {
    line_comments: &["#"],
    keywords: &["true", "false", "null"],
    ..plain()
};

static HTML: Syntax = Syntax {
    block_comments: &[("<!--", "-->")],
    ..plain()
};

static CSS: Syntax = Syntax {
    block_comments: C_BLOCK,
    keywords: &["important", "inherit", "initial", "unset", "none", "auto"],
    ..plain()
};

const fn language(
    tags: &'static [&'static str],
    name: &'static str,
    syntax: &'static Syntax,
) -> Language {
    Language { tags, name, syntax }
}

/// The 42 languages the macOS app names (`chat.code.language.*`), in its order.
static LANGUAGES: [Language; 42] = [
    language(
        &[
            "sh",
            "bash",
            "zsh",
            "shell",
            "console",
            "shellscript",
            "shell-session",
            "terminal",
            "ksh",
            "fish",
        ],
        "Shell",
        &SHELL,
    ),
    language(&["powershell", "ps1", "pwsh"], "PowerShell", &UNKNOWN),
    language(&["bat", "batch", "cmd"], "Batch", &UNKNOWN),
    language(&["python", "py", "python3", "ipython"], "Python", &PYTHON),
    language(&["ruby", "rb"], "Ruby", &UNKNOWN),
    language(&["perl", "pl"], "Perl", &UNKNOWN),
    language(&["lua"], "Lua", &UNKNOWN),
    language(&["php"], "PHP", &UNKNOWN),
    language(&["r"], "R", &UNKNOWN),
    language(&["swift"], "Swift", &SWIFT),
    language(
        &["objective-c", "objectivec", "objc", "obj-c"],
        "Objective-C",
        &UNKNOWN,
    ),
    language(&["kotlin", "kt", "kts"], "Kotlin", &UNKNOWN),
    language(&["java"], "Java", &UNKNOWN),
    language(&["scala"], "Scala", &UNKNOWN),
    language(&["dart"], "Dart", &UNKNOWN),
    language(&["c", "h"], "C", &UNKNOWN),
    language(
        &["cpp", "c++", "cc", "cxx", "hpp", "hh", "hxx"],
        "C++",
        &UNKNOWN,
    ),
    language(&["cs", "csharp", "c#"], "C#", &UNKNOWN),
    language(&["go", "golang"], "Go", &GO),
    language(&["rust", "rs"], "Rust", &RUST),
    language(
        &["javascript", "js", "jsx", "mjs", "cjs", "node"],
        "JavaScript",
        &JAVASCRIPT,
    ),
    language(
        &["typescript", "ts", "tsx", "mts", "cts"],
        "TypeScript",
        &JAVASCRIPT,
    ),
    language(&["elixir", "ex", "exs", "iex"], "Elixir", &ELIXIR),
    language(&["erlang", "erl"], "Erlang", &UNKNOWN),
    language(&["haskell", "hs"], "Haskell", &UNKNOWN),
    language(
        &["sql", "postgres", "postgresql", "psql", "mysql", "sqlite"],
        "SQL",
        &SQL,
    ),
    language(&["graphql", "gql"], "GraphQL", &UNKNOWN),
    language(&["json", "jsonc", "json5"], "JSON", &JSON),
    language(&["yaml", "yml"], "YAML", &YAML),
    language(&["toml"], "TOML", &YAML),
    language(&["ini"], "INI", &UNKNOWN),
    language(&["xml", "plist"], "XML", &HTML),
    language(&["svg"], "SVG", &HTML),
    language(&["html", "htm", "xhtml"], "HTML", &HTML),
    language(&["css"], "CSS", &CSS),
    language(&["scss"], "SCSS", &CSS),
    language(&["less"], "Less", &CSS),
    language(&["markdown", "md"], "Markdown", &UNKNOWN),
    language(&["diff", "patch"], "Diff", &UNKNOWN),
    language(
        &["dockerfile", "docker", "containerfile"],
        "Dockerfile",
        &UNKNOWN,
    ),
    language(&["makefile", "make", "mk"], "Makefile", &UNKNOWN),
    language(&["text", "txt", "plaintext", "plain"], "Text", &UNKNOWN),
];

/// Every language a fence can name.
pub fn languages() -> &'static [Language] {
    &LANGUAGES
}

/// The language a fence's tag names, in any case; `None` for a tag this does not know.
fn language_of(tag: &str) -> Option<&'static Language> {
    let tag = tag.to_lowercase();
    LANGUAGES
        .iter()
        .find(|language| language.tags.contains(&tag.as_str()))
}

/// What a block's header calls the language its fence names: its name where this knows it,
/// and the tag as written where it does not.
pub fn language_name(tag: &str) -> String {
    language_of(tag).map_or_else(|| tag.to_owned(), |language| language.name.to_owned())
}

/// The coloured stretches of `text` in the syntax `tag` names, in order, none overlapping.
/// A tag this does not know, or none, gets strings, numbers and the commonest comments.
pub fn tokens(text: &str, tag: Option<&str>) -> Vec<Token> {
    let syntax = tag
        .and_then(language_of)
        .map_or(&UNKNOWN, |language| language.syntax);
    Scanner::new(text.as_bytes(), syntax).scan()
}

/// `text` as Pango markup in `inks`, every byte escaped.
pub fn markup(text: &str, tag: Option<&str>, inks: &Inks) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut plain = 0;
    for token in tokens(text, tag) {
        let before = text.get(plain..token.start);
        let stretch = text.get(token.start..token.end);
        let (Some(before), Some(stretch)) = (before, stretch) else {
            panic!("a token cut a character: {token:?}");
        };
        out.push_str(&escape(before));
        let open = match token.kind {
            TokenKind::Keyword => format!("<span foreground=\"{}\">", inks.keyword),
            TokenKind::String => format!("<span foreground=\"{}\">", inks.string),
            TokenKind::Number => format!("<span foreground=\"{}\">", inks.number),
            TokenKind::Comment => format!("<span alpha=\"{COMMENT_ALPHA}\">"),
        };
        out.push_str(&open);
        out.push_str(&escape(stretch));
        out.push_str("</span>");
        plain = token.end;
    }
    out.push_str(&escape(&text[plain..]));
    out
}

/// Characters a character literal may hold between its quotes: an escape with what it names
/// (`'\n'`, `'\u{1F600}'`), or one character.
const CHARACTER_REACH: usize = 12;

/// One pass over a block's bytes. A position is looked at once in the main walk; a string that
/// fails to close on its line is looked at once more to find that out, and the next quote of
/// its kind on the same line knows at once it cannot close either.
struct Scanner<'a> {
    bytes: &'a [u8],
    syntax: &'static Syntax,
    tokens: Vec<Token>,
    /// For each ASCII quote, the line end before which it is known not to close.
    unclosed_until: [usize; 128],
    longest_keyword: usize,
}

impl<'a> Scanner<'a> {
    fn new(bytes: &'a [u8], syntax: &'static Syntax) -> Scanner<'a> {
        let longest_keyword = syntax.keywords.iter().map(|k| k.len()).max().unwrap_or(0);
        Scanner {
            bytes,
            syntax,
            tokens: Vec::new(),
            unclosed_until: [0; 128],
            longest_keyword,
        }
    }

    fn scan(mut self) -> Vec<Token> {
        let mut position = 0;
        // Every step moves on by at least one byte, so this ends within the text's length.
        while position < self.bytes.len() {
            let next = self.step(position);
            assert!(next > position, "the scanner stood still at {position}");
            position = next;
        }
        self.tokens
    }

    /// Reads what starts at `position` and answers where the next thing starts.
    fn step(&mut self, position: usize) -> usize {
        if let Some(end) = self
            .block_comment(position)
            .or_else(|| self.line_comment(position))
        {
            return self.mark(TokenKind::Comment, position, end);
        }
        if let Some(end) = self.string(position) {
            return self.mark(TokenKind::String, position, end);
        }
        let byte = self.bytes[position];
        if byte.is_ascii_digit() && !self.follows_word(position) {
            let end = self.run(position, is_number_byte);
            return self.mark(TokenKind::Number, position, end);
        }
        if !is_word_byte(byte) {
            return position + 1;
        }
        let end = self.run(position, is_word_byte);
        if self.is_keyword(position, end) {
            return self.mark(TokenKind::Keyword, position, end);
        }
        end
    }

    fn mark(&mut self, kind: TokenKind, start: usize, end: usize) -> usize {
        self.tokens.push(Token { kind, start, end });
        end
    }

    fn block_comment(&self, position: usize) -> Option<usize> {
        let (open, close) = self
            .syntax
            .block_comments
            .iter()
            .find(|(open, _)| self.matches(open.as_bytes(), position))?;
        let from = position + open.len();
        Some(
            self.find(close.as_bytes(), from)
                .map_or(self.bytes.len(), |at| at + close.len()),
        )
    }

    /// A line comment counts at the start of a line or after a space, so `a#b`, `${#x}` and
    /// the `//` of an address are not comments.
    fn line_comment(&self, position: usize) -> Option<usize> {
        let starts_word = position == 0 || is_space(self.bytes[position - 1]);
        let opens = self
            .syntax
            .line_comments
            .iter()
            .any(|mark| self.matches(mark.as_bytes(), position));
        (starts_word && opens).then(|| self.run(position, |b| !is_newline(b)))
    }

    fn string(&mut self, position: usize) -> Option<usize> {
        let long = self
            .syntax
            .long_quotes
            .iter()
            .find(|quote| self.matches(quote.as_bytes(), position));
        if let Some(quote) = long {
            let from = position + quote.len();
            return Some(
                self.closing(quote.as_bytes(), from)
                    .unwrap_or(self.bytes.len()),
            );
        }
        let quote = self.bytes[position];
        if self.syntax.character_quote && quote == b'\'' {
            return self.character(position);
        }
        if !self.syntax.quotes.contains(&quote) {
            return None;
        }
        self.line_string(quote, position)
    }

    /// A string that ends on its own line, or `None` where it does not close there, which is
    /// then remembered for the rest of the line.
    fn line_string(&mut self, quote: u8, position: usize) -> Option<usize> {
        let slot = usize::from(quote & 0x7f);
        if position < self.unclosed_until[slot] {
            return None;
        }
        let escapes = !self.syntax.raw_quotes.contains(&quote);
        let mut index = position + 1;
        while index < self.bytes.len() && !is_newline(self.bytes[index]) {
            let byte = self.bytes[index];
            if byte == quote {
                return Some(index + 1);
            }
            let escaped = escapes && byte == b'\\' && self.escapes_on_line(index);
            index += if escaped { 2 } else { 1 };
        }
        self.unclosed_until[slot] = index;
        None
    }

    /// Whether a backslash at `index` escapes a byte on its own line.
    fn escapes_on_line(&self, index: usize) -> bool {
        index + 1 < self.bytes.len() && !is_newline(self.bytes[index + 1])
    }

    /// A string that may run over lines, closed by the quote it opened with; `None` where it
    /// never closes. A backtick's string has no escapes.
    fn closing(&self, quote: &[u8], from: usize) -> Option<usize> {
        let escapes = quote != b"`";
        let mut index = from;
        while index < self.bytes.len() {
            if escapes && self.bytes[index] == b'\\' {
                index += 2;
                continue;
            }
            if self.matches(quote, index) {
                return Some(index + quote.len());
            }
            index += 1;
        }
        None
    }

    /// A character literal, `'a'` or `'\n'`, closed within a few bytes; any other single
    /// quote (a Rust lifetime, `'a`) is plain.
    fn character(&self, position: usize) -> Option<usize> {
        let reach = self.bytes.len().min(position + CHARACTER_REACH);
        let mut index = position + 1;
        if index >= reach || self.bytes[index] == b'\'' {
            return None;
        }
        if self.bytes[index] == b'\\' {
            index += 2;
            let open = |b: u8| b != b'\'' && !is_newline(b);
            while index < reach && open(self.bytes[index]) {
                index += 1;
            }
        } else {
            index += scalar_length(self.bytes[index]);
        }
        (index < reach && self.bytes[index] == b'\'').then_some(index + 1)
    }

    fn is_keyword(&self, start: usize, end: usize) -> bool {
        if end - start > self.longest_keyword {
            return false;
        }
        let word = &self.bytes[start..end];
        let ignore_case = self.syntax.keywords_ignore_case;
        let same = |keyword: &&str| {
            let keyword = keyword.as_bytes();
            keyword == word || (ignore_case && keyword.eq_ignore_ascii_case(word))
        };
        self.syntax.keywords.iter().any(same)
    }

    /// Whether a digit is inside a word (`utf8`, `x2`), and so no number.
    fn follows_word(&self, position: usize) -> bool {
        position > 0 && is_word_byte(self.bytes[position - 1])
    }

    fn run(&self, start: usize, belongs: fn(u8) -> bool) -> usize {
        let rest = &self.bytes[start..];
        start + rest.iter().position(|&b| !belongs(b)).unwrap_or(rest.len())
    }

    fn find(&self, mark: &[u8], from: usize) -> Option<usize> {
        let rest = self.bytes.get(from..)?;
        rest.windows(mark.len())
            .position(|window| window == mark)
            .map(|at| from + at)
    }

    fn matches(&self, mark: &[u8], position: usize) -> bool {
        self.bytes
            .get(position..position + mark.len())
            .is_some_and(|here| here == mark)
    }
}

/// A word runs over letters, digits, `_`, `$` and every byte of a character outside ASCII.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

/// A number runs over digits, letters, `_` and `.`: `0x1F`, `1_000`, `2.5e10`, `10px`, `3u8`.
fn is_number_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.'
}

fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t' || is_newline(byte)
}

fn is_newline(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

/// How many bytes the character a lead byte starts takes.
fn scalar_length(byte: u8) -> usize {
    match byte {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}
