//! Basic validation examples demonstrating core validators
//!
//! Run with: cargo run --example basic_validation -p nebula-validator

use nebula_validator::prelude::*;
use nebula_value::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula Validator - Basic Examples ===\n");

    // Example 1: Required field validation
    {
        println!("1. Required Field Validation:");
        let validator = required();

        let valid = Value::text("hello");
        let invalid = Value::Null;

        match validator.validate(&valid, None).await {
            Ok(_) => println!("   ✓ 'hello' is valid (not null)"),
            Err(e) => println!("   ✗ Error: {}", e),
        }

        match validator.validate(&invalid, None).await {
            Ok(_) => println!("   ✓ null is valid"),
            Err(e) => println!("   ✗ null is invalid: {}", e),
        }
    }

    println!();

    // Example 2: String length validation
    {
        println!("2. String Length Validation:");
        let validator = min_length(5).and(max_length(10));

        let valid = Value::text("hello");
        let too_short = Value::text("hi");
        let too_long = Value::text("hello world!");

        println!("   Validating 'hello' (5 chars): {}",
            if validator.validate(&valid, None).await.is_ok() { "✓" } else { "✗" });
        println!("   Validating 'hi' (2 chars): {}",
            if validator.validate(&too_short, None).await.is_ok() { "✓" } else { "✗ (too short)" });
        println!("   Validating 'hello world!' (12 chars): {}",
            if validator.validate(&too_long, None).await.is_ok() { "✓" } else { "✗ (too long)" });
    }

    println!();

    // Example 3: Numeric range validation
    {
        println!("3. Numeric Range Validation:");
        let validator = range(0.0, 100.0);

        let valid = Value::float(50.0);
        let negative = Value::float(-10.0);
        let too_high = Value::float(150.0);

        println!("   Validating 50.0: {}",
            if validator.validate(&valid, None).await.is_ok() { "✓ (in range)" } else { "✗" });
        println!("   Validating -10.0: {}",
            if validator.validate(&negative, None).await.is_ok() { "✓" } else { "✗ (below min)" });
        println!("   Validating 150.0: {}",
            if validator.validate(&too_high, None).await.is_ok() { "✓" } else { "✗ (above max)" });
    }

    println!();

    // Example 4: Type validation
    {
        println!("4. Type Validation:");
        let string_validator = string();
        let number_validator = number();
        let boolean_validator = boolean();

        let text_value = Value::text("hello");
        let num_value = Value::integer(42);
        let bool_value = Value::boolean(true);

        println!("   String validator on 'hello': {}",
            if string_validator.validate(&text_value, None).await.is_ok() { "✓" } else { "✗" });
        println!("   String validator on 42: {}",
            if string_validator.validate(&num_value, None).await.is_ok() { "✓" } else { "✗ (wrong type)" });

        println!("   Number validator on 42: {}",
            if number_validator.validate(&num_value, None).await.is_ok() { "✓" } else { "✗" });
        println!("   Number validator on true: {}",
            if number_validator.validate(&bool_value, None).await.is_ok() { "✓" } else { "✗ (wrong type)" });

        println!("   Boolean validator on true: {}",
            if boolean_validator.validate(&bool_value, None).await.is_ok() { "✓" } else { "✗" });
    }

    println!();

    // Example 5: Pattern matching
    {
        println!("5. Pattern Matching:");
        let email_validator = string_contains("@".to_string())
            .and(string_contains(".".to_string()));

        let valid_email = Value::text("user@example.com");
        let invalid_email = Value::text("user-at-example");

        println!("   Validating 'user@example.com': {}",
            if email_validator.validate(&valid_email, None).await.is_ok() {
                "✓ (contains @ and .)"
            } else {
                "✗"
            });
        println!("   Validating 'user-at-example': {}",
            if email_validator.validate(&invalid_email, None).await.is_ok() {
                "✓"
            } else {
                "✗ (missing @ or .)"
            });
    }

    println!();

    // Example 6: Composition
    {
        println!("6. Complex Composition:");
        // Username: 3-20 chars, alphanumeric, lowercase
        let username_validator = string()
            .and(min_length(3))
            .and(max_length(20))
            .and(alphanumeric(false))
            .and(lowercase());

        let valid = Value::text("alice123");
        let invalid_caps = Value::text("Alice123");
        let invalid_short = Value::text("ab");
        let invalid_special = Value::text("alice@123");

        println!("   Validating 'alice123': {}",
            if username_validator.validate(&valid, None).await.is_ok() { "✓" } else { "✗" });
        println!("   Validating 'Alice123': {}",
            if username_validator.validate(&invalid_caps, None).await.is_ok() { "✓" } else { "✗ (not lowercase)" });
        println!("   Validating 'ab': {}",
            if username_validator.validate(&invalid_short, None).await.is_ok() { "✓" } else { "✗ (too short)" });
        println!("   Validating 'alice@123': {}",
            if username_validator.validate(&invalid_special, None).await.is_ok() { "✓" } else { "✗ (special chars)" });
    }

    println!("\n=== All examples completed! ===");
    Ok(())
}
