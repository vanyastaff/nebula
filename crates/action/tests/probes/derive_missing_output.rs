//! Compile-fail probe: `#[derive(Action)]` requires `output = ...`.

use nebula_action::Action;

#[derive(Action)]
#[action(key = "bad.no_output", config = serde_json::Value)]
struct MissingOutput;

fn main() {}
