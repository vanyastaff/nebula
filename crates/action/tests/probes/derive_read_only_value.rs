//! Compile-fail probe: the no-effect declaration is an explicit flag.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.effect-value",
    input = serde_json::Value,
    output = serde_json::Value,
    read_only = true
)]
struct EffectValue;

fn main() {}
