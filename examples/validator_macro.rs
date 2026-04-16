//! `validator!` — declarative macro for writing custom validators.
//!
//! Run with: `cargo run -p nebula-examples --bin validator_macro`

use nebula_validator::{
    foundation::{Validate, ValidateExt, ValidationError},
    validator,
};

// Unit validator (zero-sized): no state, no configuration.
validator! {
    pub NotBlank for str;
    rule(input) { !input.trim().is_empty() }
    error(input) { ValidationError::new("not_blank", "must not be blank or whitespace-only") }
    fn not_blank();
}

// Struct validator with a parameter and an auto-generated constructor.
validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub HasMaxWords { max: usize } for str;
    rule(self, input) {
        input.split_whitespace().count() <= self.max
    }
    error(self, input) {
        ValidationError::new("max_words", format!("must have at most {} words", self.max))
            .with_param("max", self.max.to_string())
            .with_param("actual", input.split_whitespace().count().to_string())
    }
    fn has_max_words(max: usize);
}

// Fallible constructor — configuration errors surface at construction time.
validator! {
    pub BetweenChars { lo: usize, hi: usize } for str;
    rule(self, input) {
        let len = input.chars().count();
        len >= self.lo && len <= self.hi
    }
    error(self, input) {
        ValidationError::new(
            "between_chars",
            format!("must be between {} and {} chars", self.lo, self.hi),
        )
    }
    new(lo: usize, hi: usize) -> ValidationError {
        if lo > hi {
            return Err(ValidationError::new(
                "invalid_config",
                format!("lo ({lo}) must be <= hi ({hi})"),
            ));
        }
        Ok(Self { lo, hi })
    }
    fn between_chars(lo: usize, hi: usize) -> ValidationError;
}

fn main() {
    // Custom validators compose with the built-ins.
    let title = not_blank().and(has_max_words(8));

    assert!(title.validate("A short blog title").is_ok());
    assert!(title.validate("   ").is_err());
    assert!(
        title
            .validate("this title has way way way way way way too many words")
            .is_err()
    );

    // Fallible constructor returns a Result.
    let valid = between_chars(5, 20).expect("lo <= hi");
    assert!(valid.validate("hello world").is_ok());
    assert!(valid.validate("ab").is_err());

    let invalid = between_chars(20, 5);
    assert!(invalid.is_err());
    println!("✓ validator! macro examples all pass");
}
