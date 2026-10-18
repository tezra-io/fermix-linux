//! A code block's restrained colours: which language a fence names, which stretches of its
//! code are keywords, strings, numbers and comments, and the Pango markup that draws them.
//! A port of the macOS app's `CodeHighlight` and `CodeLanguage`.

use fermix_client::markdown::highlight::{
    language_name, languages, markup, tokens, Inks, Token, TokenKind,
};
use fermix_client::markdown::{render, Block};

/// The stretches of `text` in `tag`'s syntax, as (kind, the text it covers).
fn spans<'a>(text: &'a str, tag: Option<&str>) -> Vec<(TokenKind, &'a str)> {
    tokens(text, tag)
        .iter()
        .map(|token| (token.kind, &text[token.start..token.end]))
        .collect()
}

fn inks() -> Inks {
    Inks {
        keyword: "#aa00aa".into(),
        string: "#00aa00".into(),
        number: "#aa5500".into(),
    }
}

use TokenKind::{Comment, Keyword, Number, String as Str};

#[test]
fn a_fence_tag_names_its_language_in_any_case_and_an_unknown_one_as_written() {
    assert_eq!(language_name("py"), "Python");
    assert_eq!(language_name("TS"), "TypeScript");
    assert_eq!(language_name("c++"), "C++");
    assert_eq!(language_name("Bash"), "Shell");
    assert_eq!(language_name("yml"), "YAML");
    assert_eq!(language_name("zig"), "zig");
    assert_eq!(language_name("MyLang"), "MyLang");
}

#[test]
fn every_spelling_names_one_language_and_there_are_the_macs_42() {
    let all = languages();
    assert_eq!(all.len(), 42);
    let mut tags: Vec<&str> = all
        .iter()
        .flat_map(|language| language.tags.iter().copied())
        .collect();
    let count = tags.len();
    tags.sort_unstable();
    tags.dedup();
    assert_eq!(tags.len(), count, "a spelling given to two languages");
    assert!(
        tags.iter().all(|tag| *tag == tag.to_lowercase()),
        "tags are lowercase"
    );
}

#[test]
fn rust_colours_keywords_strings_numbers_and_comments() {
    let code = "fn main() {\n    let s = \"hi\"; // note\n    let n = 42;\n}";
    assert_eq!(
        spans(code, Some("rust")),
        [
            (Keyword, "fn"),
            (Keyword, "let"),
            (Str, "\"hi\""),
            (Comment, "// note"),
            (Keyword, "let"),
            (Number, "42"),
        ]
    );
}

#[test]
fn a_line_comment_counts_only_at_a_line_start_or_after_a_space() {
    assert_eq!(
        spans("echo a#b ${#x} # real", Some("sh")),
        [(Comment, "# real")]
    );
    assert_eq!(spans("curl https://a.io", Some("go")), []);
}

#[test]
fn a_string_that_does_not_close_on_its_line_is_no_string() {
    assert_eq!(
        spans("it's a log line\nand 'so' on", Some("python")),
        [(Keyword, "and"), (Str, "'so'")]
    );
    assert_eq!(spans("say \"unclosed\nnext", Some("js")), []);
}

#[test]
fn a_long_string_runs_over_lines_and_to_the_end_while_still_being_written() {
    assert_eq!(
        spans("x = \"\"\"a\nb\"\"\" + 1", Some("python")),
        [(Str, "\"\"\"a\nb\"\"\""), (Number, "1")]
    );
    assert_eq!(spans("x = `a\nb", Some("ts")), [(Str, "`a\nb")]);
}

#[test]
fn a_rust_lifetime_is_no_string_and_a_character_is() {
    assert_eq!(
        spans("fn f<'a>(c: char) { c == 'x' || c == '\\n' }", Some("rs")),
        [(Keyword, "fn"), (Str, "'x'"), (Str, "'\\n'"),]
    );
}

#[test]
fn a_shell_single_quote_has_no_escapes() {
    assert_eq!(spans(r"echo 'a\' b", Some("bash")), [(Str, r"'a\'")]);
}

#[test]
fn sql_keywords_are_any_case() {
    assert_eq!(
        spans("SELECT name from t -- all", Some("sql")),
        [(Keyword, "SELECT"), (Keyword, "from"), (Comment, "-- all")]
    );
}

#[test]
fn a_digit_inside_a_word_is_no_number() {
    assert_eq!(
        spans("utf8 x2 0x1F 1_000 2.5e10", Some("python")),
        [(Number, "0x1F"), (Number, "1_000"), (Number, "2.5e10")]
    );
}

#[test]
fn an_unknown_language_gets_strings_numbers_and_the_commonest_comments_but_no_keywords() {
    assert_eq!(
        spans("fn x = \"a\" 7 // c\n# d\n/* e */", Some("zig")),
        [
            (Str, "\"a\""),
            (Number, "7"),
            (Comment, "// c"),
            (Comment, "# d"),
            (Comment, "/* e */"),
        ]
    );
    assert_eq!(spans("fn x", None), []);
}

#[test]
fn a_block_comment_never_closed_runs_to_the_end() {
    assert_eq!(
        spans("a /* open\nstill", Some("c")),
        [(Comment, "/* open\nstill")]
    );
}

#[test]
fn html_and_css_colour_only_what_they_have() {
    assert_eq!(
        spans("<p>hi</p><!-- c -->", Some("html")),
        [(Comment, "<!-- c -->")]
    );
    assert_eq!(
        spans("a { color: red !important; width: 10px }", Some("css")),
        [(Keyword, "important"), (Number, "10px")]
    );
}

/// Odd input costs time in proportion to its length, and every stretch is in order, apart,
/// and on character boundaries, so the markup can always cut the text there.
#[test]
fn odd_input_is_scanned_in_one_pass_and_cut_only_at_characters() {
    let long = format!("let x = \"{}", "é".repeat(50_000));
    let quotes = "'".repeat(20_000) + &"\"".repeat(20_000);
    let mixed = "日本 'é' \"漢字\" 1é2 // コメント\n/* ü */ `ß";
    for (text, tag) in [
        (long.as_str(), Some("rust")),
        (quotes.as_str(), Some("python")),
        (mixed, Some("go")),
        (mixed, Some("js")),
        (mixed, None),
    ] {
        let found = tokens(text, tag);
        assert!(found.len() <= text.len());
        let mut end = 0;
        for Token {
            start, end: stop, ..
        } in &found
        {
            assert!(*start >= end && start < stop, "in order and apart");
            assert!(text.is_char_boundary(*start) && text.is_char_boundary(*stop));
            end = *stop;
        }
        markup(text, tag, &inks());
    }
}

#[test]
fn the_markup_colours_the_stretches_and_escapes_every_byte() {
    assert_eq!(
        markup("if a<b { \"x&y\" } // <c>", Some("rust"), &inks()),
        "<span foreground=\"#aa00aa\">if</span> a&lt;b { \
         <span foreground=\"#00aa00\">\"x&amp;y\"</span> } \
         <span alpha=\"55%\">// &lt;c&gt;</span>"
    );
    assert_eq!(
        markup("n = 3", Some("python"), &inks()),
        "n = <span foreground=\"#aa5500\">3</span>"
    );
    assert_eq!(
        markup("<plain> & text", None, &inks()),
        "&lt;plain&gt; &amp; text"
    );
}

/// Windows line ends and tabs: the parse makes each `\r\n` a `\n` (so the label and Copy hold the
/// same text) and keeps tabs; the markup escapes neither away, a tab starts a comment as a
/// space does, and a `\r` still left in code ends one as a `\n` does.
#[test]
fn line_ends_and_tabs_pass_through_render_and_the_markup() {
    let blocks = render("```rust\r\n\tlet x = 1;\t// c\r\nfn\r\n```");
    let [Block::Code { language, text }] = blocks.as_slice() else {
        panic!("one code block");
    };
    assert_eq!(text, "\tlet x = 1;\t// c\nfn");
    assert_eq!(
        markup(text, language.as_deref(), &inks()),
        "\t<span foreground=\"#aa00aa\">let</span> x = <span foreground=\"#aa5500\">1</span>;\t\
         <span alpha=\"55%\">// c</span>\n<span foreground=\"#aa00aa\">fn</span>"
    );
    assert_eq!(
        markup("\t// c\r\nfn", Some("rust"), &inks()),
        "\t<span alpha=\"55%\">// c</span>\r\n<span foreground=\"#aa00aa\">fn</span>"
    );
}
