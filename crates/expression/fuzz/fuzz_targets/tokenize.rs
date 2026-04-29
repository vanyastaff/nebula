#![no_main]
//! Fuzz target: `lexer::Lexer::tokenize`.
//!
//! Exercises the lexer in isolation — useful when a parser-level fuzz
//! crash needs to be split between "lexer ate it" and "parser ate it".
//! The lexer is exposed as `#[doc(hidden) pub mod lexer]` so this still
//! goes through the public crate boundary.

use libfuzzer_sys::fuzz_target;
use nebula_expression::lexer::Lexer;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let mut lex = Lexer::new(s);
        let _ = lex.tokenize();
    }
});
