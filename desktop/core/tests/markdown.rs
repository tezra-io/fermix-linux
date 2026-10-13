//! Replies arrive as Markdown. They are drawn as Pango markup blocks, and every
//! byte of model text is escaped before it becomes markup.

use fermix_client::markdown::{render, Block};

#[test]
fn plain_text_is_one_escaped_paragraph() {
    assert_eq!(
        render("Tom & Jerry <3"),
        [Block::Text("Tom &amp; Jerry &lt;3".into())]
    );
}

#[test]
fn emphasis_code_and_links_become_markup() {
    assert_eq!(
        render("**bold** *it* `x<y` [site](https://example.com/?a=1&b=\"2\")"),
        [Block::Text(
            "<b>bold</b> <i>it</i> <tt>x&lt;y</tt> <a href=\"https://example.com/?a=1&amp;b=&quot;2&quot;\">site</a>"
                .into()
        )]
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
            Block::Text("•  one\n•  two <b>2</b>".into()),
            Block::Text("1.  a\n2.  b".into()),
        ]
    );
}

#[test]
fn a_code_block_keeps_its_text_raw_for_a_monospace_view() {
    assert_eq!(
        render("Run:\n\n```sh\necho <hi> && ls\n```\n"),
        [
            Block::Text("Run:".into()),
            Block::Code("echo <hi> && ls".into())
        ]
    );
}

#[test]
fn an_unfinished_code_fence_mid_stream_still_renders() {
    assert_eq!(
        render("```\nlet x = 1;"),
        [Block::Code("let x = 1;".into())]
    );
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
fn a_table_is_kept_as_monospace_text_one_row_per_line() {
    assert_eq!(
        render("| a | bb |\n|---|---|\n| 1 | 2 |\n"),
        [Block::Code("a | bb\n1 | 2".into())]
    );
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
            "<a href=\"https://a.io\">https://a.io</a> <tt>https://b.io</tt> <a href=\"https://c.io\">https://c.io</a>"
                .into()
        )]
    );
    assert_eq!(
        render("```\ncurl https://d.io\n```"),
        [Block::Code("curl https://d.io".into())]
    );
}

#[test]
fn a_scheme_alone_is_not_a_link() {
    assert_eq!(
        render("type https:// then the host"),
        [Block::Text("type https:// then the host".into())]
    );
}
