//! Examples of enhanced error handling with context and suggestions.
//!
//! Run with: cargo run --example enhanced_errors --features serde

use nebula_value::error_ext::{EnhancedError, ErrorBuilder, ValueErrorExt};
use nebula_value::{Object, Value, ValueError};

fn main() {
    println!("=== Enhanced Error Examples ===\n");

    example_type_mismatch();
    println!("\n{}", "=".repeat(60));

    example_key_not_found();
    println!("\n{}", "=".repeat(60));

    example_index_out_of_bounds();
    println!("\n{}", "=".repeat(60));

    example_conversion_error();
    println!("\n{}", "=".repeat(60));

    example_nested_context();
}

fn example_type_mismatch() {
    println!("\n📝 Example 1: Type Mismatch with Suggestions\n");

    // Simulate a type mismatch error
    let error = ValueError::type_mismatch("Integer", "String");

    // Enhance with suggestions
    let enhanced = EnhancedError::new(error)
        .with_suggestion("Use Value::integer() to create an integer value")
        .with_suggestion("Or use to_integer() to convert the string")
        .with_hint("The value contains text, not a number")
        .with_context("While processing user input")
        .with_doc("https://docs.rs/nebula-value/latest/nebula_value/core/value/enum.Value.html");

    println!("{}", enhanced);
}

fn example_key_not_found() {
    println!("\n📝 Example 2: Key Not Found with Available Keys\n");

    let available_keys = vec![
        "username".to_string(),
        "email".to_string(),
        "age".to_string(),
        "created_at".to_string(),
    ];

    let error = ErrorBuilder::key_not_found("user_name", &available_keys);

    println!("{}", error);
}

fn example_index_out_of_bounds() {
    println!("\n📝 Example 3: Index Out of Bounds\n");

    let error = ErrorBuilder::index_out_of_bounds(10, 5);

    println!("{}", error);
}

fn example_conversion_error() {
    println!("\n📝 Example 4: Conversion Error\n");

    let error = ErrorBuilder::conversion_error("Text", "Integer", "\"hello world\"");

    println!("{}", error);
}

fn example_nested_context() {
    println!("\n📝 Example 5: Nested Context Chain\n");

    // Simulate a complex operation with multiple context layers
    let result = process_user_data();

    if let Err(error) = result {
        println!("{}", error);
    }
}

fn process_user_data() -> Result<(), EnhancedError> {
    validate_profile()
        .map_err(|e| e.with_context("While processing user data"))
        .map_err(|e| e.with_context("In API endpoint /api/users/create"))
        .map_err(|e| e.with_context("Request from client 192.168.1.100"))
}

fn validate_profile() -> Result<(), EnhancedError> {
    check_required_fields()
        .map_err(|e| e.with_context("While validating user profile"))
}

fn check_required_fields() -> Result<(), EnhancedError> {
    // Simulate missing required field
    Err(ValueError::key_not_found("email")
        .enhanced()
        .with_suggestion("Add an 'email' field to the user profile")
        .with_suggestion("Or mark email as optional in the schema")
        .with_hint("Email is required for user registration")
        .with_doc("https://docs.rs/nebula-value"))
}

// Additional example: Using with real Values

#[allow(dead_code)]
fn real_world_example() {
    let obj = Object::from_iter(vec![
        ("name".to_string(), Value::text("Alice")),
        ("age".to_string(), Value::integer(30)),
    ]);

    // Try to access a non-existent key
    match obj.get("email") {
        Some(value) => println!("Email: {}", value),
        None => {
            let keys: Vec<String> = obj.keys().cloned().collect();
            let error = ErrorBuilder::key_not_found("email", &keys)
                .with_context("While loading user preferences");

            eprintln!("{}", error);
        }
    }
}
