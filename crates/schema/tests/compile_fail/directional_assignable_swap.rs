//! Swapping producer and consumer in `is_assignable_schema` must not compile:
//! the first argument is the producer's `OutputSchema`, the second the
//! consumer's `InputSchema` (ADR-0100 C15 — direction enforced by the types).

use nebula_schema::{InputSchema, OutputSchema, ValidSchema, is_assignable_schema};

fn main() {
    let input = InputSchema::new(ValidSchema::empty());
    let output = OutputSchema::new(ValidSchema::empty());

    // Correct direction would be `is_assignable_schema(&output, &input)`.
    // Transposing them is a type error, not a silent logic bug.
    let _ = is_assignable_schema(&input, &output);
}
