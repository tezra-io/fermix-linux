//! Replies arrive as Markdown. They are drawn as Pango markup blocks, and every
//! byte of model text is escaped before it becomes markup.

use fermix_client::markdown::{render, Block, Cell, Draft, Item, Table, CODE_SPAN_OPEN};

#[test]
fn plain_text_is_one_escaped_paragraph() {
    assert_eq!(
        render("Tom & Jerry <3"),
        [Block::Text("Tom &amp; Jerry &lt;3".into())]
    );
}

/// A code span as the renderer draws it: monospace on the faint fill.
fn span(code: &str) -> String {
    format!("{CODE_SPAN_OPEN}{code}</tt></span>")
}

fn code(language: Option<&str>, text: &str) -> Block {
    Block::Code {
        language: language.map(str::to_owned),
        text: text.into(),
    }
}

#[test]
fn emphasis_code_and_links_become_markup() {
    assert_eq!(
        render("**bold** *it* `x<y` [site](https://example.com/?a=1&b=\"2\")"),
        [Block::Text(format!(
            "<b>bold</b> <i>it</i> {} <a href=\"https://example.com/?a=1&amp;b=&quot;2&quot;\">site</a>",
            span("x&lt;y")
        ))]
    );
}

/// A light grey at 18% lands on a white card at 238 and on a dark one (51) at 71: a step
/// stronger than a code block's 7% panel in both, as the macOS app's span fill is.
#[test]
fn a_code_span_sits_on_a_faint_fill_that_reads_in_light_and_dark() {
    assert!(CODE_SPAN_OPEN.contains("bgcolor=\"#a0a0a0\""));
    assert!(CODE_SPAN_OPEN.contains("bgalpha=\"18%\""));
    assert!(CODE_SPAN_OPEN.ends_with("<tt>"));
}

#[test]
fn a_fence_names_its_language_by_the_first_word_of_its_info() {
    assert_eq!(
        render("```python title=x\nprint(1)\n```"),
        [code(Some("python"), "print(1)")]
    );
    assert_eq!(render("```C++\nint x;\n```"), [code(Some("C++"), "int x;")]);
    assert_eq!(render("```\nplain\n```"), [code(None, "plain")]);
    assert_eq!(render("    indented\n"), [code(None, "indented")]);
}

/// A fence's tag is plain text by contract, never markup: it reaches the block as written, and
/// the app sets it as a label's text.
#[test]
fn a_fence_tag_with_markup_in_it_reaches_the_block_as_written() {
    assert_eq!(
        render("```<b>x</b>\ncode\n```"),
        [code(Some("<b>x</b>"), "code")]
    );
}

#[test]
fn a_link_to_anything_but_the_web_or_mail_stays_plain_text() {
    assert_eq!(
        render("[run](file:///etc/passwd) [mail](mailto:a@b.c)"),
        [Block::Text("run <a href=\"mailto:a@b.c\">mail</a>".into())]
    );
}

#[test]
fn headings_paragraphs_and_lists_are_separate_blocks() {
    let blocks = render("# Plan\n\nFirst para.\n\n- one\n- two **2**\n\n1. a\n2. b\n");
    assert_eq!(
        blocks,
        [
            Block::Heading(1, "Plan".into()),
            Block::Text("First para.".into()),
            Block::List(vec![item(0, "•", "one"), item(0, "•", "two <b>2</b>")]),
            Block::List(vec![item(0, "1.", "a"), item(0, "2.", "b")]),
        ]
    );
}

#[test]
fn a_code_block_keeps_its_text_raw_for_a_monospace_view() {
    assert_eq!(
        render("Run:\n\n```sh\necho <hi> && ls\n```\n"),
        [
            Block::Text("Run:".into()),
            code(Some("sh"), "echo <hi> && ls")
        ]
    );
}

#[test]
fn an_unfinished_code_fence_mid_stream_still_renders() {
    assert_eq!(render("```\nlet x = 1;"), [code(None, "let x = 1;")]);
}

#[test]
fn a_quote_is_marked_and_a_rule_is_its_own_block() {
    assert_eq!(
        render("> careful\n\n---\n\nafter"),
        [
            Block::Quote("careful".into()),
            Block::Rule,
            Block::Text("after".into())
        ]
    );
}

#[test]
fn soft_and_hard_breaks_keep_the_lines() {
    assert_eq!(
        render("one\ntwo  \nthree"),
        [Block::Text("one\ntwo\nthree".into())]
    );
}

#[test]
fn a_table_keeps_its_cells_with_their_markup_and_text() {
    assert_eq!(
        render("| a | **bb** |\n|---|---|\n| 1 & 2 | [x](https://x.io) |\n"),
        [Block::Table(Table {
            head: vec![cell("a", "a"), cell("<b>bb</b>", "bb")],
            rows: vec![vec![
                cell("1 &amp; 2", "1 & 2"),
                cell("<a href=\"https://x.io\">x</a>", "x"),
            ]],
        })]
    );
}

#[test]
fn a_table_copies_as_tab_separated_lines() {
    let blocks = render("| a | b |\n|---|---|\n| 1 | `2` |\n| 3 |  |\n");
    let [Block::Table(table)] = &blocks[..] else {
        panic!("one table, got {blocks:?}");
    };
    assert_eq!(table.tab_text(), "a\tb\n1\t2\n3\t");
}

#[test]
fn a_nested_list_keeps_each_item_on_its_own_row_with_its_depth() {
    assert_eq!(
        render("1. one\n   - inner *x*\n   - [ ] task\n2. two\n\n   more\n"),
        [Block::List(vec![
            item(0, "1.", "one"),
            item(1, "•", "inner <i>x</i>"),
            item(1, "☐", "task"),
            item(0, "2.", "two\nmore"),
        ])]
    );
}

#[test]
fn a_code_block_inside_a_list_splits_it_around_the_code() {
    assert_eq!(
        render("- a\n\n  ```\n  x\n  ```\n- b\n"),
        [
            Block::List(vec![item(0, "•", "a")]),
            code(None, "x"),
            Block::List(vec![item(0, "•", "b")]),
        ]
    );
}

#[test]
fn a_list_inside_a_quote_stays_in_the_quote() {
    assert_eq!(
        render("> - a\n> - b\n"),
        [Block::Quote("•  a\n•  b".into())]
    );
}

fn item(depth: u8, marker: &str, markup: &str) -> Item {
    Item {
        depth,
        marker: marker.into(),
        markup: markup.into(),
    }
}

fn cell(markup: &str, text: &str) -> Cell {
    Cell {
        markup: markup.into(),
        text: text.into(),
    }
}

const LONG_REPLY: &str = "# Plan\n\nIntro with a [link](https://a.io) and https://b.io/x.\n\n\
- one\n- two\n\n  still two\n- three\n\n\
1. first\n2. second\n\n10 apples is a lot.\n\n\
> quoted\n> more\n\n\
```rust\nfn main() {\n\n    let x = 1;\n}\n```\n\n\
| col | other |\n|-----|-------|\n| 1 | two |\n| 3 | *four* |\n\n\
<pre>\n\nraw\n</pre>\n\nTail paragraph.\n\n---\n\nLast.";

#[test]
fn a_draft_fed_piece_by_piece_always_matches_a_whole_parse() {
    for step in [1, 2, 3, 7, 16] {
        let mut draft = Draft::default();
        let mut end = 0;
        while end < LONG_REPLY.len() {
            end = (end + step).min(LONG_REPLY.len());
            while !LONG_REPLY.is_char_boundary(end) {
                end += 1;
            }
            let text = &LONG_REPLY[..end];
            let tail = draft.update(text);
            let drawn: Vec<Block> = draft.settled().iter().cloned().chain(tail).collect();
            assert_eq!(drawn, render(text), "step {step}, at {end} bytes");
        }
    }
}

#[test]
fn a_draft_settles_what_a_blank_line_closed() {
    let mut draft = Draft::default();
    draft.update("First.\n\nSecond.\n\nThi");
    assert_eq!(
        draft.settled(),
        [Block::Text("First.".into()), Block::Text("Second.".into())]
    );
}

#[test]
fn a_draft_with_a_reference_definition_parses_whole() {
    let text = "See [a].\n\nMore.\n\n[a]: https://a.io\n";
    let mut draft = Draft::default();
    let mut drawn = Vec::new();
    for end in 1..=text.len() {
        let tail = draft.update(&text[..end]);
        drawn = draft.settled().iter().cloned().chain(tail).collect();
    }
    assert_eq!(drawn, render(text));
}

#[test]
fn a_draft_given_a_different_text_starts_over() {
    let mut draft = Draft::default();
    draft.update("Old.\n\nGone.\n\nx");
    let tail = draft.update("New.");
    assert!(draft.settled().is_empty());
    assert_eq!(tail, [Block::Text("New.".into())]);
}

#[test]
fn a_bare_web_address_becomes_a_link() {
    assert_eq!(
        render("See https://example.com/a?b=1&c=2."),
        [Block::Text(
            "See <a href=\"https://example.com/a?b=1&amp;c=2\">https://example.com/a?b=1&amp;c=2</a>."
                .into()
        )]
    );
}

#[test]
fn a_bare_address_keeps_only_the_brackets_that_are_its_own() {
    assert_eq!(
        render("(https://en.wikipedia.org/wiki/Rust_(language))"),
        [Block::Text(
            "(<a href=\"https://en.wikipedia.org/wiki/Rust_(language)\">https://en.wikipedia.org/wiki/Rust_(language)</a>)"
                .into()
        )]
    );
    assert_eq!(
        render("(see http://a.io/x), then"),
        [Block::Text(
            "(see <a href=\"http://a.io/x\">http://a.io/x</a>), then".into()
        )]
    );
}

#[test]
fn text_already_linked_or_in_code_is_not_linked_again() {
    assert_eq!(
        render("[https://a.io](https://a.io) `https://b.io` <https://c.io>"),
        [Block::Text(
            format!(
                "<a href=\"https://a.io\">https://a.io</a> {} <a href=\"https://c.io\">https://c.io</a>",
                span("https://b.io")
            )
        )]
    );
    assert_eq!(
        render("```\ncurl https://d.io\n```"),
        [code(None, "curl https://d.io")]
    );
}

#[test]
fn a_scheme_alone_is_not_a_link() {
    assert_eq!(
        render("type https:// then the host"),
        [Block::Text("type https:// then the host".into())]
    );
}
