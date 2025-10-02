//! Operations examples for nebula-value
//!
//! Run with: cargo run --example operations

use nebula_value::collections::array::ArrayBuilder;
use nebula_value::collections::object::ObjectBuilder;
use nebula_value::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula Value Operations ===\n");

    // Arithmetic operations
    arithmetic_operations()?;

    // Comparison operations
    comparison_operations()?;

    // Logical operations
    logical_operations();

    // Merge operations
    merge_operations()?;

    // Path access
    path_access_examples()?;

    Ok(())
}

fn arithmetic_operations() -> ValueResult<()> {
    println!("1. Arithmetic Operations:");

    let a = Value::integer(10);
    let b = Value::integer(5);

    let sum = a.add(&b)?;
    let diff = a.sub(&b)?;
    let prod = a.mul(&b)?;
    let quot = a.div(&b)?;
    let rem = a.rem(&b)?;

    println!("  10 + 5 = {}", sum);
    println!("  10 - 5 = {}", diff);
    println!("  10 * 5 = {}", prod);
    println!("  10 / 5 = {}", quot);
    println!("  10 % 5 = {}", rem);

    // Type coercion: Integer + Float -> Float
    let int = Value::integer(10);
    let float = Value::float(3.5);
    let result = int.add(&float)?;
    println!("  10 + 3.5 = {} (auto-coercion to float)", result);

    // Text concatenation
    let hello = Value::text("Hello ");
    let world = Value::text("World");
    let greeting = hello.add(&world)?;
    println!("  'Hello ' + 'World' = {}\n", greeting);

    Ok(())
}

fn comparison_operations() -> ValueResult<()> {
    println!("2. Comparison Operations:");

    let a = Value::integer(10);
    let b = Value::integer(5);
    let c = Value::integer(10);

    println!("  10 > 5: {}", a.gt(&b)?);
    println!("  10 < 5: {}", a.lt(&b)?);
    println!("  10 >= 10: {}", a.ge(&c)?);
    println!("  10 <= 10: {}", a.le(&c)?);

    // Float comparison with NaN handling
    let f1 = Value::float(3.14);
    let f2 = Value::float(2.71);
    println!("  3.14 > 2.71: {}", f1.gt(&f2)?);

    // Text comparison
    let apple = Value::text("apple");
    let banana = Value::text("banana");
    println!("  'apple' < 'banana': {}\n", apple.lt(&banana)?);

    Ok(())
}

fn logical_operations() {
    println!("3. Logical Operations:");

    let t = Value::boolean(true);
    let f = Value::boolean(false);

    println!("  true AND false: {}", t.and(&f));
    println!("  true OR false: {}", t.or(&f));
    println!("  NOT true: {}", t.not());
    println!("  NOT false: {}", f.not());

    // Truthy/falsy values
    let zero = Value::integer(0);
    let non_zero = Value::integer(42);
    let empty_text = Value::text("");
    let text = Value::text("hello");

    println!("  0 is truthy: {}", zero.is_truthy());
    println!("  42 is truthy: {}", non_zero.is_truthy());
    println!("  '' is falsy: {}", empty_text.is_falsy());
    println!("  'hello' is truthy: {}\n", text.is_truthy());
}

fn merge_operations() -> ValueResult<()> {
    println!("4. Merge Operations:");

    // Merge objects
    let obj1 = ObjectBuilder::new()
        .insert("a", serde_json::json!(1))
        .insert("b", serde_json::json!(2))
        .build()?;

    let obj2 = ObjectBuilder::new()
        .insert("c", serde_json::json!(3))
        .insert("d", serde_json::json!(4))
        .build()?;

    let merged = Value::Object(obj1).merge(&Value::Object(obj2))?;
    println!(
        "  Merged object keys: {}",
        if let Value::Object(o) = &merged {
            o.len()
        } else {
            0
        }
    );

    // Merge arrays (concatenation)
    let arr1 = ArrayBuilder::new()
        .push(serde_json::json!(1))
        .push(serde_json::json!(2))
        .build()?;

    let arr2 = ArrayBuilder::new()
        .push(serde_json::json!(3))
        .push(serde_json::json!(4))
        .build()?;

    let merged_arr = Value::Array(arr1).merge(&Value::Array(arr2))?;
    println!(
        "  Merged array length: {}\n",
        if let Value::Array(a) = &merged_arr {
            a.len()
        } else {
            0
        }
    );

    Ok(())
}

fn path_access_examples() -> ValueResult<()> {
    println!("5. Path Access:");

    // Note: Currently limited due to serde_json::Value placeholder
    // Will be fully functional once Array/Object use Value internally

    let obj = ObjectBuilder::new()
        .insert("name", serde_json::json!("Alice"))
        .insert("age", serde_json::json!(30))
        .build()?;

    let value = Value::Object(obj);

    // Direct key access
    if let Value::Object(ref o) = value {
        println!("  Direct access obj['name']: {:?}", o.get("name"));
        println!("  Direct access obj['age']: {:?}", o.get("age"));
    }

    println!("\n  Note: Full path syntax ($.user.name) will be available");
    println!("  once Array/Object are migrated from serde_json::Value\n");

    Ok(())
}
