//! Compile-fail probe: the no-effect declaration is an explicit flag.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.effect-value",
    input = serde_json::Value,
    output = serde_json::Value,
    no_external_effects = true
)]
struct EffectValue;

fn main() {}
