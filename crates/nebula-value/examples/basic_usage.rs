//! Basic usage examples for nebula-value
//!
//! Run with: cargo run --example basic_usage

use nebula_value::prelude::*;
use nebula_value::collections::array::ArrayBuilder;
use nebula_value::collections::object::ObjectBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula Value Basic Usage ===\n");

    // Creating simple values
    creating_values();

    // Working with collections
    working_with_collections()?;

    // Type conversions
    type_conversions()?;

    // Using builders
    using_builders()?;

    // Hashing and HashMap usage
    hashing_examples();

    // Serialization
    serialization_examples()?;

    Ok(())
}

fn creating_values() {
    println!("1. Creating Values:");

    let null = Value::Null;
    let boolean = Value::boolean(true);
    let integer = Value::integer(42);
    let float = Value::float(3.14);
    let text = Value::text("hello world");
    let bytes = Value::bytes(vec![1, 2, 3, 4]);

    println!("  Null: {}", null);
    println!("  Boolean: {}", boolean);
    println!("  Integer: {}", integer);
    println!("  Float: {}", float);
    println!("  Text: {}", text);
    println!("  Bytes: {}\n", bytes);
}

fn working_with_collections() -> ValueResult<()> {
    println!("2. Working with Collections:");

    // Create array
    let mut array = Array::new();
    array = array.push(serde_json::json!(1));
    array = array.push(serde_json::json!(2));
    array = array.push(serde_json::json!(3));

    println!("  Array length: {}", array.len());
    println!("  Array[0]: {:?}", array.get(0));

    // Create object
    let mut object = Object::new();
    object = object.insert("name".to_string(), serde_json::json!("Alice"));
    object = object.insert("age".to_string(), serde_json::json!(30));
    object = object.insert("active".to_string(), serde_json::json!(true));

    println!("  Object keys: {}", object.len());
    println!("  Object['name']: {:?}\n", object.get("name"));

    Ok(())
}

fn type_conversions() -> ValueResult<()> {
    println!("3. Type Conversions:");

    // From primitives to Value
    let val1 = Value::from(42i64);
    let val2 = Value::from(3.14f64);
    let val3 = Value::from("hello");
    let val4 = Value::from(true);

    println!("  From i64: {}", val1);
    println!("  From f64: {}", val2);
    println!("  From &str: {}", val3);
    println!("  From bool: {}", val4);

    // From Value to primitives
    let num: i64 = val1.as_integer().unwrap();
    let text: &str = val3.as_str().unwrap();
    let flag: bool = val4.as_boolean().unwrap();

    println!("  To i64: {}", num);
    println!("  To &str: {}", text);
    println!("  To bool: {}\n", flag);

    Ok(())
}

fn using_builders() -> ValueResult<()> {
    println!("4. Using Builders:");

    // ArrayBuilder
    let array = ArrayBuilder::new()
        .push(serde_json::json!(1))
        .push(serde_json::json!(2))
        .push(serde_json::json!(3))
        .build()?;

    println!("  Built array: {} items", array.len());

    // ObjectBuilder
    let object = ObjectBuilder::new()
        .insert("name", serde_json::json!("Bob"))
        .insert("score", serde_json::json!(95))
        .insert("passed", serde_json::json!(true))
        .build()?;

    println!("  Built object: {} keys\n", object.len());

    Ok(())
}

fn hashing_examples() {
    use std::collections::HashMap;
    use nebula_value::core::hash::HashableValue;

    println!("5. Hashing & HashMap:");

    let mut map = HashMap::new();

    map.insert(HashableValue(Value::integer(1)), "one");
    map.insert(HashableValue(Value::integer(2)), "two");
    map.insert(HashableValue(Value::text("key")), "value");

    println!("  HashMap size: {}", map.len());
    println!("  map[1]: {:?}", map.get(&HashableValue(Value::integer(1))));
    println!("  map['key']: {:?}\n", map.get(&HashableValue(Value::text("key"))));
}

fn serialization_examples() -> Result<(), Box<dyn std::error::Error>> {
    println!("6. Serialization:");

    // Serialize to JSON
    let value = Value::integer(42);
    let json = serde_json::to_string(&value)?;
    println!("  Serialized: {}", json);

    // Deserialize from JSON
    let deserialized: Value = serde_json::from_str(&json)?;
    println!("  Deserialized: {}", deserialized);

    // Pretty print
    let complex = Value::integer(123);
    println!("  Pretty: {}\n", complex.pretty_print());

    Ok(())
}