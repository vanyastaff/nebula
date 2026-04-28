#![no_main]
//! Fuzz target: `nebula_expression::Template::new`.
//!
//! Exercises the template parser (which has its own newline/`{{`/`}}`
//! state machine separate from the lexer). Same contract as
//! `parse_expression`: panic-free for any UTF-8 input.

use libfuzzer_sys::fuzz_target;
use nebula_expression::Template;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = Template::new(s.to_owned());
    }
});
