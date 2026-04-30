//! Compile-fail probe: `#[derive(Action)]` requires `input = ...`.

use nebula_action::Action;

#[derive(Action)]
#[action(key = "bad.no_input", output = serde_json::Value)]
struct MissingInput;

fn main() {}
