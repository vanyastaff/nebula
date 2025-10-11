//! Limits and validation examples for nebula-value
//!
//! Run with: cargo run --example limits_and_validation

use nebula_value::collections::array::ArrayBuilder;
use nebula_value::collections::object::ObjectBuilder;
use nebula_value::core::limits::ValueLimits;
use nebula_value::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula Value Limits & Validation ===\n");

    // Default limits
    default_limits();

    // Strict limits for untrusted input
    strict_limits()?;

    // Custom limits
    custom_limits()?;

    // Validation in builders
    builder_validation()?;

    Ok(())
}

fn default_limits() {
    println!("1. Default Limits:");

    let limits = ValueLimits::default();

    println!("  max_array_length: {}", limits.max_array_length);
    println!("  max_object_keys: {}", limits.max_object_keys);
    println!("  max_string_bytes: {}", limits.max_string_bytes);
    println!("  max_bytes_length: {}", limits.max_bytes_length);
    println!("  max_nesting_depth: {}\n", limits.max_nesting_depth);
}

fn strict_limits() -> ValueResult<()> {
    println!("2. Strict Limits (for untrusted input):");

    let limits = ValueLimits::strict();

    println!("  max_array_length: {}", limits.max_array_length);
    println!("  max_object_keys: {}", limits.max_object_keys);
    println!("  max_nesting_depth: {}", limits.max_nesting_depth);

    // Try to create array that exceeds strict limits
    let result = ArrayBuilder::new()
        .with_limits(limits)
        .extend((0..2000).map(|i| Value::integer(i as i64)))
        .build();

    match result {
        Ok(_) => println!("  Array creation succeeded"),
        Err(e) => println!("  Array creation failed (expected): {}", e),
    }

    println!();
    Ok(())
}

fn custom_limits() -> ValueResult<()> {
    println!("3. Custom Limits:");

    let limits = ValueLimits {
        max_array_length: 10,
        max_object_keys: 5,
        max_string_bytes: 100,
        max_bytes_length: 1000,
        max_nesting_depth: 5,
    };

    println!("  Custom max_array_length: {}", limits.max_array_length);
    println!("  Custom max_object_keys: {}", limits.max_object_keys);

    // Create array within limits
    let result = ArrayBuilder::new()
        .with_limits(limits)
        .push(serde_json::json!(1))
        .push(serde_json::json!(2))
        .push(serde_json::json!(3))
        .build();

    match result {
        Ok(arr) => println!("  Created array with {} items", arr.len()),
        Err(e) => println!("  Failed: {}", e),
    }

    println!();
    Ok(())
}

fn builder_validation() -> ValueResult<()> {
    println!("4. Builder Validation:");

    let limits = ValueLimits {
        max_array_length: 3,
        max_object_keys: 2,
        max_string_bytes: 50,
        max_bytes_length: 1000,
        max_nesting_depth: 10,
    };

    // Array builder with try_push
    println!("  ArrayBuilder with validation:");
    let result = ArrayBuilder::new()
        .with_limits(limits)
        .try_push(serde_json::json!(1))?
        .try_push(serde_json::json!(2))?
        .try_push(serde_json::json!(3))?
        .try_push(serde_json::json!(4)); // This should fail

    match result {
        Ok(_) => println!("    Unexpected success"),
        Err(e) => println!("    Expected failure: {}", e),
    }

    // Object builder with try_insert
    println!("\n  ObjectBuilder with validation:");
    let result = ObjectBuilder::new()
        .with_limits(limits)
        .try_insert("a", serde_json::json!(1))?
        .try_insert("b", serde_json::json!(2))?
        .try_insert("c", serde_json::json!(3)); // This should fail

    match result {
        Ok(_) => println!("    Unexpected success"),
        Err(e) => println!("    Expected failure: {}", e),
    }

    // Key length validation
    println!("\n  Key length validation:");
    let result = ObjectBuilder::new().with_limits(limits).try_insert(
        "this_is_a_very_long_key_name_that_exceeds_the_limit",
        serde_json::json!(1),
    );

    match result {
        Ok(_) => println!("    Unexpected success"),
        Err(e) => println!("    Expected failure: {}", e),
    }

    println!();
    Ok(())
}

#[allow(dead_code)]
fn permissive_limits_example() {
    println!("5. Permissive Limits (for trusted input):");

    let limits = ValueLimits::permissive();

    println!("  max_array_length: {}", limits.max_array_length);
    println!("  max_object_keys: {}", limits.max_object_keys);
    println!("  max_nesting_depth: {}\n", limits.max_nesting_depth);
}
