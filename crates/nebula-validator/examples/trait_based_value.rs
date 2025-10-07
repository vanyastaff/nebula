//! Trait-based Value integration example

use nebula_validator::core::TypedValidator;
use nebula_validator::validators::string::{min_length, max_length};
use nebula_validator::bridge::ValueValidatorExt;
use nebula_validator::combinators::and;
use nebula_value::Value;

fn main() {
    println!("=== Trait-Based Value Integration ===\n");

    // NEW WAY (automatic with traits):
    let validator = min_length(5).for_value();

    println!("1. Simple string validation:");
    let text_value = Value::Text("hello".into());
    match validator.validate(&text_value) {
        Ok(_) => println!("   ✓ 'hello' is valid (>= 5 chars)"),
        Err(e) => println!("   ✗ Error: {}", e),
    }

    let short_text = Value::Text("hi".into());
    match validator.validate(&short_text) {
        Ok(_) => println!("   ✓ 'hi' is valid"),
        Err(e) => println!("   ✗ 'hi' is invalid: {}", e),
    }

    // Wrong type
    let number_value = Value::Integer(nebula_value::Integer::new(42));
    match validator.validate(&number_value) {
        Ok(_) => println!("   ✓ Number validated as string?!"),
        Err(e) => println!("   ✗ Type mismatch (expected): {}", e),
    }

    println!("\n2. Combining validators with .for_value():");
    let username_validator = and(min_length(3), max_length(20)).for_value();

    let valid_username = Value::Text("alice".into());
    match username_validator.validate(&valid_username) {
        Ok(_) => println!("   ✓ 'alice' is a valid username"),
        Err(e) => println!("   ✗ Error: {}", e),
    }

    let too_short = Value::Text("ab".into());
    match username_validator.validate(&too_short) {
        Ok(_) => println!("   ✓ 'ab' is valid"),
        Err(e) => println!("   ✗ 'ab' too short: {}", e),
    }

    println!("\n3. Benefits:");
    println!("   • Automatic type extraction using Extract trait");
    println!("   • No manual wrapper types needed");
    println!("   • Works with any validator via extension trait");

    println!("\n✨ Trait-based Value integration working!");
}
