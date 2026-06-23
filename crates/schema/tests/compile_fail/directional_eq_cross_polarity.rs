//! Comparing an `InputSchema` to an `OutputSchema` must not compile: polarity is
//! part of the type, so `PartialEq` only relates same-polarity schemas. An
//! input port and an output port are never "equal" by construction (ADR-0100 C15).

use nebula_schema::{InputSchema, OutputSchema, ValidSchema};

fn main() {
    let input = InputSchema::new(ValidSchema::empty());
    let output = OutputSchema::new(ValidSchema::empty());

    // Cross-polarity equality is a type error, not a runtime `false`.
    let _ = input == output;
}
