//! Compile-fail probe: `#[resource]` on a non-`ResourceHandle` field type.
//!
//! Variant A requires resource-marked fields to be `ResourceHandle<R>`
//! (optionally wrapped in `Option<...>`).

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.resource_wrong_type",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct ResourceWrongType {
    #[resource]
    not_a_guard: String,
}

fn main() {}
