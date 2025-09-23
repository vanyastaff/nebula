use nebula_value::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 Simple Value Example");

    // Creating values using From trait
    let name = Value::from("John Doe");
    println!("   Name: {}", name);

    let age = Value::from(30i64);
    println!("   Age: {}", age);

    let active = Value::from(true);
    println!("   Active: {}", active);

    let height = Value::from(5.9f64);
    println!("   Height: {}", height);

    // Working with null
    let empty: Value = Value::null();
    println!("   Empty: {}", empty);

    println!("✅ Simple values created successfully");

    Ok(())
}