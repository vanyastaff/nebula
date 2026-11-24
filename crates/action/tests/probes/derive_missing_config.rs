//! Compile-fail probe: `#[derive(Action)]` requires `config = ...`.

use nebula_action::Action;

#[derive(Action)]
#[action(key = "bad.no_config", output = serde_json::Value)]
struct MissingConfig;

fn main() {}
