//! Example demonstrating beautiful error messages with source context
//!
//! This example shows how the template engine provides detailed error messages
//! with source code context and visual highlighting. Parse errors carry a
//! structured `Position`; rendering the caret view is the caller's job via
//! `ErrorFormatter`.

#![expect(
    clippy::print_stdout,
    reason = "example: printed output is the demonstration"
)]

use nebula_expression::{
    EvaluationContext, ExpressionEngine, ExpressionError, Template, error_formatter::ErrorFormatter,
};
use serde_json::Value;

/// Render a parse failure with source context; fall back to `Display` for
/// errors that carry no position (raw-grammar failures).
fn render_parse_error(source: &str, error: &ExpressionError) -> String {
    match error {
        ExpressionError::ParseError {
            position: Some(position),
            message,
        } => ErrorFormatter::new(source, *position, message).format(),
        _ => error.to_string(),
    }
}

fn main() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::String("Alice".to_string()));

    println!("=== Example 1: Undefined Variable ===\n");
    let template = r"<html>
  <head>
    <title>{{ $execution.title }}</title>
  </head>
  <body>
    <h1>Hello {{ $undefined_variable }}!</h1>
  </body>
</html>";

    match Template::new(template) {
        Ok(tmpl) => match tmpl.render(&engine, &context) {
            Ok(_) => println!("Success!"),
            Err(e) => {
                println!("{e}\n");
            },
        },
        Err(e) => println!("Parse error: {e}\n"),
    }

    println!("=== Example 2: Invalid Function ===\n");
    let template2 = r#"{
  "name": "{{ $input }}",
  "upper": "{{ $input | uppercase() }}",
  "invalid": "{{ $input | nonexistent_function() }}"
}"#;

    match Template::new(template2) {
        Ok(tmpl) => match tmpl.render(&engine, &context) {
            Ok(_) => println!("Success!"),
            Err(e) => {
                println!("{e}\n");
            },
        },
        Err(e) => println!("Parse error: {e}\n"),
    }

    println!("=== Example 3: Unclosed Expression ===\n");
    let template3 = r"Line 1
Line 2
Line 3 has {{ unclosed expression
Line 4
Line 5";

    match Template::new(template3) {
        Ok(_) => println!("Parsed successfully"),
        Err(e) => {
            println!("{}\n", render_parse_error(template3, &e));
        },
    }

    println!("=== Example 4: Type Error ===\n");
    context.set_execution_var("count", Value::String("not a number".to_string()));

    let template4 = r"<div>
    <p>Total items: {{ $execution.count * 2 }}</p>
</div>";

    match Template::new(template4) {
        Ok(tmpl) => match tmpl.render(&engine, &context) {
            Ok(_) => println!("Success!"),
            Err(e) => {
                println!("{e}\n");
            },
        },
        Err(e) => println!("Parse error: {e}\n"),
    }

    println!("=== Example 5: Multiline with Good Error ===\n");
    let template5 = r"<!DOCTYPE html>
<html>
<head>
    <title>My Page</title>
</head>
<body>
    <header>
        <h1>{{ $execution.page_title }}</h1>
    </header>
    <main>
        <p>Content goes here</p>
    </main>
</body>
</html>";

    match Template::new(template5) {
        Ok(tmpl) => match tmpl.render(&engine, &context) {
            Ok(_) => println!("Success!"),
            Err(e) => {
                println!("{e}\n");
            },
        },
        Err(e) => println!("Parse error: {e}\n"),
    }
}
