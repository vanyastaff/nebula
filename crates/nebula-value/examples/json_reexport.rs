//! Example demonstrating native Value construction
//!
//! Run with: cargo run --example json_reexport

use nebula_value::collections::array::ArrayBuilder;
use nebula_value::collections::object::ObjectBuilder;
use nebula_value::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build array with native Values
    let array = ArrayBuilder::new()
        .push(1)
        .push("hello")
        .push(true)
        .push(Value::Null)
        .build()?;

    println!("Array: {:?}", array);

    // Build complex objects
    let tags = Array::from_vec(vec!["admin".into(), "developer".into()]);
    let user = ObjectBuilder::new()
        .insert("id", 42)
        .insert("name", "Alice")
        .insert("email", "alice@example.com")
        .insert("active", true)
        .insert("tags", Value::Array(tags))
        .build()?;

    println!("User: {:?}", user);

    // Nested structures with parse_json helper
    let settings: Value = r#"{
        "timeout": 30,
        "retries": 3,
        "verbose": true
    }"#.parse()?;

    let endpoints = Array::from_vec(vec![
        "https://api.example.com".into(),
        "https://backup.example.com".into()
    ]);

    let config = ObjectBuilder::new()
        .insert("version", "1.0.0")
        .insert("settings", settings)
        .insert("endpoints", Value::Array(endpoints))
        .build()?;

    println!("Config: {:?}", config);

    Ok(())
}
