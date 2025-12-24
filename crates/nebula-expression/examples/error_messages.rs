//! Example demonstrating beautiful error messages with source context
//!
//! This example shows how the template engine provides detailed error messages
//! with source code context and visual highlighting.

use nebula_expression::{EvaluationContext, ExpressionEngine, Template};
use nebula_value::Value;

fn main() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::text("Alice"));

    println!("=== Example 1: Undefined Variable ===\n");
    let template = r#"<html>
  <head>
    <title>{{ $execution.title }}</title>
  </head>
  <body>
    <h1>Hello {{ $undefined_variable }}!</h1>
  </body>
</html>"#;

    match Template::new(template) {
        Ok(tmpl) => match tmpl.render(&engine, &context) {
            Ok(_) => println!("Success!"),
            Err(e) => {
                println!("{}\n", e);
            }
        },
        Err(e) => println!("Parse error: {}\n", e),
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
                println!("{}\n", e);
            }
        },
        Err(e) => println!("Parse error: {}\n", e),
    }

    println!("=== Example 3: Unclosed Expression ===\n");
    let template3 = r#"Line 1
Line 2
Line 3 has {{ unclosed expression
Line 4
Line 5"#;

    match Template::new(template3) {
        Ok(_) => println!("Parsed successfully"),
        Err(e) => {
            println!("{}\n", e);
        }
    }

    println!("=== Example 4: Type Error ===\n");
    context.set_execution_var("count", Value::text("not a number"));

    let template4 = r#"<div>
    <p>Total items: {{ $execution.count * 2 }}</p>
</div>"#;

    match Template::new(template4) {
        Ok(tmpl) => match tmpl.render(&engine, &context) {
            Ok(_) => println!("Success!"),
            Err(e) => {
                println!("{}\n", e);
            }
        },
        Err(e) => println!("Parse error: {}\n", e),
    }

    println!("=== Example 5: Multiline with Good Error ===\n");
    let template5 = r#"<!DOCTYPE html>
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
</html>"#;

    match Template::new(template5) {
        Ok(tmpl) => match tmpl.render(&engine, &context) {
            Ok(_) => println!("Success!"),
            Err(e) => {
                println!("{}\n", e);
            }
        },
        Err(e) => println!("Parse error: {}\n", e),
    }
}
