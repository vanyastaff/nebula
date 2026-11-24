//! Compile-fail probe: two `#[slot]` fields with the same key are rejected
//! at macro expansion, before any field type is checked.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.dup_slot_keys",
    config = serde_json::Value,
    output = serde_json::Value,
)]
struct ConflictingSlotKeys {
    #[slot(key = "shared")]
    cred_a: u32,
    #[slot(key = "shared")]
    cred_b: u32,
}

fn main() {}
