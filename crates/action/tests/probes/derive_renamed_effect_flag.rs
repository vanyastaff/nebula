//! Compile-fail probe: the pre-0.22.0 spelling of `#[action(read_only)]`
//! is refused with a hint naming the new flag.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.renamed-effect-flag",
    input = serde_json::Value,
    output = serde_json::Value,
    no_external_effects
)]
struct RenamedFlag;

fn main() {}
