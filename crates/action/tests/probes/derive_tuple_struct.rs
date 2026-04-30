//! Compile-fail probe: tuple structs are not supported by `#[derive(Action)]`.

use nebula_action::Action;

#[derive(Action)]
#[action(
    key = "bad.tuple",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct TupleAction(u32, u32);

fn main() {}
