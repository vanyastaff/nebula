//! Compile-fail probe: the pre-0.41.0 `input = ...` attribute is refused with
//! a hint naming `config`.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.renamed-input",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct RenamedInput;

fn main() {}
