#![no_main]
//! Fuzz target: `nebula_expression::parse_expression`.
//!
//! Drives the public parse-and-syntax-check entry point with arbitrary
//! UTF-8 input. The target succeeds when the function either returns a
//! result OR returns a typed `ExpressionError` — what we are looking for
//! is panics, infinite loops, or stack overflows on hostile input.
//!
//! Run locally:
//!
//! ```bash
//! cd crates/expression/fuzz
//! cargo +nightly fuzz run parse_expression -- -max_total_time=60
//! ```

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Don't unwrap — Err is the *expected* outcome for malformed
        // input. We only care that the call returns rather than panics.
        let _ = nebula_expression::parse_expression(s);
    }
});
