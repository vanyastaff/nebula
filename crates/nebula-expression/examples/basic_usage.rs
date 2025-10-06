//! Basic usage example of nebula-expression
//!
//! Run with: cargo run --example basic_usage

use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_value::Value;

fn main() {
    println!("=== Nebula Expression Engine - Basic Usage ===\n");

    // Create the expression engine
    let engine = ExpressionEngine::new();

    // Create a context
    let mut context = EvaluationContext::new();

    // Example 1: Simple arithmetic
    println!("Example 1: Simple arithmetic");
    let result = engine.evaluate("{{ 2 + 2 }}", &context).unwrap();
    println!("  {{ 2 + 2 }} = {}", result);

    // Example 2: String operations
    println!("\nExample 2: String operations");
    let result = engine
        .evaluate("{{ \"hello\" + \" \" + \"world\" }}", &context)
        .unwrap();
    println!("  {{ \"hello\" + \" \" + \"world\" }} = {}", result);

    // Example 3: Using variables
    println!("\nExample 3: Using variables");
    context.set_input(Value::Object(
        nebula_value::Object::new()
            .insert("name".to_string(), serde_json::json!("Alice"))
            .insert("age".to_string(), serde_json::json!(25)),
    ));

    let result = engine.evaluate("{{ $input.name }}", &context).unwrap();
    println!("  {{ $input.name }} = {}", result);

    let result = engine.evaluate("{{ $input.age }}", &context).unwrap();
    println!("  {{ $input.age }} = {}", result);

    // Example 4: Comparisons
    println!("\nExample 4: Comparisons");
    let result = engine.evaluate("{{ $input.age >= 18 }}", &context).unwrap();
    println!("  {{ $input.age >= 18 }} = {}", result);

    // Example 5: Conditionals
    println!("\nExample 5: Conditionals");
    let result = engine
        .evaluate(
            "{{ if $input.age >= 18 then \"adult\" else \"minor\" }}",
            &context,
        )
        .unwrap();
    println!(
        "  {{ if $input.age >= 18 then \"adult\" else \"minor\" }} = {}",
        result
    );

    // Example 6: Pipeline operations
    println!("\nExample 6: Pipeline operations");
    let result = engine
        .evaluate("{{ \"HELLO WORLD\" | lowercase() }}", &context)
        .unwrap();
    println!("  {{ \"HELLO WORLD\" | lowercase() }} = {}", result);

    // Example 7: Math functions
    println!("\nExample 7: Math functions");
    let result = engine.evaluate("{{ 3.14159 | round(2) }}", &context).unwrap();
    println!("  {{ 3.14159 | round(2) }} = {}", result);

    // Example 8: String functions
    println!("\nExample 8: String functions");
    let result = engine
        .evaluate("{{ \"hello,world,test\" | split(\",\") }}", &context)
        .unwrap();
    println!("  {{ \"hello,world,test\" | split(\",\") }} = {:?}", result);

    println!("\n=== All examples completed successfully! ===");
}
