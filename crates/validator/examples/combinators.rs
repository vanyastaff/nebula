//! Combinators example for nebula-validator

use nebula_validator::prelude::*;

fn main() {
    // Combine validators with AND
    let username_validator = and(min_length(3), max_length(20));

    println!("Testing username validation (length 3-20):\n");

    // Valid username
    match username_validator.validate("alice") {
        Ok(_) => println!("✓ 'alice' is valid"),
        Err(e) => println!("✗ Error: {}", e),
    }

    // Too short
    match username_validator.validate("ab") {
        Ok(_) => println!("✓ 'ab' is valid"),
        Err(e) => println!("✗ 'ab' is too short: {}", e),
    }

    // Too long
    let long_name = "verylongusernamethatexceedslimit";
    match username_validator.validate(long_name) {
        Ok(_) => println!("✓ '{}' is valid", long_name),
        Err(e) => println!("✗ '{}' is too long: {}", long_name, e),
    }

    println!("\nCombinators are working correctly!");
}
