//! What a row shows under its words: its voice note, its tiles, the link cards of its stored
//! previews, and its text less the lines that only name what the strip shows (macOS
//! `ChatStrip`); and the bounds on reading the pictures.

use fermix_client::chat_wire::protocol::{decode_line, Block, LinkPreview, Row, ServerEvent};
use fermix_client::media::{
    decode_size, format_size, lane, link_cards, save_name, strip, tile, within_ceiling,
    without_shown_lines, words, Admission, Fit, TileKind, MAX_DECODING, MAX_WAITING,
};
use fermix_client::places::opened_by;
use serde_json::{json, Value};

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");
const LAMP: &str = "fe07114ad9867f226ff7c3f455645de3a1bde0a29509b3c5631768faba215a70";
const LAMP_PICTURE: &str = "98c18f914067176b153e5c7f4e0a25adf34563bd82b73f9bbfbd659118698382";
const NOTE: &str = "9879f6d3c9efefe8c4f4aabe9d3269762264be52794e96f62ab8ffa53b7fadec";

/// The vendored row `seq`.
fn fixture_row(seq: u64) -> Row {
    for line in SERVER.lines() {
        match decode_line(line.as_bytes()) {
            Ok(ServerEvent::Row(row)) if row.server_seq == seq => return row,
            _ => {}
        }
    }
    panic!("no row {seq} in the fixtures");
}

/// A row with `extra` over a user row that says nothing.
fn row(extra: Value) -> Row {
    let mut value = json!({"type": "row", "profile_id": "main", "server_seq": 1, "role": "user",
        "text": "", "media_refs": []});
    for (key, field) in extra.as_object().expect("an object") {
        value[key] = field.clone();
    }
    let ServerEvent::Row(row) = decode_line(value.to_string().as_bytes()).expect("a row") else {
        panic!("a row");
    };
    row
}

fn media(r: &str, kind: &str, mime: &str) -> Value {
    json!({"ref": r, "kind": kind, "mime": mime, "size_bytes": 48213})
}

fn preview(fields: Value) -> LinkPreview {
    let mut value = json!({"url": "https://a.test/x", "site": "A", "title": "Page A"});
    for (key, field) in fields.as_object().expect("an object") {
        value[key] = field.clone();
    }
    serde_json::from_value(value).expect("a preview")
}

fn places(canonical: &str, source: &str) -> Vec<Block> {
    let place = json!({"name": "Spot", "location": {"latitude": 1.0, "longitude": 2.0},
        "canonical_url": canonical, "source_url": source});
    let reply = json!({"type": "text_done", "turn_id": "t", "server_seq": 1, "text": "",
        "blocks": [{"type": "places", "places": [place]}]});
    let ServerEvent::TextDone(reply) = decode_line(reply.to_string().as_bytes()).expect("a reply")
    else {
        panic!("a reply");
    };
    reply.blocks
}

/// A phone's voice note is the row's note, and the row says nothing else.
#[test]
fn a_voice_note_row_draws_its_note_alone() {
    let row = fixture_row(21);
    let shown = strip(&row);
    assert_eq!(
        shown.voice.as_ref().map(|note| note.r#ref.as_str()),
        Some(NOTE)
    );
    assert!(shown.tiles.is_empty());
    assert!(shown.links.is_empty());
    assert_eq!(words(&row, &shown), "");
    assert!(!shown.is_empty());
}

/// A picture and the stored preview of the page the text links, under words left as written:
/// the address stands in a sentence.
#[test]
fn a_picture_row_draws_its_tile_and_its_link_card() {
    let row = fixture_row(23);
    let shown = strip(&row);
    assert_eq!(shown.voice, None);
    assert_eq!(shown.tiles.len(), 1);
    let lamp = &shown.tiles[0];
    assert_eq!(lamp.media.r#ref, LAMP);
    assert_eq!(lamp.kind, TileKind::Picture);
    assert_eq!(lamp.name, "lamp.jpg");
    assert_eq!(lamp.detail, "Picture · 182 kB");
    assert_eq!(lamp.spoken, "Picture, lamp.jpg");
    assert_eq!(shown.links.len(), 1);
    let card = &shown.links[0];
    assert_eq!(card.opens, "https://shop.test/lamp");
    assert_eq!(card.title, "Brass desk lamp");
    assert_eq!(card.site, "Shop");
    assert_eq!(card.initial, "S");
    assert_eq!(card.image_ref.as_deref(), Some(LAMP_PICTURE));
    assert_eq!(card.spoken, "Brass desk lamp, Link");
    assert_eq!(
        words(&row, &shown),
        "Is this the lamp? https://shop.test/lamp"
    );
}

/// Only what this app draws is a picture: the rest is a file that opens outside, pictures first.
#[test]
fn every_picture_this_app_draws_is_a_picture_tile_and_the_rest_files() {
    let refs: Vec<Value> = [
        ("application/pdf", "file"),
        ("image/jpeg", "image"),
        ("image/png", "image"),
        ("image/webp", "image"),
        ("image/gif", "image"),
        ("image/heic", "image"),
        ("image/svg+xml", "image"),
        ("video/mp4", "video"),
        ("audio/mp4", "audio"),
    ]
    .iter()
    .enumerate()
    .map(|(index, (mime, kind))| media(&format!("{index:064x}"), kind, mime))
    .collect();
    let shown = strip(&row(json!({"media_refs": refs})));
    let kinds: Vec<(&str, TileKind)> = shown
        .tiles
        .iter()
        .map(|tile| (tile.media.mime.as_str(), tile.kind))
        .collect();
    use TileKind::{File, Picture};
    assert_eq!(
        kinds,
        [
            ("image/jpeg", Picture),
            ("image/png", Picture),
            ("image/webp", Picture),
            ("image/gif", Picture),
            ("image/heic", Picture),
            ("application/pdf", File),
            ("image/svg+xml", File),
            ("video/mp4", File),
            ("audio/mp4", File),
        ]
    );
    let names: Vec<&str> = shown.tiles.iter().map(|tile| tile.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Picture", "Picture", "Picture", "Picture", "Picture", "PDF", "Picture", "Video",
            "Sound"
        ]
    );
}

/// The first Ogg note is the row's note; any other sound is a file, never dropped. A ref named
/// twice is one tile.
#[test]
fn a_second_note_and_any_other_sound_are_file_tiles() {
    let refs = json!([
        media(NOTE, "audio", "audio/ogg"),
        media(LAMP, "audio", "audio/ogg"),
        media(LAMP, "audio", "audio/ogg"),
        media(LAMP_PICTURE, "audio", "audio/mpeg")
    ]);
    let shown = strip(&row(json!({"media_refs": refs})));
    assert_eq!(
        shown.voice.as_ref().map(|note| note.r#ref.as_str()),
        Some(NOTE)
    );
    let tiles: Vec<(&str, TileKind, &str)> = shown
        .tiles
        .iter()
        .map(|tile| (tile.media.r#ref.as_str(), tile.kind, tile.name.as_str()))
        .collect();
    assert_eq!(
        tiles,
        [
            (LAMP, TileKind::File, "Voice note"),
            (LAMP_PICTURE, TileKind::File, "Sound")
        ]
    );
    assert_eq!(shown.tiles[0].detail, "48.2 kB", "the name says what it is");
    assert_eq!(shown.tiles[0].spoken, "Voice note");
}

/// A voice note that cannot play is a file tile like any other, first in the lane where the note
/// stood; one that plays is its chip, and the lane is the other tiles alone.
#[test]
fn a_voice_note_that_cannot_play_leads_the_lane_as_a_file_tile() {
    let refs = json!([
        media(LAMP, "document", "application/pdf"),
        media(NOTE, "audio", "audio/ogg"),
        media(LAMP_PICTURE, "image", "image/jpeg")
    ]);
    let shown = strip(&row(json!({"media_refs": refs})));
    let unplayable = lane(&shown, false);
    let named: Vec<(&str, TileKind, &str)> = unplayable
        .iter()
        .map(|tile| (tile.media.r#ref.as_str(), tile.kind, tile.name.as_str()))
        .collect();
    assert_eq!(
        named,
        [
            (NOTE, TileKind::File, "Voice note"),
            (LAMP_PICTURE, TileKind::Picture, "Picture"),
            (LAMP, TileKind::File, "PDF")
        ]
    );
    assert_eq!(unplayable[0].detail, "48.2 kB");
    assert_eq!(unplayable[0], tile(shown.voice.as_ref().expect("a note")));
    assert_eq!(
        lane(&shown, true),
        shown.tiles,
        "a note that plays is no tile"
    );
}

/// A row with no voice note shows its tiles whether or not a note could play.
#[test]
fn a_lane_without_a_voice_note_is_its_tiles() {
    let refs = json!([media(LAMP_PICTURE, "image", "image/jpeg")]);
    let shown = strip(&row(json!({"media_refs": refs})));
    assert_eq!(lane(&shown, false), shown.tiles);
    assert_eq!(lane(&shown, true), shown.tiles);
}

/// A tile with no name of its own is named by its kind, so its detail is its size alone; a named
/// one says its kind beside its size.
#[test]
fn a_tile_named_by_its_kind_says_only_its_size() {
    let mut unnamed = media(LAMP, "image", "image/png");
    unnamed["filename"] = json!("   ");
    let shown = strip(&row(json!({"media_refs": [unnamed]})));
    assert_eq!(shown.tiles[0].name, "Picture");
    assert_eq!(shown.tiles[0].detail, "48.2 kB");
    let mut named = media(LAMP, "image", "image/png");
    named["filename"] = json!("shelf.png");
    let shown = strip(&row(json!({"media_refs": [named]})));
    assert_eq!(shown.tiles[0].detail, "Picture · 48.2 kB");
}

/// Save As starts from a bare name, never a path the phone wrote: the last segment, without
/// control characters, and the kind word where nothing is left but dots.
#[test]
fn a_picture_is_saved_under_its_bare_name() {
    let named = |filename: &str| {
        let mut picture = media(LAMP, "image", "image/jpeg");
        picture["filename"] = json!(filename);
        let row = row(json!({"media_refs": [picture]}));
        save_name(&strip(&row).tiles[0])
    };
    assert_eq!(named("lamp.jpg"), "lamp.jpg");
    assert_eq!(named("photos/lamp.jpg"), "lamp.jpg");
    assert_eq!(named("/var/mobile/photos/lamp.jpg"), "lamp.jpg");
    assert_eq!(named("photos/"), "photos");
    assert_eq!(named("la\u{7}mp\t.jpg\u{1b}"), "lamp.jpg");
    assert_eq!(named("../.."), "Picture");
    assert_eq!(named("."), "Picture");
    assert_eq!(named("/"), "Picture");
    let unnamed = tile(&serde_json::from_value(media(LAMP, "image", "image/jpeg")).expect("a ref"));
    assert_eq!(save_name(&unnamed), "Picture");
}

/// A card opens the address the text wrote, else the one the page answered from, and only a
/// web address: a preview can never open another app.
#[test]
fn a_link_card_opens_a_web_address_once() {
    let opens = |fields: Value| {
        link_cards(&[preview(fields)], &[])
            .first()
            .map(|card| card.opens.clone())
    };
    assert_eq!(
        opens(json!({"link": "https://a.test/written", "url": "https://a.test/read"})).as_deref(),
        Some("https://a.test/written")
    );
    assert_eq!(opens(json!({})).as_deref(), Some("https://a.test/x"));
    for refused in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,hi",
        "http://",
    ] {
        assert_eq!(
            opens(json!({"link": refused, "url": refused})),
            None,
            "{refused}"
        );
    }
    let twice = [
        preview(json!({"url": "https://Shop.TEST/lamp"})),
        preview(json!({"url": "HTTPS://shop.test/lamp", "title": "Again"})),
        preview(json!({"url": "https://shop.test/Lamp"})),
    ];
    let cards = link_cards(&twice, &[]);
    let kept: Vec<&str> = cards.iter().map(|card| card.opens.as_str()).collect();
    assert_eq!(
        kept,
        ["https://Shop.TEST/lamp", "https://shop.test/Lamp"],
        "scheme and host in any case are one destination; the path is as written"
    );
    let many: Vec<LinkPreview> = (0..6)
        .map(|n| preview(json!({"url": format!("https://a.test/{n}")})))
        .collect();
    assert_eq!(link_cards(&many, &[]).len(), 4);
}

/// A card says what the preview says, or the address where the preview is blank.
#[test]
fn a_link_card_falls_back_to_its_address() {
    let card = |fields: Value| link_cards(&[preview(fields)], &[]).remove(0);
    let blank = card(json!({"url": "https://www.example.org/a", "site": " ", "title": ""}));
    assert_eq!(blank.title, "www.example.org");
    assert_eq!(blank.site, "example.org");
    assert_eq!(blank.initial, "E");
    assert_eq!(blank.spoken, "www.example.org, Link");
    let marked = card(json!({"site": "«42 Café»", "image_ref": "not-a-ref"}));
    assert_eq!(marked.initial, "4");
    assert_eq!(marked.image_ref, None);
    let symbols = card(json!({"site": "—"}));
    assert_eq!(symbols.initial, "");
}

/// A place under the reply stands for its own page and website: a preview of either is the
/// place's card's, not a second card (Mac fbd2880).
#[test]
fn a_page_a_place_stands_for_is_its_cards() {
    let blocks = places(
        "https://tequilamama.com/",
        "https://provider.example/place/1",
    );
    for covered in [
        "https://tequilamama.com/",
        "https://tequilamama.com",
        "https://www.tequilamama.com/menu",
        "https://TequilaMama.com/menu/#drinks",
        "https://provider.example/place/1/",
        "https://provider.example/place/1#top",
    ] {
        assert!(opened_by(covered, &blocks), "{covered}");
    }
    for apart in [
        "https://provider.example/place/2",
        "https://provider.example/",
        "https://other.test/",
        "https://tequilamama.com.evil.test/",
    ] {
        assert!(!opened_by(apart, &blocks), "{apart}");
    }
    let shared = places("https://example.social/tequilamama", "https://p.test/1");
    assert!(opened_by(
        "https://example.social/tequilamama/menu",
        &shared
    ));
    assert!(!opened_by("https://example.social/other", &shared));
    assert!(!opened_by("https://example.social/tequilamamas", &shared));
    assert!(!opened_by("https://tequilamama.com/", &[]));
    let menu = [preview(json!({"url": "https://www.tequilamama.com/menu"}))];
    assert!(link_cards(&menu, &blocks).is_empty());
}

/// A line that only names what the strip shows goes; one that says anything more stays.
#[test]
fn a_line_that_only_names_a_tile_or_a_card_goes() {
    let shown = ["https://a.test/x".to_owned(), "lamp.jpg".to_owned()];
    for dropped in [
        "https://a.test/x",
        "HTTPS://A.test/x",
        "<https://a.test/x>",
        "`https://a.test/x`",
        "- https://a.test/x",
        "  * https://a.test/x",
        "• https://a.test/x",
        "1. https://a.test/x.",
        "2) https://a.test/x!",
        "[https://a.test/x](https://a.test/x)",
        "[lamp.jpg](lamp.jpg)",
        "lamp.jpg",
    ] {
        assert_eq!(
            without_shown_lines(&format!("Look\n{dropped}"), &shown),
            "Look",
            "{dropped}"
        );
    }
    for kept in [
        "[lamp.jpg](https://x.test/lamp.jpg)",
        "[the lamp](https://a.test/x)",
        "See https://a.test/x for more",
        "https://a.test/y",
        "lamp.jpg is here",
    ] {
        let text = format!("Look\n{kept}");
        assert_eq!(without_shown_lines(&text, &shown), text, "{kept}");
    }
    let fenced = "```\nhttps://a.test/x\n```";
    assert_eq!(without_shown_lines(fenced, &shown), fenced);
}

/// An address's own closing bracket is part of it, as a link reader takes one that pairs; only
/// the marks round it are set aside (Mac `isReading`).
#[test]
fn a_line_naming_an_address_that_ends_in_a_bracket_goes() {
    let shown = ["https://en.test/Foo_(bar)".to_owned()];
    for dropped in [
        "https://en.test/Foo_(bar)",
        "(https://en.test/Foo_(bar)).",
        "<https://en.test/Foo_(bar)>",
        "- https://en.test/Foo_(bar)!",
    ] {
        assert_eq!(
            without_shown_lines(&format!("Look\n{dropped}"), &shown),
            "Look",
            "{dropped}"
        );
    }
    let other = "Look\nhttps://en.test/Foo_(baz)";
    assert_eq!(without_shown_lines(other, &shown), other);
    let plain = ["https://a.test/x".to_owned()];
    assert_eq!(
        without_shown_lines("Look\n(https://a.test/x)", &plain),
        "Look"
    );
}

/// A tile's name is its file name trimmed, and so is the name a line is matched against.
#[test]
fn a_line_naming_a_file_whose_name_has_spaces_round_it_goes() {
    let refs = json!([{"ref": LAMP, "kind": "image", "mime": "image/jpeg", "size_bytes": 1,
        "filename": " lamp.jpg  "}]);
    let named = row(json!({"text": "Here:\nlamp.jpg", "media_refs": refs}));
    assert_eq!(words(&named, &strip(&named)), "Here:");
}

#[test]
fn a_dropped_line_closes_up_its_blank_lines_and_the_rest_is_untouched() {
    let shown = ["https://a.test/x".to_owned()];
    assert_eq!(
        without_shown_lines("Look:\n\nhttps://a.test/x\n\nThanks", &shown),
        "Look:\n\nThanks"
    );
    assert_eq!(
        without_shown_lines("\nhttps://a.test/x\n\nThanks\n\n", &shown),
        "Thanks"
    );
    assert_eq!(without_shown_lines("https://a.test/x\n", &shown), "");
    let untouched = "  Hello  \n\n\nThere\n";
    assert_eq!(without_shown_lines(untouched, &shown), untouched);
}

/// The words of a row lose the lines its strip shows, and a row of files alone says nothing.
#[test]
fn a_row_draws_its_text_less_what_its_strip_shows() {
    let refs = json!([{"ref": LAMP, "kind": "image", "mime": "image/jpeg", "size_bytes": 1,
        "filename": "lamp.jpg"}]);
    let previews = json!([{"url": "https://shop.test/lamp", "site": "Shop", "title": "Lamp"}]);
    let text = "The lamp:\n\nlamp.jpg\nhttps://shop.test/lamp";
    let both = row(json!({"text": text, "media_refs": refs, "link_previews": previews}));
    assert_eq!(words(&both, &strip(&both)), "The lamp:");
    let alone = row(json!({"text": "lamp.jpg", "media_refs": refs}));
    assert_eq!(words(&alone, &strip(&alone)), "");
    assert!(strip(&row(json!({"text": "Hi"}))).is_empty());
}

#[test]
fn a_size_reads_as_the_desktop_writes_it() {
    for (bytes, written) in [
        (0, "0 bytes"),
        (1, "1 byte"),
        (999, "999 bytes"),
        (1_000, "1 kB"),
        (9_999, "10 kB"),
        (48_213, "48.2 kB"),
        (182_044, "182 kB"),
        (999_950, "1 MB"),
        (1_073_741_824, "1.07 GB"),
        (u64::MAX, "18.4 EB"),
    ] {
        assert_eq!(format_size(bytes), written, "{bytes}");
    }
}

/// At most two pictures decode at once and 64 wait; past that the oldest waiting gives way.
#[test]
fn decoding_admits_two_at_once_and_drops_the_oldest_waiting() {
    assert_eq!((MAX_DECODING, MAX_WAITING), (2, 64));
    let mut admission = Admission::default();
    assert_eq!(admission.ask(0), (Some(0), None));
    assert_eq!(admission.ask(1), (Some(1), None));
    for job in 2..66 {
        assert_eq!(admission.ask(job), (None, None), "{job} waits");
    }
    assert_eq!(
        admission.ask(66),
        (None, Some(2)),
        "the oldest waiting gives way"
    );
    assert_eq!(admission.done(), Some(3));
    let mut started = vec![3];
    for _ in 0..200 {
        match admission.done() {
            Some(job) => started.push(job),
            None => break,
        }
    }
    assert_eq!(started, (3..=66).collect::<Vec<_>>());
    assert_eq!(admission.done(), None);
    assert_eq!(
        admission.ask(67),
        (Some(67), None),
        "the queue is empty again"
    );
}

/// A picture is read only when its header says it fits the ceiling, whatever the numbers.
#[test]
fn a_picture_past_the_ceiling_is_never_decoded() {
    for refused in [(0, 10), (10, 0), (u32::MAX, u32::MAX), (10_001, 10_000)] {
        assert!(!within_ceiling(refused.0, refused.1), "{refused:?}");
    }
    assert!(within_ceiling(10_000, 10_000));
    assert!(within_ceiling(1, 1));
}

/// A picture is decoded no larger than it is drawn: a tile's square covered, the viewer's held,
/// both by a square, so the size holds whichever way the photo's own orientation turns it.
#[test]
fn a_picture_is_decoded_at_the_size_it_is_drawn() {
    // A phone's 12 MP photo, either way up, for a 180 by 120 tile at scale 2 (side 360).
    assert_eq!(decode_size(4032, 3024, 360, Fit::Cover), (480, 360));
    assert_eq!(decode_size(3024, 4032, 360, Fit::Cover), (360, 480));
    // The viewer's 1000 by 760 at scale 1 (side 1000).
    assert_eq!(decode_size(4032, 3024, 1000, Fit::Contain), (1000, 750));
    assert_eq!(decode_size(3024, 4032, 1000, Fit::Contain), (750, 1000));
    // Never larger than the picture, and never below a pixel.
    assert_eq!(decode_size(100, 50, 360, Fit::Cover), (100, 50));
    assert_eq!(decode_size(100, 50, 1000, Fit::Contain), (100, 50));
    assert_eq!(decode_size(10_000, 1, 360, Fit::Cover), (10_000, 1));
    assert_eq!(decode_size(10_000, 1, 80, Fit::Contain), (80, 1));
    assert_eq!(decode_size(10_000, 10_000, 80, Fit::Cover), (80, 80));
}
