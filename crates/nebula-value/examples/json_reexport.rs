//! Example demonstrating the re-exported json! macro
//!
//! Instead of importing serde_json::json!, you can use nebula_value::json!

use nebula_value::prelude::*;
use nebula_value::collections::array::ArrayBuilder;
use nebula_value::collections::object::ObjectBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use json! macro from nebula_value (re-exported from serde_json)
    let array = ArrayBuilder::new()
        .push(json!(1))
        .push(json!("hello"))
        .push(json!(true))
        .push(json!(null))
        .build()?;

    println!("Array: {:?}", array);

    // Build complex objects
    let user = ObjectBuilder::new()
        .insert("id", json!(42))
        .insert("name", json!("Alice"))
        .insert("email", json!("alice@example.com"))
        .insert("active", json!(true))
        .insert("tags", json!(["admin", "developer"]))
        .build()?;

    println!("User: {:?}", user);

    // Nested structures
    let config = ObjectBuilder::new()
        .insert("version", json!("1.0.0"))
        .insert("settings", json!({
            "timeout": 30,
            "retries": 3,
            "verbose": true
        }))
        .insert("endpoints", json!([
            "https://api.example.com",
            "https://backup.example.com"
        ]))
        .build()?;

    println!("Config: {:?}", config);

    Ok(())
}
