//! Example demonstrating template rendering with multiple expressions
//!
//! This example shows how to use Template for parse-once, render-many pattern
//! with multiple {{ }} expressions in various formats (HTML, JSON, plain text)

use nebula_expression::{EvaluationContext, ExpressionEngine, Template};
use serde_json::Value;

fn main() {
    let engine = ExpressionEngine::new();

    // Example 1: Simple text template
    println!("=== Example 1: Simple Text Template ===");
    let mut context = EvaluationContext::new();
    context.set_input(Value::String("Alice".to_string()));
    context.set_execution_var("order_id", serde_json::json!(12345));

    let template = Template::new(
        "Hello {{ $input }}! Your order #{{ $execution.order_id }} is being processed.",
    )
    .unwrap();
    let result = template.render(&engine, &context).unwrap();
    println!("{}", result);

    // Example 2: HTML Email Template
    println!("\n=== Example 2: HTML Email Template ===");
    context.set_execution_var("total", serde_json::json!(99.99));
    context.set_execution_var("items_count", serde_json::json!(3));

    let html_template = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Order Confirmation</title>
</head>
<body>
    <h1>Thank you, {{ $input | uppercase() }}!</h1>
    <p>Your order #{{ $execution.order_id }} has been confirmed.</p>
    <div class="summary">
        <p>Items: {{ $execution.items_count }}</p>
        <p>Total: ${{ $execution.total }}</p>
    </div>
    <footer>Order placed on {{ now_iso() }}</footer>
</body>
</html>
"#;

    let result = Template::new(html_template)
        .unwrap()
        .render(&engine, &context)
        .unwrap();
    println!("{}", result);

    // Example 3: JSON Template
    println!("\n=== Example 3: JSON Template ===");
    context.set_execution_var("user_id", serde_json::json!(42));
    context.set_execution_var("username", Value::String("alice_dev".to_string()));

    let json_template = r#"{
  "user": {
    "id": {{ $execution.user_id }},
    "name": "{{ $input }}",
    "username": "{{ $execution.username }}",
    "display_name": "{{ $input | uppercase() }}"
  },
  "order": {
    "id": {{ $execution.order_id }},
    "total": {{ $execution.total }},
    "timestamp": {{ now() }}
  }
}"#;

    let result = Template::new(json_template)
        .unwrap()
        .render(&engine, &context)
        .unwrap();
    println!("{}", result);

    // Example 4: Markdown Document
    println!("\n=== Example 4: Markdown Document ===");
    context.set_execution_var("product", Value::String("Premium Widget".to_string()));
    context.set_execution_var("price", serde_json::json!(29.99));

    let markdown_template = r#"
# Order Summary

**Customer:** {{ $input }}
**Order ID:** #{{ $execution.order_id }}

## Items

1. {{ $execution.product }} - ${{ $execution.price }}

**Total:** ${{ $execution.total }}

---

*Generated on {{ now_iso() }}*
"#;

    let result = Template::new(markdown_template)
        .unwrap()
        .render(&engine, &context)
        .unwrap();
    println!("{}", result);

    // Example 5: Complex expressions with functions
    println!("\n=== Example 5: Complex Expressions ===");
    context.set_input(serde_json::json!(1704067200)); // 2024-01-01 00:00:00 UTC

    let template = r#"
Report for {{ $execution.username }}:
- Name length: {{ length($execution.username) }} characters
- Uppercase: {{ $execution.username | uppercase() }}
- Order timestamp: {{ $input }}
- Order year: {{ $input | date_year() }}
- Order month: {{ $input | date_month() }}
- Formatted: {{ $input | format_date("YYYY-MM-DD HH:mm") }}
"#;

    let result = Template::new(template)
        .unwrap()
        .render(&engine, &context)
        .unwrap();
    println!("{}", result);

    // Example 6: Conditional content (using pipeline)
    println!("\n=== Example 6: With Calculations ===");
    context.set_execution_var("quantity", serde_json::json!(5));
    context.set_execution_var("unit_price", serde_json::json!(19.99));

    let template = r#"
Invoice:
--------
Quantity: {{ $execution.quantity }}
Unit Price: ${{ $execution.unit_price }}
Subtotal: ${{ $execution.quantity * $execution.unit_price }}
Tax (10%): ${{ $execution.quantity * $execution.unit_price * 0.1 | round(2) }}
Total: ${{ $execution.quantity * $execution.unit_price * 1.1 | round(2) }}
"#;

    let result = Template::new(template)
        .unwrap()
        .render(&engine, &context)
        .unwrap();
    println!("{}", result);
}
