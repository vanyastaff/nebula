//! Advanced template usage with Template struct
//!
//! Demonstrates:
//! - Using Template struct for parsing and caching
//! - Position tracking for error reporting
//! - MaybeTemplate for conditional rendering

use nebula_expression::{
    EvaluationContext, ExpressionEngine, MaybeTemplate, Template, TemplatePart,
};
use nebula_value::Value;

fn main() {
    let engine = ExpressionEngine::new();

    // Example 1: Basic Template usage
    println!("=== Example 1: Template Parsing and Rendering ===");
    let template =
        Template::new("Hello {{ $input }}, you have {{ $execution.count }} messages!").unwrap();

    println!("Template source: {}", template.source());
    println!("Number of parts: {}", template.parts().len());
    println!("Number of expressions: {}", template.expression_count());
    println!("Expressions: {:?}", template.expressions());

    let mut context = EvaluationContext::new();
    context.set_input(Value::text("Alice"));
    context.set_execution_var("count", Value::integer(5));

    let result = template.render(&engine, &context).unwrap();
    println!("Result: {}\n", result);

    // Example 2: Template part inspection
    println!("=== Example 2: Inspecting Template Parts ===");
    let template = Template::new("Static {{ expr1 }} more static {{ expr2 }}").unwrap();

    for (i, part) in template.parts().iter().enumerate() {
        match part {
            TemplatePart::Static { content, position } => {
                println!("Part {}: Static at {} = {:?}", i, position, content);
            }
            TemplatePart::Expression {
                content,
                position,
                length,
                strip_left,
                strip_right,
            } => {
                println!(
                    "Part {}: Expression at {} (length: {}, strip_left: {}, strip_right: {}) = {:?}",
                    i, position, length, strip_left, strip_right, content
                );
            }
        }
    }
    println!();

    // Example 3: Error handling with position information
    println!("=== Example 3: Error with Position Information ===");
    let template = Template::new(
        r#"Line 1
Line 2 with {{ invalid_function() }}
Line 3"#,
    )
    .unwrap();

    match template.render(&engine, &context) {
        Ok(result) => println!("Result: {}", result),
        Err(e) => println!("Error: {}\n", e),
    }

    // Example 4: MaybeTemplate auto-detection
    println!("=== Example 4: MaybeTemplate Auto-Detection ===");

    let dynamic = MaybeTemplate::from_string("Hello {{ $input }}!");
    let static_text = MaybeTemplate::from_string("Hello World!");

    println!("Dynamic template? {}", dynamic.is_template());
    println!("Static template? {}", static_text.is_template());

    context.set_input(Value::text("Bob"));

    let result1 = dynamic.resolve(&engine, &context).unwrap();
    let result2 = static_text.resolve(&engine, &context).unwrap();

    println!("Dynamic result: {}", result1);
    println!("Static result: {}\n", result2);

    // Example 5: Multiline template with position tracking
    println!("=== Example 5: Multiline Template ===");
    let html = Template::new(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>{{ $execution.title }}</title>
</head>
<body>
    <h1>Welcome, {{ $input | uppercase() }}!</h1>
    <p>You have {{ $execution.message_count }} new messages.</p>
    <p>Last login: {{ $execution.last_login | format_date("YYYY-MM-DD HH:mm") }}</p>
</body>
</html>"#,
    )
    .unwrap();

    context.set_input(Value::text("charlie"));
    context.set_execution_var("title", Value::text("Dashboard"));
    context.set_execution_var("message_count", Value::integer(42));
    context.set_execution_var("last_login", Value::integer(1704067200)); // 2024-01-01

    let result = html.render(&engine, &context).unwrap();
    println!("{}\n", result);

    // Example 6: Template reusability
    println!("=== Example 6: Template Reusability ===");
    let greeting_template =
        Template::new("Hello {{ $input }}, your score is {{ $execution.score }}!").unwrap();

    // Render with different contexts
    let users = vec![("Alice", 100), ("Bob", 85), ("Charlie", 92)];

    for (name, score) in users {
        context.set_input(Value::text(name));
        context.set_execution_var("score", Value::integer(score));

        let result = greeting_template.render(&engine, &context).unwrap();
        println!("{}", result);
    }
    println!();

    // Example 7: Unclosed expression error
    println!("=== Example 7: Parse Error with Position ===");
    match Template::new("Hello {{ $input") {
        Ok(_) => println!("Parsed successfully"),
        Err(e) => println!("Parse error: {}", e),
    }
    println!();

    // Example 8: Complex nested template
    println!("=== Example 8: JSON Template ===");
    let json_template = Template::new(
        r#"{
  "user": {
    "name": "{{ $input }}",
    "id": {{ $execution.user_id }},
    "roles": {{ $execution.roles | to_json() }},
    "active": {{ $execution.active }}
  },
  "metadata": {
    "timestamp": {{ now() }},
    "version": "1.0"
  }
}"#,
    )
    .unwrap();

    context.set_input(Value::text("admin"));
    context.set_execution_var("user_id", Value::integer(1));

    // Create array manually
    let mut roles = nebula_value::Array::new();
    roles = roles.push(serde_json::Value::String("admin".to_string()));
    roles = roles.push(serde_json::Value::String("user".to_string()));
    context.set_execution_var("roles", Value::Array(roles));

    context.set_execution_var("active", Value::Boolean(true));

    let result = json_template.render(&engine, &context).unwrap();
    println!("{}", result);
}
