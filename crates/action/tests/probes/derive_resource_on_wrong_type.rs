//! Compile-fail probe: `#[resource]` on a non-`ResourceGuard` field type.
//!
//! Variant A requires resource-marked fields to be `ResourceGuard<R>`
//! (optionally wrapped in `Option<...>` and/or `Lazy<...>`).

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
