//! Integration tests for nebula-expression

use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_value::{Float, Integer, Value};
use serde_json::json;

#[test]
fn test_basic_arithmetic() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    assert_eq!(
        engine.evaluate("{{ 2 + 2 }}", &context).unwrap(),
        Value::integer(4)
    );
    assert_eq!(
        engine.evaluate("{{ 10 - 3 }}", &context).unwrap(),
        Value::integer(7)
    );
    assert_eq!(
        engine.evaluate("{{ 3 * 4 }}", &context).unwrap(),
        Value::integer(12)
    );
    assert_eq!(
        engine.evaluate("{{ 15 / 3 }}", &context).unwrap(),
        Value::integer(5)
    );
}

#[test]
fn test_string_operations() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine
        .evaluate("{{ \"hello\" + \" \" + \"world\" }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("hello world"));

    let result = engine
        .evaluate("{{ \"HELLO\" | lowercase() }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("hello"));

    let result = engine
        .evaluate("{{ \"hello\" | uppercase() }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("HELLO"));
}

#[test]
fn test_comparisons() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    assert_eq!(
        engine.evaluate("{{ 5 > 3 }}", &context).unwrap(),
        Value::boolean(true)
    );
    assert_eq!(
        engine.evaluate("{{ 5 < 3 }}", &context).unwrap(),
        Value::boolean(false)
    );
    assert_eq!(
        engine.evaluate("{{ 5 == 5 }}", &context).unwrap(),
        Value::boolean(true)
    );
    assert_eq!(
        engine.evaluate("{{ 5 != 3 }}", &context).unwrap(),
        Value::boolean(true)
    );
}

#[test]
fn test_logical_operations() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    assert_eq!(
        engine.evaluate("{{ true && true }}", &context).unwrap(),
        Value::boolean(true)
    );
    assert_eq!(
        engine.evaluate("{{ true && false }}", &context).unwrap(),
        Value::boolean(false)
    );
    assert_eq!(
        engine.evaluate("{{ true || false }}", &context).unwrap(),
        Value::boolean(true)
    );
    assert_eq!(
        engine.evaluate("{{ !true }}", &context).unwrap(),
        Value::boolean(false)
    );
}

#[test]
fn test_conditionals() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine
        .evaluate("{{ if true then \"yes\" else \"no\" }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("yes"));

    let result = engine
        .evaluate("{{ if false then \"yes\" else \"no\" }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("no"));

    let result = engine
        .evaluate("{{ if 5 > 3 then 100 else 200 }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(100)));
}

#[test]
fn test_input_variable() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();

    context.set_input(Value::Object(
        nebula_value::Object::new()
            .insert("name".to_string(), json!("Alice"))
            .insert("age".to_string(), json!(30)),
    ));

    let result = engine.evaluate("{{ $input.name }}", &context).unwrap();
    assert_eq!(result.as_str(), Some("Alice"));

    let result = engine.evaluate("{{ $input.age }}", &context).unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(30)));
}

#[test]
fn test_node_data() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();

    context.set_node_data(
        "http",
        Value::Object(nebula_value::Object::new().insert(
            "response".to_string(),
            json!({
                "statusCode": 200,
                "body": "Success"
            }),
        )),
    );

    let result = engine
        .evaluate("{{ $node.http.response.statusCode }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(200)));

    let result = engine
        .evaluate("{{ $node.http.response.body }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("Success"));
}

#[test]
fn test_math_functions() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine.evaluate("{{ -5 | abs() }}", &context).unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(5)));

    let result = engine.evaluate("{{ 3.7 | floor() }}", &context).unwrap();
    assert_eq!(result.as_float(), Some(&Float::new(3.0)));

    let result = engine.evaluate("{{ 3.2 | ceil() }}", &context).unwrap();
    assert_eq!(result.as_float(), Some(&Float::new(4.0)));

    let result = engine.evaluate("{{ 16 | sqrt() }}", &context).unwrap();
    assert_eq!(result.as_float(), Some(&Float::new(4.0)));
}

#[test]
fn test_string_functions() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine
        .evaluate("{{ \"hello\" | length() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(5)));

    let result = engine
        .evaluate("{{ \"  hi  \" | trim() }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("hi"));

    let result = engine
        .evaluate("{{ \"hello\" | substring(0, 3) }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("hel"));
}

#[test]
fn test_pipeline_operations() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine
        .evaluate("{{ \"HELLO WORLD\" | lowercase() | length() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(11)));

    let result = engine
        .evaluate("{{ 3.14159 | round(2) | to_string() }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("3.14"));
}

#[test]
fn test_type_conversion() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let result = engine
        .evaluate("{{ 123 | to_string() }}", &context)
        .unwrap();
    assert_eq!(result.as_str(), Some("123"));

    let result = engine
        .evaluate("{{ \"456\" | to_number() }}", &context)
        .unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(456)));

    let result = engine
        .evaluate("{{ \"true\" | to_boolean() }}", &context)
        .unwrap();
    assert_eq!(result.as_boolean(), Some(true));
}

#[test]
fn test_caching() {
    let engine = ExpressionEngine::with_cache_size(100);
    let context = EvaluationContext::new();

    // Evaluate the same expression multiple times
    // With caching, this should be faster than without
    for _ in 0..5 {
        let result = engine.evaluate("{{ 2 + 2 }}", &context).unwrap();
        assert_eq!(result.as_integer(), Some(Integer::new(4)));
    }

    // Just verify that caching doesn't break functionality
    let result = engine.evaluate("{{ 3 + 3 }}", &context).unwrap();
    assert_eq!(result.as_integer(), Some(Integer::new(6)));
}
