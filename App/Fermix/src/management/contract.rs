//! The vendored contract, read at compile time.
//!
//! The client derives every bound it enforces from the same schema the daemon
//! is tested against: the version window, the per-method minimums and the
//! transport limits. Nothing here is a second copy of a number the schema
//! publishes, which is the whole reason the tree is vendored rather than
//! hand-transcribed.

use std::sync::OnceLock;

use serde::Deserialize;

/// The vendored schema, compiled into the binary.
pub const SCHEMA_JSON: &str = include_str!("../../contracts/management/protocol.schema.json");

/// The inclusive range of protocol versions a peer accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct VersionRange {
    pub min: u32,
    pub max: u32,
}

impl VersionRange {
    /// Whether this range carries a version.
    pub fn contains(&self, version: u32) -> bool {
        version >= self.min && version <= self.max
    }
}

/// The transport and envelope bounds the schema publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Limits {
    pub max_frame_bytes: usize,
    pub max_params_bytes: usize,
    pub max_result_bytes: usize,
    pub max_error_details_bytes: usize,
    pub max_json_depth: u32,
    pub max_json_collection_items: u32,
}

#[derive(Deserialize)]
struct Schema {
    #[serde(rename = "x-protocol-version")]
    protocol_version: u32,
    #[serde(rename = "x-supported-version-range")]
    supported_version_range: VersionRange,
    #[serde(rename = "x-limits")]
    limits: Limits,
    #[serde(rename = "x-method-minimum-versions")]
    method_minimum_versions: std::collections::BTreeMap<String, u32>,
}

fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(SCHEMA_JSON).expect("the vendored protocol schema does not parse")
    })
}

/// The protocol version this contract publishes as current.
pub fn protocol_version() -> u32 {
    schema().protocol_version
}

/// The window of versions the contract declares. There is exactly one such key
/// in the schema, and `tests/contract.rs` proves there is no second one: a
/// second key drifts from the first and the drift is invisible until a
/// negotiation fails in the field.
pub fn supported_range() -> VersionRange {
    schema().supported_version_range
}

/// The published bounds.
pub fn limits() -> Limits {
    schema().limits
}

/// The minimum protocol version one method needs, or `None` for a method this
/// contract does not publish.
pub fn method_minimum(method: &str) -> Option<u32> {
    schema().method_minimum_versions.get(method).copied()
}

/// Every method the contract publishes, with its minimum.
pub fn methods() -> impl Iterator<Item = (&'static str, u32)> {
    schema()
        .method_minimum_versions
        .iter()
        .map(|(method, minimum)| (method.as_str(), *minimum))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_read_from_the_schema() {
        let range = supported_range();
        assert!(range.min >= 1);
        assert!(range.max >= range.min);
        assert!(range.contains(protocol_version()));
    }

    #[test]
    fn every_published_method_carries_a_minimum_inside_the_window() {
        let range = supported_range();
        let mut count = 0;
        for (method, minimum) in methods() {
            assert!(
                range.contains(minimum),
                "{method} declares a minimum outside the window"
            );
            count += 1;
        }
        assert!(count > 0, "the contract publishes no methods");
    }

    #[test]
    fn hello_is_always_reachable_at_the_floor() {
        assert_eq!(method_minimum("hello"), Some(supported_range().min));
    }

    #[test]
    fn an_unpublished_method_has_no_minimum() {
        assert_eq!(method_minimum("settings.invent"), None);
    }

    #[test]
    fn the_frame_ceiling_is_the_published_one() {
        assert_eq!(limits().max_frame_bytes, 4_194_304);
    }
}
