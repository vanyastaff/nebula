//! Combinators example for nebula-validator

use nebula_validator::prelude::{Validate, and, max_length, min_length};

fn main() {
    // Combine validators with AND
    let username_validator = and(min_length(3), max_length(20));

    println!("Testing username validation (length 3-20):\n");

    // Valid username
    match username_validator.validate("alice") {
        Ok(()) => println!("✓ 'alice' is valid"),
        Err(e) => println!("✗ Error: {e}"),
    }

    // Too short
    match username_validator.validate("ab") {
        Ok(()) => println!("✓ 'ab' is valid"),
        Err(e) => println!("✗ 'ab' is too short: {e}"),
    }

    // Too long
    let long_name = "verylongusernamethatexceedslimit";
    match username_validator.validate(long_name) {
        Ok(()) => println!("✓ '{long_name}' is valid"),
        Err(e) => println!("✗ '{long_name}' is too long: {e}"),
    }

    println!("\nCombinators are working correctly!");
}
