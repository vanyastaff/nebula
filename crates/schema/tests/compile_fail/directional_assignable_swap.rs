//! Swapping producer and consumer in `explain_assignable` must not compile:
//! the first argument is the producer's `OutputSchema`, the second the
//! consumer's `InputSchema` (ADR-0100 C15 — direction enforced by the types).

use nebula_schema::{InputSchema, OutputSchema, ValidSchema, explain_assignable};

fn main() {
    let input = InputSchema::new(ValidSchema::empty());
    let output = OutputSchema::new(ValidSchema::empty());

    // Correct direction would be `explain_assignable(&output, &input)`.
    // Transposing them is a type error, not a silent logic bug.
    let _ = explain_assignable(&input, &output);
}
