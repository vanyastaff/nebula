//! Tests for the Action derive macro - error cases.

use nebula_macros::Action;

// Missing required `key` attribute
#[derive(Action)]
#[action(
    name = "Invalid Action"
)]
pub struct MissingKeyAction;

// Missing required `name` attribute
#[derive(Action)]
#[action(
    key = "test.missing_name"
)]
pub struct MissingNameAction;

// Struct with fields — not allowed
#[derive(Action)]
#[action(key = "test.with_fields", name = "With Fields")]
pub struct ActionWithFields {
    config: String,
}

fn main() {}
