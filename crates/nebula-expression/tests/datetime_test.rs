//! Tests for date/time functions

use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_value::Value;

#[test]
fn test_now() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine.evaluate("{{ now() }}", &context).unwrap();
    // Should be a timestamp (integer)
    assert!(result.is_integer());
    let timestamp = result.as_integer().unwrap();
    // Should be a reasonable timestamp (after 2020)
    assert!(timestamp > 1577836800); // 2020-01-01
}

#[test]
fn test_now_iso() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine.evaluate("{{ now_iso() }}", &context).unwrap();
    // Should be a string
    assert!(result.is_text());
    let iso = result.as_str().unwrap();
    // Should contain typical ISO 8601 parts
    assert!(iso.contains("T"));
    assert!(iso.contains(":"));
}

#[test]
fn test_format_date() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    // Fixed timestamp: 2024-01-01 00:00:00 UTC
    context.set_input(Value::integer(1704067200));

    let result = engine
        .evaluate(r#"{{ $input | format_date("YYYY-MM-DD") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("2024-01-01"));

    let result = engine
        .evaluate(r#"{{ $input | format_date("DD.MM.YYYY") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("01.01.2024"));

    let result = engine
        .evaluate(r#"{{ $input | format_date("HH:mm:ss") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("00:00:00"));
}

#[test]
fn test_parse_date() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine
        .evaluate(r#"{{ parse_date("2024-01-01 00:00:00") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1704067200));

    let result = engine
        .evaluate(r#"{{ parse_date("2024-01-01") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1704067200)); // Midnight UTC
}

#[test]
fn test_date_add() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    // 2024-01-01 00:00:00 UTC
    context.set_input(Value::integer(1704067200));

    // Add 7 days
    let result = engine
        .evaluate(r#"{{ $input | date_add(7, "days") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1704672000)); // 2024-01-08

    // Add 2 hours
    let result = engine
        .evaluate(r#"{{ $input | date_add(2, "hours") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1704074400));
}

#[test]
fn test_date_subtract() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    // 2024-01-08 00:00:00 UTC
    context.set_input(Value::integer(1704672000));

    // Subtract 7 days
    let result = engine
        .evaluate(r#"{{ $input | date_subtract(7, "days") }}"#, &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1704067200)); // 2024-01-01
}

#[test]
fn test_date_diff() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();

    context.set_input(Value::Object(
        nebula_value::Object::new()
            .insert("date1".to_string(), serde_json::json!(1704672000)) // 2024-01-08
            .insert("date2".to_string(), serde_json::json!(1704067200)), // 2024-01-01
    ));

    let result = engine
        .evaluate(
            r#"{{ date_diff($input.date1, $input.date2, "days") }}"#,
            &context,
        )
        .unwrap();
    assert_eq!(result.as_integer(), Some(7));

    let result = engine
        .evaluate(
            r#"{{ date_diff($input.date1, $input.date2, "hours") }}"#,
            &context,
        )
        .unwrap();
    assert_eq!(result.as_integer(), Some(168)); // 7 * 24
}

#[test]
fn test_date_year() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(1704067200)); // 2024-01-01 00:00:00

    let result = engine
        .evaluate("{{ $input | date_year() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(2024));
}

#[test]
fn test_date_month() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(1704067200)); // 2024-01-01 00:00:00

    let result = engine
        .evaluate("{{ $input | date_month() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1));
}

#[test]
fn test_date_day() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(1704067200)); // 2024-01-01 00:00:00

    let result = engine
        .evaluate("{{ $input | date_day() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1));
}

#[test]
fn test_date_hour() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(1704067200)); // 2024-01-01 00:00:00

    let result = engine
        .evaluate("{{ $input | date_hour() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(0));
}

#[test]
fn test_date_minute() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(1704067200)); // 2024-01-01 00:00:00

    let result = engine
        .evaluate("{{ $input | date_minute() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(0));
}

#[test]
fn test_date_second() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(1704067200)); // 2024-01-01 00:00:00

    let result = engine
        .evaluate("{{ $input | date_second() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(0));
}

#[test]
fn test_date_day_of_week() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    // 2024-01-01 is Monday
    context.set_input(Value::integer(1704067200)); // 2024-01-01 00:00:00

    let result = engine
        .evaluate("{{ $input | date_day_of_week() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(1)); // Monday
}

#[test]
fn test_date_pipeline_operations() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(1704067200)); // 2024-01-01

    // Complex pipeline: add 6 months, add 15 days, format
    let result = engine
        .evaluate(
            r#"{{ $input | date_add(180, "days") | format_date("YYYY-MM-DD") }}"#,
            &context,
        )
        .unwrap();
    assert_eq!(result.as_str(), Some("2024-06-29"));
}
