//! A reply's place cards, read off its places block as the macOS app's `ChatPlaceFacts` reads
//! them, without the map: what each card says, where its website is, and what Open in Maps
//! launches.

use fermix_client::chat_wire::protocol::{decode_line, Block, Place, ServerEvent};
use fermix_client::places::{card, cards, maps_launch, MapsLaunch, MAX_PLACES};
use serde_json::{json, Value};

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");

/// The blocks of the vendored `text_done` for `turn`.
fn fixture_blocks(turn: &str) -> Vec<Block> {
    for line in SERVER.lines() {
        match decode_line(line.as_bytes()) {
            Ok(ServerEvent::TextDone(reply)) if reply.turn_id == turn => return reply.blocks,
            _ => {}
        }
    }
    panic!("no text_done for {turn} in the fixtures");
}

/// A place with only what the contract requires, and `extra` over it.
fn place(extra: Value) -> Place {
    let mut value = json!({"name": "Spot", "location": {"latitude": 1.5, "longitude": -2.25},
        "source_url": "https://spot.example/"});
    for (key, field) in extra.as_object().expect("an object") {
        value[key] = field.clone();
    }
    serde_json::from_value(value).expect("a place")
}

#[test]
fn a_search_place_reads_its_rating_price_kind_hours_and_distance() {
    let shown = cards(&fixture_blocks("turn-mac-6"));
    assert_eq!(shown.len(), 2);
    let coffee = &shown[0];
    assert_eq!(coffee.name, "Example Coffee");
    assert_eq!(
        coffee.lines,
        [
            "4.6 (321) · $$ · Coffee shop",
            "Thursday 07:00-19:00 · 0.4 km"
        ]
    );
    assert_eq!(coffee.website.as_deref(), Some("https://example.coffee/"));
    assert_eq!(coffee.address.as_deref(), Some("12 Example St, New York"));
    let quiet = &shown[1];
    assert_eq!(quiet.name, "Quiet Grounds");
    assert_eq!(quiet.lines, ["Cafe"]);
    assert_eq!(
        quiet.website.as_deref(),
        Some("https://provider.example/place/2")
    );
    assert_eq!(quiet.address, None);
}

/// An Apple Maps place has no rating, price or hours: its kind, over its street.
#[test]
fn a_place_without_hours_says_its_street() {
    let shown = cards(&fixture_blocks("turn-mac-7"));
    assert_eq!(shown[0].lines, ["Restaurant", "100 Example Ave"]);
    assert_eq!(
        shown[0].website.as_deref(),
        Some("https://taqueria.example/")
    );
    assert_eq!(shown[1].lines, ["Nightlife", "200 Example Rd"]);
    assert_eq!(
        shown[1].website.as_deref(),
        Some("https://maps.apple.com/place?place-id=I1A2B3C4D5E6F7081"),
        "the source's page is a web address too, as on the Mac"
    );
}

#[test]
fn a_rating_shows_its_value_and_count_and_a_count_alone_says_nothing() {
    let lines = |rating: Value| card(&place(json!({"rating": rating}))).lines;
    assert_eq!(lines(json!({"value": 4.0})), ["4.0"]);
    assert_eq!(lines(json!({"value": 4.25, "count": 321.0})), ["4.2 (321)"]);
    assert_eq!(lines(json!({"value": 3.9, "count": 1234})), ["3.9 (1,234)"]);
    assert_eq!(
        lines(json!({"value": 5, "best": 5, "count": 1_234_567})),
        ["5.0 (1,234,567)"]
    );
    assert!(lines(json!({"count": 321, "best": 5})).is_empty());
}

#[test]
fn a_distance_shows_at_most_two_decimals_and_its_units_as_given() {
    let lines =
        |value: f64| card(&place(json!({"distance": {"value": value, "units": "mi"}}))).lines;
    assert_eq!(lines(0.4), ["0.4 mi"]);
    assert_eq!(lines(1.25), ["1.25 mi"]);
    assert_eq!(lines(2.0), ["2 mi"]);
    assert_eq!(lines(1.234), ["1.23 mi"]);
    assert_eq!(lines(1500.5), ["1,500.5 mi"]);
}

#[test]
fn a_kind_is_in_sentence_case_and_hours_beat_the_street_as_given() {
    let kind = |category: &str| card(&place(json!({"categories": [category, "other"]}))).lines;
    assert_eq!(kind("coffee_shop"), ["Coffee shop"]);
    assert_eq!(kind("CAFE"), ["Cafe"]);
    assert_eq!(kind("élan_bar"), ["Élan bar"]);
    let visit = |extra: Value| card(&place(extra)).lines;
    assert_eq!(
        visit(json!({"address": "12 Example St, New York"})),
        ["12 Example St"]
    );
    assert_eq!(
        visit(json!({"address": "  Main Square  "})),
        ["Main Square"]
    );
    assert!(visit(json!({"address": ", Suwanee"})).is_empty());
    assert_eq!(
        visit(json!({"address": "1 A St, B", "opening_hours": {"current_day": "Open 24 hours"}})),
        ["Open 24 hours"]
    );
}

/// A part with only spaces in it says nothing: no line of spaces is drawn, and a screen reader
/// hears no empty part.
#[test]
fn a_blank_part_is_left_out_and_every_part_is_trimmed() {
    let spaced = card(&place(json!({
        "price_range": "  ",
        "categories": ["_"],
        "opening_hours": {"current_day": " 07:00-19:00 "},
        "distance": {"value": 0.4, "units": " km "}
    })));
    assert_eq!(spaced.lines, ["07:00-19:00 · 0.4 km"]);
    assert_eq!(spaced.spoken, "Spot, 07:00-19:00, 0.4 km");
    let blank = card(&place(json!({
        "price_range": "",
        "categories": ["   "],
        "distance": {"value": 2.0, "units": ""}
    })));
    assert_eq!(blank.lines, ["2"]);
    assert_eq!(blank.spoken, "Spot, 2");
    let nothing = card(&place(json!({"price_range": " ", "categories": ["__"]})));
    assert!(nothing.lines.is_empty());
    assert_eq!(nothing.spoken, "Spot");
    assert_eq!(
        card(&place(json!({"categories": ["_coffee_shop_"]}))).lines,
        ["Coffee shop"]
    );
}

/// Open website hands its address to the desktop: only a web address may go, so no record can
/// open another app (Mac 7066656).
#[test]
fn a_website_is_the_first_web_address_of_the_place_and_its_source() {
    let website = |canonical: &str, source: &str| {
        card(&place(
            json!({"canonical_url": canonical, "source_url": source}),
        ))
        .website
    };
    for refused in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "maps://?q=x",
        "geo:1,2",
        "ftp://x.example/",
        "https://",
        " https://x.example/",
        "https://a b.example/",
    ] {
        assert_eq!(website(refused, refused), None, "{refused}");
    }
    assert_eq!(
        website("HTTPS://Example.com/x", "https://source.example/").as_deref(),
        Some("HTTPS://Example.com/x")
    );
    assert_eq!(
        website("http://x.example", "https://source.example/").as_deref(),
        Some("http://x.example")
    );
    assert_eq!(
        website("maps://x", "https://source.example/").as_deref(),
        Some("https://source.example/")
    );
}

#[test]
fn maps_opens_the_place_by_its_coordinates_and_its_encoded_name() {
    let coffee = &cards(&fixture_blocks("turn-mac-6"))[0];
    assert_eq!(
        coffee.maps,
        "geo:40.7233,-74.003?q=40.7233,-74.003(Example%20Coffee)"
    );
    assert_eq!(
        coffee.web_map,
        "https://www.openstreetmap.org/?mlat=40.7233&mlon=-74.003#map=17/40.7233/-74.003"
    );
    let odd = card(&place(json!({"name": "A (b), c&d #e? Café Zoë",
        "location": {"latitude": 0.0, "longitude": -0.5}})));
    assert_eq!(
        odd.maps,
        "geo:0,-0.5?q=0,-0.5(A%20%28b%29%2C%20c%26d%20%23e%3F%20Caf%C3%A9%20Zo%C3%AB)"
    );
    let tiny = card(&place(
        json!({"location": {"latitude": 1e-7, "longitude": 179.9999999}}),
    ));
    assert_eq!(
        tiny.maps,
        "geo:0.0000001,179.9999999?q=0.0000001,179.9999999(Spot)"
    );
}

/// With an app for `geo:` it opens there, and the web map only when that launch fails; with
/// none, the web map at once, so nobody meets a chooser with no apps in it.
#[test]
fn open_in_maps_tries_the_maps_app_only_where_one_is() {
    let spot = card(&place(json!({})));
    assert_eq!(
        maps_launch(&spot, true),
        MapsLaunch {
            first: spot.maps.clone(),
            fallback: Some(spot.web_map.clone()),
        }
    );
    assert_eq!(
        maps_launch(&spot, false),
        MapsLaunch {
            first: spot.web_map.clone(),
            fallback: None,
        }
    );
}

#[test]
fn a_screen_reader_hears_the_name_and_every_part() {
    let coffee = &cards(&fixture_blocks("turn-mac-6"))[0];
    assert_eq!(
        coffee.spoken,
        "Example Coffee, 4.6 (321), $$, Coffee shop, Thursday 07:00-19:00, 0.4 km"
    );
    assert_eq!(card(&place(json!({}))).spoken, "Spot");
}

#[test]
fn a_block_shows_at_most_ten_places_and_an_empty_one_none() {
    let many: Vec<Place> = (0..12)
        .map(|n| place(json!({"name": format!("Place {n}")})))
        .collect();
    let shown = cards(&[Block::Places(many)]);
    assert_eq!(shown.len(), MAX_PLACES);
    assert_eq!(shown[9].name, "Place 9");
    assert!(cards(&[Block::Places(Vec::new())]).is_empty());
    assert!(cards(&[]).is_empty());
    assert!(card(&place(json!({}))).lines.is_empty());
}

#[test]
fn a_nul_in_a_place_reaches_its_card_replaced() {
    let line = json!({"type": "text_done", "turn_id": "t", "server_seq": 1, "text": "x",
        "blocks": [{"type": "places", "places": [{"name": "Caf\u{0}e",
        "location": {"latitude": 1, "longitude": 2}, "source_url": "https://x.example/",
        "opening_hours": {"current_day": "Mon\u{0}"}}]}]});
    let Ok(ServerEvent::TextDone(reply)) = decode_line(line.to_string().as_bytes()) else {
        panic!("a reply");
    };
    let shown = cards(&reply.blocks);
    assert_eq!(shown[0].name, "Caf\u{FFFD}e");
    assert_eq!(shown[0].lines, ["Mon\u{FFFD}"]);
}
