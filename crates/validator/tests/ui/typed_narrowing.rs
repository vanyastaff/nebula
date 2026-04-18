//! Typed narrowing: validate_value belongs to ValueRule, not Predicate.

use nebula_validator::{foundation::FieldPath, Predicate};
use serde_json::json;

fn main() {
    let p = Predicate::Eq(FieldPath::parse("x").unwrap(), json!(1));
    // Should fail: Predicate does not implement validate_value.
    let _ = p.validate_value(&json!(1));
}
