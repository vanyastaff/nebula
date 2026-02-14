//! Comparison of MaybeExpression vs MaybeTemplate
//!
//! This example demonstrates when to use each type and how they work together.

use nebula_expression::{
    EvaluationContext, ExpressionEngine, MaybeExpression, MaybeTemplate, Template,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration using MaybeExpression for typed parameters
#[derive(Debug, Serialize, Deserialize)]
struct ApiConfig {
    /// Timeout in milliseconds - can be static value or expression
    #[serde(default)]
    timeout: MaybeExpression<i64>,

    /// Retry count - can be static value or expression
    #[serde(default)]
    retry_count: MaybeExpression<i64>,

    /// Base URL - can be static string or expression
    #[serde(default)]
    base_url: MaybeExpression<String>,

    /// Enable debug mode - can be static bool or expression
    #[serde(default)]
    debug: MaybeExpression<bool>,
}

/// Email template using MaybeTemplate for text with multiple expressions
#[derive(Debug, Serialize, Deserialize)]
struct EmailTemplate {
    /// Subject line - may contain {{ }} expressions
    subject: MaybeTemplate,

    /// Email body - may contain multiple {{ }} expressions
    body: MaybeTemplate,

    /// From address - may be static or expression
    from: MaybeExpression<String>,
}

fn main() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();

    // Set up context
    context.set_input(Value::integer(3));
    context.set_execution_var("env", Value::text("production"));
    context.set_execution_var("user_name", Value::text("Alice"));
    context.set_execution_var("user_email", Value::text("alice@example.com"));
    context.set_execution_var("order_id", Value::integer(12345));
    context.set_execution_var("total", Value::float(99.99));

    // Example 1: MaybeExpression for typed configuration
    println!("=== Example 1: MaybeExpression for Configuration ===\n");

    // Static configuration
    let static_config = r#"{
        "timeout": 5000,
        "retry_count": 3,
        "base_url": "https://api.example.com",
        "debug": false
    }"#;

    let config: ApiConfig = serde_json::from_str(static_config).unwrap();
    println!("Static config (from JSON):");
    println!("  timeout: {:?}", config.timeout);
    println!("  retry_count: {:?}", config.retry_count);
    println!("  base_url: {:?}", config.base_url);
    println!("  debug: {:?}\n", config.debug);

    // Resolve values
    let timeout = config.timeout.resolve(&engine, &context).unwrap();
    let retry_count = config.retry_count.resolve(&engine, &context).unwrap();
    let base_url = config.base_url.resolve(&engine, &context).unwrap();
    let debug = config.debug.resolve(&engine, &context).unwrap();

    println!("Resolved values:");
    println!("  timeout: {}", timeout);
    println!("  retry_count: {}", retry_count);
    println!("  base_url: {}", base_url);
    println!("  debug: {}\n", debug);

    // Dynamic configuration with expressions
    let dynamic_config = r#"{
        "timeout": "{{ $input * 1000 }}",
        "retry_count": "{{ $input }}",
        "base_url": "{{ \"https://\" + $execution.env + \".api.example.com\" }}",
        "debug": "{{ $execution.env == \"development\" }}"
    }"#;

    let config: ApiConfig = serde_json::from_str(dynamic_config).unwrap();
    println!("Dynamic config (with expressions):");
    println!("  timeout: {:?}", config.timeout);
    println!("  retry_count: {:?}", config.retry_count);
    println!("  base_url: {:?}", config.base_url);
    println!("  debug: {:?}\n", config.debug);

    // Resolve values
    let timeout = config.timeout.resolve(&engine, &context).unwrap();
    let retry_count = config.retry_count.resolve(&engine, &context).unwrap();
    let base_url = config.base_url.resolve(&engine, &context).unwrap();
    let debug = config.debug.resolve(&engine, &context).unwrap();

    println!("Resolved values:");
    println!("  timeout: {} (computed from $input * 1000)", timeout);
    println!("  retry_count: {} (from $input)", retry_count);
    println!("  base_url: {} (computed from env)", base_url);
    println!("  debug: {} (env is production, not development)\n", debug);

    // Example 2: MaybeTemplate for text templates
    println!("=== Example 2: MaybeTemplate for Text Templates ===\n");

    // Static email template
    let static_email = r#"{
        "subject": "Welcome to our service",
        "body": "Thank you for signing up!",
        "from": "noreply@example.com"
    }"#;

    let email: EmailTemplate = serde_json::from_str(static_email).unwrap();
    println!("Static email template:");
    println!("  subject: {}", email.subject.as_str());
    println!("  body: {}", email.body.as_str());
    println!("  from: {:?}\n", email.from);

    // Resolve
    let subject = email.subject.resolve(&engine, &context).unwrap();
    let body = email.body.resolve(&engine, &context).unwrap();
    let from = email.from.resolve(&engine, &context).unwrap();

    println!("Rendered email:");
    println!("  From: {}", from);
    println!("  Subject: {}", subject);
    println!("  Body: {}\n", body);

    // Dynamic email template with multiple expressions
    let dynamic_email = r#"{
        "subject": "Order #{{ $execution.order_id }} Confirmation",
        "body": "Dear {{ $execution.user_name }},\n\nThank you for your order #{{ $execution.order_id }}.\n\nOrder Details:\n- Total: ${{ $execution.total }}\n- Status: Processing\n\nWe will send updates to {{ $execution.user_email }}.\n\nBest regards,\nThe Team",
        "from": "{{ \"orders@\" + $execution.env + \".example.com\" }}"
    }"#;

    let email: EmailTemplate = serde_json::from_str(dynamic_email).unwrap();
    println!("Dynamic email template:");
    println!("  subject.is_template(): {}", email.subject.is_template());
    println!("  body.is_template(): {}", email.body.is_template());
    println!("  from: {:?}\n", email.from);

    // Resolve
    let subject = email.subject.resolve(&engine, &context).unwrap();
    let body = email.body.resolve(&engine, &context).unwrap();
    let from = email.from.resolve(&engine, &context).unwrap();

    println!("Rendered email:");
    println!("  From: {}", from);
    println!("  Subject: {}", subject);
    println!("  Body:\n{}\n", body);

    // Example 3: Using Template directly for advanced features
    println!("=== Example 3: Template for Advanced Features ===\n");

    let template_str = r#"<!DOCTYPE html>
<html>
<head>
    <title>Order #{{ $execution.order_id }}</title>
</head>
<body>
    <h1>Hello {{ $execution.user_name | uppercase() }}!</h1>
    <p>Your order total is: ${{ $execution.total }}</p>
    <p>Environment: {{ $execution.env }}</p>
</body>
</html>"#;

    let template = Template::new(template_str).unwrap();

    println!("Template analysis:");
    println!("  Number of parts: {}", template.parts().len());
    println!("  Number of expressions: {}", template.expression_count());
    println!("  Expressions:");
    for expr in template.expressions() {
        println!("    - {}", expr.trim());
    }
    println!();

    let result = template.render(&engine, &context).unwrap();
    println!("Rendered HTML:\n{}\n", result);

    // Example 4: Combining both in a real-world scenario
    println!("=== Example 4: Real-World Scenario - HTTP Request ===\n");

    #[derive(Debug, Serialize, Deserialize)]
    struct HttpRequest {
        url: MaybeExpression<String>,
        method: MaybeExpression<String>,
        timeout_ms: MaybeExpression<i64>,
        body: MaybeTemplate,
    }

    let request_config = r#"{
        "url": "{{ $execution.env + \".api.example.com/orders\" }}",
        "method": "POST",
        "timeout_ms": "{{ $input * 1000 }}",
        "body": "User: {{ $execution.user_email }}, Order: {{ $execution.order_id }}, Total: ${{ $execution.total }}"
    }"#;

    let request: HttpRequest = serde_json::from_str(request_config).unwrap();

    println!("HTTP Request Configuration:");
    println!("  url (expression): {:?}", request.url);
    println!("  method (value): {:?}", request.method);
    println!("  timeout_ms (expression): {:?}", request.timeout_ms);
    println!(
        "  body (template): is_template = {}\n",
        request.body.is_template()
    );

    // Resolve all fields
    let url = request.url.resolve(&engine, &context).unwrap();
    let method = request.method.resolve(&engine, &context).unwrap();
    let timeout = request.timeout_ms.resolve(&engine, &context).unwrap();
    let body = request.body.resolve(&engine, &context).unwrap();

    println!("Resolved HTTP Request:");
    println!("  URL: {}", url);
    println!("  Method: {}", method);
    println!("  Timeout: {} ms", timeout);
    println!("  Body: {}\n", body);

    // Example 5: When to use which
    println!("=== Example 5: Decision Guide ===\n");
    println!("Use MaybeExpression<T> when:");
    println!("  ✓ You need a typed value (number, boolean, etc.)");
    println!("  ✓ You have a single expression that returns one value");
    println!("  ✓ You're working with configuration parameters");
    println!("  ✓ You want type safety and validation");
    println!();

    println!("Use MaybeTemplate when:");
    println!("  ✓ You have text with multiple {{ }} expressions");
    println!("  ✓ You're working with templates (HTML, JSON, emails, etc.)");
    println!("  ✓ The result is always a string");
    println!("  ✓ You need to preserve static text between expressions");
    println!();

    println!("Use Template when:");
    println!("  ✓ You want to reuse the same template multiple times");
    println!("  ✓ You need detailed error reporting (line/column)");
    println!("  ✓ You want to inspect the template structure");
    println!("  ✓ Performance matters (caching of parsed structure)");
}
