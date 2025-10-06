//! Example demonstrating MaybeExpression usage
//!
//! Run with: cargo run --example maybe_expression

use nebula_expression::{EvaluationContext, ExpressionEngine, MaybeExpression};
use nebula_value::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowConfig {
    /// Timeout can be a fixed value or an expression
    timeout: MaybeExpression<i64>,
    /// Message can be static or dynamic
    message: MaybeExpression<String>,
    /// Retry count can be computed
    retry_count: MaybeExpression<i64>,
    /// Flag can be conditional
    enabled: MaybeExpression<bool>,
}

fn main() {
    println!("=== MaybeExpression Example ===\n");

    // Example 1: Static configuration
    println!("Example 1: Static configuration");
    let config_json = r#"{
        "timeout": 30,
        "message": "Hello, World!",
        "retry_count": 3,
        "enabled": true
    }"#;

    let config: WorkflowConfig = serde_json::from_str(config_json).unwrap();
    println!("  Timeout: {:?}", config.timeout);
    println!("  Message: {:?}", config.message);
    println!("  Retry count: {:?}", config.retry_count);
    println!("  Enabled: {:?}", config.enabled);

    // Example 2: Dynamic configuration with expressions
    println!("\nExample 2: Dynamic configuration with expressions");
    let dynamic_config_json = r#"{
        "timeout": "{{ $input.timeout_seconds }}",
        "message": "{{ \"User: \" + $input.username }}",
        "retry_count": "{{ $input.max_retries }}",
        "enabled": "{{ $input.priority > 5 }}"
    }"#;

    let dynamic_config: WorkflowConfig = serde_json::from_str(dynamic_config_json).unwrap();
    println!("  Timeout: {:?}", dynamic_config.timeout);
    println!("  Message: {:?}", dynamic_config.message);
    println!("  Retry count: {:?}", dynamic_config.retry_count);
    println!("  Enabled: {:?}", dynamic_config.enabled);

    // Example 3: Resolving expressions
    println!("\nExample 3: Resolving expressions");
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();

    context.set_input(Value::Object(
        nebula_value::Object::new()
            .insert("timeout_seconds".to_string(), serde_json::json!(60))
            .insert("username".to_string(), serde_json::json!("Alice"))
            .insert("max_retries".to_string(), serde_json::json!(5))
            .insert("priority".to_string(), serde_json::json!(8)),
    ));

    let timeout = dynamic_config
        .timeout
        .resolve_as_integer(&engine, &context)
        .unwrap();
    println!("  Resolved timeout: {}", timeout);

    let message = dynamic_config
        .message
        .resolve_as_string(&engine, &context)
        .unwrap();
    println!("  Resolved message: {}", message);

    let retry_count = dynamic_config
        .retry_count
        .resolve_as_integer(&engine, &context)
        .unwrap();
    println!("  Resolved retry count: {}", retry_count);

    let enabled = dynamic_config
        .enabled
        .resolve_as_bool(&engine, &context)
        .unwrap();
    println!("  Resolved enabled: {}", enabled);

    // Example 4: Mixed static and dynamic
    println!("\nExample 4: Mixed static and dynamic configuration");
    let mixed_config_json = r#"{
        "timeout": 30,
        "message": "{{ $input.username + \" is awesome!\" }}",
        "retry_count": 3,
        "enabled": "{{ $input.is_production }}"
    }"#;

    let mixed_config: WorkflowConfig = serde_json::from_str(mixed_config_json).unwrap();

    context.set_input(Value::Object(
        nebula_value::Object::new()
            .insert("username".to_string(), serde_json::json!("Bob"))
            .insert("is_production".to_string(), serde_json::json!(false)),
    ));

    // Static values resolve directly
    let timeout = mixed_config
        .timeout
        .resolve_as_integer(&engine, &context)
        .unwrap();
    println!("  Timeout (static): {}", timeout);

    // Dynamic values are evaluated
    let message = mixed_config
        .message
        .resolve_as_string(&engine, &context)
        .unwrap();
    println!("  Message (dynamic): {}", message);

    let retry_count = mixed_config
        .retry_count
        .resolve_as_integer(&engine, &context)
        .unwrap();
    println!("  Retry count (static): {}", retry_count);

    let enabled = mixed_config
        .enabled
        .resolve_as_bool(&engine, &context)
        .unwrap();
    println!("  Enabled (dynamic): {}", enabled);

    println!("\n=== All examples completed! ===");
}
