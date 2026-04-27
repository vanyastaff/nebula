//! Smoke tests for ActionMetadata::max_concurrent (Q8 F9).

use std::num::NonZeroU32;

use nebula_action::ActionMetadata;
use nebula_core::ActionKey;

fn meta() -> ActionMetadata {
    ActionMetadata::new(
        ActionKey::new("test.maxc").expect("valid key"),
        "test",
        "max_concurrent smoke",
    )
}

#[test]
fn default_is_none() {
    let m = meta();
    assert_eq!(m.max_concurrent, None);
}

#[test]
fn round_trips_through_json() {
    let mut m = meta();
    m.max_concurrent = Some(NonZeroU32::new(4).unwrap());
    let s = serde_json::to_string(&m).expect("serialize");
    let back: ActionMetadata = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.max_concurrent, Some(NonZeroU32::new(4).unwrap()));
}

#[test]
fn omits_when_none() {
    let m = meta();
    let s = serde_json::to_string(&m).expect("serialize");
    // Deserialize to verify the field isn't present as a JSON key
    let v: serde_json::Value = serde_json::from_str(&s).expect("parse JSON");
    assert!(
        v.get("max_concurrent").is_none(),
        "serialized form should omit None field, got: {}",
        s
    );
}

#[test]
fn deserializes_when_field_absent() {
    // Older metadata (saved before F9 landed) MUST still deserialize.
    // Build a modern metadata, serialize it, then remove max_concurrent field
    // to simulate pre-F9 payloads.
    let legacy = meta();
    let mut as_value: serde_json::Value = serde_json::to_value(&legacy).expect("serialize");
    // Simulate pre-field payload by removing the key we just added.
    as_value.as_object_mut().unwrap().remove("max_concurrent");
    let json_string = serde_json::to_string(&as_value).expect("to string");
    let _: ActionMetadata =
        serde_json::from_str(&json_string).expect("backwards-compat deserialize");
}
