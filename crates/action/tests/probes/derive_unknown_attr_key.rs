//! Compile-fail probe: unknown key inside `#[action(...)]` is rejected.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.unknown",
    input = serde_json::Value,
    output = serde_json::Value,
    typo_attribute = "oops"
)]
struct UnknownAttr;

fn main() {}
