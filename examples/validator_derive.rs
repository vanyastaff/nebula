//! `#[derive(Validator)]` — attribute-driven struct validation.
//!
//! Run with: `cargo run -p nebula-examples --bin validator_derive`

use nebula_validator::{Validator, foundation::Validate};

/// A user registration payload.
///
/// Every `#[validate(...)]` clause is checked when `.validate(&user)` or
/// `user.validate_fields()` is called; errors are accumulated by field
/// rather than short-circuiting.
#[derive(Debug, Validator)]
#[validator(message = "user registration failed")]
struct User {
    /// Must be 3–32 alphanumeric characters.
    #[validate(min_length = 3, max_length = 32, alphanumeric)]
    username: String,

    /// Must be a syntactically valid email.
    #[validate(email)]
    email: String,

    /// Adults only.
    #[validate(required, range(min = 18, max = 120))]
    age: Option<u8>,

    /// At least one tag, each 2–20 chars, lowercase letters only.
    /// The regex is compiled once per process via `LazyLock` and verified
    /// at macro-time — typos become compile errors.
    #[validate(
        min_size = 1,
        max_size = 10,
        each(min_length = 2, max_length = 20, regex = "^[a-z]+$")
    )]
    tags: Vec<String>,
}

fn main() {
    let valid = User {
        username: "alice42".into(),
        email: "alice@example.com".into(),
        age: Some(30),
        tags: vec!["rust".into(), "systems".into()],
    };
    assert!(valid.validate(&valid).is_ok());
    println!("✓ valid user passes");

    let invalid = User {
        username: "a!".into(),    // too short and non-alphanumeric
        email: "nope".into(),     // not an email
        age: None,                // missing required field
        tags: vec!["BAD".into()], // uppercase tags rejected by regex
    };

    match invalid.validate_fields() {
        Ok(()) => unreachable!(),
        Err(errors) => {
            println!("✗ invalid user: {} error(s)", errors.len());
            for e in errors.errors() {
                println!(
                    "    [{}] {}: {}",
                    e.field.as_deref().unwrap_or("-"),
                    e.code,
                    e.message
                );
            }
        },
    }
}
