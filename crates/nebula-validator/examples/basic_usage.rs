//! Basic usage example for nebula-validator

use nebula_validator::core::Validate;
use nebula_validator::validators::string::min_length;

fn main() {
    // Create a simple string validator
    let validator = min_length(5);

    // Valid input
    match validator.validate("hello") {
        Ok(_) => println!("✓ 'hello' is valid (length >= 5)"),
        Err(e) => println!("✗ Error: {}", e),
    }

    // Invalid input
    match validator.validate("hi") {
        Ok(_) => println!("✓ 'hi' is valid"),
        Err(e) => println!("✗ 'hi' is invalid: {}", e),
    }

    println!("\nnebula-validator is working correctly!");
}
