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

// Invalid action type
#[derive(Action)]
#[action(
    key = "test.invalid_type",
    name = "Invalid Type",
    action_type = "unknown"
)]
pub struct InvalidTypeAction;

// Invalid isolation level
#[derive(Action)]
#[action(
    key = "test.invalid_isolation",
    name = "Invalid Isolation",
    isolation = "invalid"
)]
pub struct InvalidIsolationAction;

fn main() {}
