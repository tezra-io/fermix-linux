//! Home's runtime details, read from `overview.get` (spec §1.2).

use fermix_client::management::decode_response;
use fermix_client::overview::{channels_line, duration_words, tools_count, Overview};
use serde_json::Value;

const SUCCESS: &str = include_str!("fixtures/management/success.jsonl");

fn overview_json() -> Value {
    let found = SUCCESS
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["name"] == "overview_get")
        .unwrap();
    let response = &found["response"];
    let id = response["request_id"].as_str().unwrap();
    decode_response(&serde_json::to_vec(response).unwrap(), id).unwrap()
}

fn overview() -> Overview {
    serde_json::from_value(overview_json()).unwrap()
}

#[test]
fn the_overview_decodes_what_home_shows() {
    let o = overview();
    assert_eq!(o.daemon.uptime_ms, Some(864_213));
    assert_eq!(o.agents.main.active_conversations, 0);
    assert_eq!(o.capabilities.skill, 9);
}

#[test]
fn tools_are_built_in_plus_mcp() {
    assert_eq!(tools_count(&overview()), 53);
}

#[test]
fn channels_list_the_enabled_ones_by_their_names() {
    assert_eq!(channels_line(&overview()), "Telegram");
}

#[test]
fn a_duration_reads_in_its_largest_units() {
    assert_eq!(duration_words(864_213), "14 minutes");
    assert_eq!(duration_words(61_897_905), "17 hours, 11 minutes");
    assert_eq!(duration_words(3 * 86_400_000 + 3_600_000), "3 days, 1 hour");
    assert_eq!(duration_words(20_000), "less than a minute");
}

#[test]
fn the_voice_facts_decode_and_are_optional() {
    let realtime = overview().realtime.expect("the fixture reports voice");
    assert!(realtime.enabled);
    assert_eq!(realtime.status, "ready");
    assert_eq!(realtime.socket_alive, Some(true));
    let mut bare: Value = serde_json::to_value(overview_json()).unwrap();
    bare.as_object_mut().unwrap().remove("realtime");
    let without: Overview = serde_json::from_value(bare).unwrap();
    assert!(without.realtime.is_none());
}
