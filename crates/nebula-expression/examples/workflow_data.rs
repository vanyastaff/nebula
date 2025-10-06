//! Example showing workflow data processing
//!
//! Run with: cargo run --example workflow_data

use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_value::Value;
use serde_json::json;

fn main() {
    println!("=== Nebula Expression - Workflow Data Processing ===\n");

    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();

    // Simulate an HTTP node response
    context.set_node_data(
        "http",
        Value::Object(
            nebula_value::Object::new()
                .insert(
                    "response".to_string(),
                    json!({
                        "statusCode": 200,
                        "body": {
                            "user": {
                                "name": "John Doe",
                                "email": "john@example.com",
                                "price": 123.456,
                                "active": true
                            }
                        }
                    }),
                )
                .insert("timestamp".to_string(), json!(1698765432)),
        ),
    );

    // Example 1: Access nested data
    println!("Example 1: Access nested data from HTTP response");
    let result = engine
        .evaluate("{{ $node.http.response.body.user.name }}", &context)
        .unwrap();
    println!("  User name: {}", result);

    // Example 2: Format price with rounding
    println!("\nExample 2: Format price with rounding");
    let result = engine
        .evaluate(
            "{{ $node.http.response.body.user.price | round(2) }}",
            &context,
        )
        .unwrap();
    println!("  Rounded price: {}", result);

    // Example 3: Conditional based on status
    println!("\nExample 3: Conditional based on user status");
    let result = engine
        .evaluate(
            "{{ if $node.http.response.body.user.active then \"Active User\" else \"Inactive User\" }}",
            &context,
        )
        .unwrap();
    println!("  Status: {}", result);

    // Example 4: String transformation
    println!("\nExample 4: String transformation");
    let result = engine
        .evaluate(
            "{{ $node.http.response.body.user.email | uppercase() }}",
            &context,
        )
        .unwrap();
    println!("  Email (uppercase): {}", result);

    // Example 5: Type conversion
    println!("\nExample 5: Type conversion");
    let result = engine
        .evaluate(
            "{{ $node.http.response.body.user.price | to_string() }}",
            &context,
        )
        .unwrap();
    println!("  Price as string: {}", result);

    // Add execution variables
    context.set_execution_var("id", Value::text("exec-12345"));
    context.set_execution_var("mode", Value::text("production"));

    println!("\nExample 6: Access execution variables");
    let result = engine.evaluate("{{ $execution.id }}", &context).unwrap();
    println!("  Execution ID: {}", result);

    let result = engine.evaluate("{{ $execution.mode }}", &context).unwrap();
    println!("  Execution mode: {}", result);

    println!("\n=== Workflow data examples completed! ===");
}
