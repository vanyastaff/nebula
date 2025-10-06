//! Example demonstrating date/time functions
//!
//! Run with: cargo run --example datetime_usage

use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_value::Value;

fn main() {
    println!("=== Nebula Expression - Date/Time Functions ===\n");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    // Example 1: Get current time
    println!("Example 1: Current time");
    let result = engine.evaluate("{{ now() }}", &context).unwrap();
    println!("  Current timestamp: {}", result);

    let result = engine.evaluate("{{ now_iso() }}", &context).unwrap();
    println!("  Current ISO time: {}", result);

    // Example 2: Format dates
    println!("\nExample 2: Date formatting");
    let result = engine
        .evaluate("{{ now() | format_date(\"YYYY-MM-DD\") }}", &context)
        .unwrap();
    println!("  Date (YYYY-MM-DD): {}", result);

    let result = engine
        .evaluate(
            "{{ now() | format_date(\"DD.MM.YYYY HH:mm:ss\") }}",
            &context,
        )
        .unwrap();
    println!("  DateTime (DD.MM.YYYY HH:mm:ss): {}", result);

    // Example 3: Parse dates
    println!("\nExample 3: Parsing dates");
    let result = engine
        .evaluate("{{ parse_date(\"2024-01-15 10:30:00\") }}", &context)
        .unwrap();
    println!("  Parsed timestamp: {}", result);

    // Example 4: Date arithmetic
    println!("\nExample 4: Date arithmetic");
    let result = engine
        .evaluate(
            "{{ now() | date_add(7, \"days\") | format_date(\"YYYY-MM-DD\") }}",
            &context,
        )
        .unwrap();
    println!("  7 days from now: {}", result);

    let result = engine
        .evaluate(
            "{{ now() | date_subtract(30, \"days\") | format_date(\"YYYY-MM-DD\") }}",
            &context,
        )
        .unwrap();
    println!("  30 days ago: {}", result);

    let result = engine
        .evaluate(
            "{{ now() | date_add(2, \"hours\") | format_date(\"HH:mm\") }}",
            &context,
        )
        .unwrap();
    println!("  2 hours from now: {}", result);

    // Example 5: Date difference
    println!("\nExample 5: Date difference");

    let mut context = EvaluationContext::new();
    context.set_input(Value::Object(
        nebula_value::Object::new()
            .insert("start".to_string(), serde_json::json!("2024-01-01"))
            .insert("end".to_string(), serde_json::json!("2024-12-31")),
    ));

    let result = engine
        .evaluate(
            r#"{{ date_diff($input.end | parse_date(), $input.start | parse_date(), "days") }}"#,
            &context,
        )
        .unwrap();
    println!("  Days between 2024-01-01 and 2024-12-31: {}", result);

    // Example 6: Extract date components
    println!("\nExample 6: Extract date components");
    let mut context = EvaluationContext::new();
    context.set_input(Value::text("2024-06-15 14:30:45"));

    let result = engine
        .evaluate("{{ parse_date($input) | date_year() }}", &context)
        .unwrap();
    println!("  Year: {}", result);

    let result = engine
        .evaluate("{{ parse_date($input) | date_month() }}", &context)
        .unwrap();
    println!("  Month: {}", result);

    let result = engine
        .evaluate("{{ parse_date($input) | date_day() }}", &context)
        .unwrap();
    println!("  Day: {}", result);

    let result = engine
        .evaluate("{{ parse_date($input) | date_hour() }}", &context)
        .unwrap();
    println!("  Hour: {}", result);

    let result = engine
        .evaluate("{{ parse_date($input) | date_minute() }}", &context)
        .unwrap();
    println!("  Minute: {}", result);

    let result = engine
        .evaluate("{{ parse_date($input) | date_second() }}", &context)
        .unwrap();
    println!("  Second: {}", result);

    // Example 7: Day of week
    println!("\nExample 7: Day of week");
    let result = engine
        .evaluate(
            "{{ parse_date(\"2024-06-15\") | date_day_of_week() }}",
            &context,
        )
        .unwrap();
    println!("  Day of week (0=Sunday, 6=Saturday): {}", result);

    // Example 8: Complex date manipulations
    println!("\nExample 8: Complex date manipulation");
    context.set_input(Value::integer(1609459200)); // 2021-01-01 00:00:00 UTC

    let result = engine
        .evaluate(
            r#"{{ $input | date_add(6, "months") | date_add(15, "days") | format_date("YYYY-MM-DD") }}"#,
            &context,
        )
        .unwrap();
    println!("  6 months + 15 days from 2021-01-01: {}", result);

    // Example 9: Check if date is in range
    println!("\nExample 9: Date comparison");
    let result = engine
        .evaluate(
            r#"{{ if date_diff(now(), parse_date("2024-01-01"), "days") > 0 then "After 2024" else "Before 2024" }}"#,
            &context,
        )
        .unwrap();
    println!("  Is today after 2024-01-01? {}", result);

    // Example 10: Working with timestamps from workflow
    println!("\nExample 10: Workflow timestamp formatting");
    let mut context = EvaluationContext::new();
    context.set_node_data(
        "webhook",
        Value::Object(nebula_value::Object::new().insert(
            "timestamp".to_string(),
            serde_json::json!(1704067200), // 2024-01-01 00:00:00
        )),
    );

    let result = engine
        .evaluate(
            r#"{{ $node.webhook.timestamp | format_date("DD MMM YYYY") }}"#,
            &context,
        )
        .unwrap();
    println!("  Formatted webhook timestamp: {}", result);

    println!("\n=== All date/time examples completed! ===");
}
