//! Integration test: Cross-module interactions
//!
//! Tests how different nebula-value modules interact

use nebula_value::{Array, Bytes, Float, Integer, Object, Text, Value};
use serde_json::json;

#[test]
fn test_scalar_to_value_integration() {
    // Create scalars
    let int = Integer::new(42);
    let float = Float::new(3.14);
    let text = Text::from_str("hello");
    let bytes = Bytes::new(vec![1, 2, 3]);

    // Convert to Value
    let values = vec![
        Value::Integer(int),
        Value::Float(float),
        Value::Text(text),
        Value::Bytes(bytes),
    ];

    // All should be different types
    assert!(values[0].is_integer());
    assert!(values[1].is_float());
    assert!(values[2].is_text());
    assert!(values[3].is_bytes());

    // Clone works for all
    for val in values {
        let _ = val.clone();
    }
}

#[test]
fn test_collection_with_mixed_types() {
    // Array with mixed types
    let mixed_array = Array::from_vec(vec![
        json!(42),
        json!(3.14),
        json!("hello"),
        json!(true),
        json!(null),
    ]);

    assert_eq!(mixed_array.len(), 5);

    // Iterate over mixed types
    let mut count = 0;
    for _ in mixed_array.iter() {
        count += 1;
    }
    assert_eq!(count, 5);
}

#[test]
fn test_nested_collections() {
    // Object containing arrays and objects
    let nested = Object::from_iter(vec![
        ("numbers".to_string(), json!([1, 2, 3, 4, 5])),
        (
            "config".to_string(),
            json!({
                "enabled": true,
                "level": 3
            }),
        ),
        (
            "tags".to_string(),
            json!(["rust", "workflow", "automation"]),
        ),
    ]);

    assert_eq!(nested.len(), 3);
    assert!(nested.contains_key("numbers"));
    assert!(nested.contains_key("config"));
    assert!(nested.contains_key("tags"));

    // Clone preserves nesting
    let cloned = nested.clone();
    assert_eq!(cloned.len(), nested.len());
}

#[test]
fn test_operations_across_types() {
    // Integer + Integer
    let i1 = Value::integer(10);
    let i2 = Value::integer(20);
    let sum_ii = i1.add(&i2).unwrap();
    assert!(sum_ii.is_integer());

    // Integer + Float (coercion)
    let i = Value::integer(10);
    let f = Value::float(2.5);
    let sum_if = i.add(&f).unwrap();
    assert!(sum_if.is_float());

    // Text + Text (concatenation)
    let t1 = Value::text("Hello ");
    let t2 = Value::text("World");
    let concat = t1.add(&t2).unwrap();
    assert!(concat.is_text());
}

#[test]
fn test_comparison_across_types() {
    // Same type comparison
    let i1 = Value::integer(10);
    let i2 = Value::integer(20);
    assert!(!i1.eq(&i2));
    assert!(i1.eq(&Value::integer(10)));

    // Different type comparison
    let int_val = Value::integer(10);
    let text_val = Value::text("10");
    assert!(!int_val.eq(&text_val)); // Different types not equal
}

#[test]
fn test_builder_with_limits() {
    use nebula_value::collections::array::ArrayBuilder;
    use nebula_value::collections::object::ObjectBuilder;
    use nebula_value::core::limits::ValueLimits;

    // Create strict limits
    let limits = ValueLimits {
        max_array_length: 5,
        max_object_keys: 3,
        max_string_bytes: 100,
        max_bytes_length: 1024,
        max_nesting_depth: 10,
    };

    // Array builder with limits
    let array = ArrayBuilder::new()
        .with_limits(limits.clone())
        .try_push(json!(1))
        .unwrap()
        .try_push(json!(2))
        .unwrap()
        .try_push(json!(3))
        .unwrap()
        .build()
        .unwrap();

    assert_eq!(array.len(), 3);

    // Object builder with limits
    let object = ObjectBuilder::new()
        .with_limits(limits)
        .try_insert("a", json!(1))
        .unwrap()
        .try_insert("b", json!(2))
        .unwrap()
        .build()
        .unwrap();

    assert_eq!(object.len(), 2);
}

#[test]
#[cfg(feature = "serde")]
fn test_serde_integration_all_types() {
    use std::convert::TryFrom;

    // Create value with all types
    let complex = Value::Object(Object::from_iter(vec![
        ("null".to_string(), json!(null)),
        ("bool".to_string(), json!(true)),
        ("integer".to_string(), json!(42)),
        ("float".to_string(), json!(3.14)),
        ("text".to_string(), json!("hello")),
        ("array".to_string(), json!([1, 2, 3])),
        ("object".to_string(), json!({"key": "value"})),
    ]));

    // Serialize
    let json_str = serde_json::to_string(&complex).unwrap();

    // Deserialize
    let restored: Value = serde_json::from_str(&json_str).unwrap();

    // Verify all types preserved
    if let Value::Object(obj) = restored {
        assert!(obj.get("null").is_some());
        assert!(obj.get("bool").is_some());
        assert!(obj.get("integer").is_some());
        assert!(obj.get("float").is_some());
        assert!(obj.get("text").is_some());
        assert!(obj.get("array").is_some());
        assert!(obj.get("object").is_some());
    } else {
        panic!("Expected object");
    }
}

#[test]
fn test_error_propagation() {
    // Errors should propagate correctly

    // Arithmetic error
    let result = Value::integer(10).div(&Value::integer(0));
    assert!(result.is_err());

    // Type error
    let result = Value::integer(10).add(&Value::text("hello"));
    assert!(result.is_err());

    // Conversion error
    let result = i64::try_from(Value::text("not a number"));
    assert!(result.is_err());
}

#[test]
fn test_persistent_data_structures() {
    // Test that mutations don't affect originals

    let original_array = Array::from_vec(vec![json!(1), json!(2), json!(3)]);
    let modified_array = original_array.push(json!(4));

    // Original unchanged
    assert_eq!(original_array.len(), 3);
    assert_eq!(modified_array.len(), 4);

    let original_object = Object::from_iter(vec![("a".to_string(), json!(1))]);
    let modified_object = original_object.insert("b".to_string(), json!(2));

    // Original unchanged
    assert_eq!(original_object.len(), 1);
    assert_eq!(modified_object.len(), 2);
}

#[test]
fn test_hash_and_equality() {
    use nebula_value::core::hash::HashableValueExt;
    use std::collections::HashMap;

    // Test that hashable values work in HashMap
    let mut map = HashMap::new();

    let key1 = Value::integer(42).hashable();
    let key2 = Value::text("key").hashable();

    map.insert(key1, "value1");
    map.insert(key2, "value2");

    // Lookup with same values
    let lookup1 = Value::integer(42).hashable();
    let lookup2 = Value::text("key").hashable();

    assert_eq!(map.get(&lookup1), Some(&"value1"));
    assert_eq!(map.get(&lookup2), Some(&"value2"));

    // NaN handling
    let nan1 = Value::float(f64::NAN).hashable();
    let nan2 = Value::float(f64::NAN).hashable();

    map.insert(nan1.clone(), "nan_value");

    // All NaN values should hash to same value
    assert_eq!(map.get(&nan2), Some(&"nan_value"));
}

#[test]
fn test_display_integration() {
    // Test Display implementations

    let int = Value::integer(42);
    assert_eq!(int.to_string(), "42");

    let float = Value::float(3.14);
    assert_eq!(float.to_string(), "3.14");

    let text = Value::text("hello");
    assert_eq!(text.to_string(), "hello");

    let bool_val = Value::boolean(true);
    assert_eq!(bool_val.to_string(), "true");

    let null = Value::null();
    assert_eq!(null.to_string(), "null");
}

#[test]
fn test_conversion_chain() {
    // Test chaining conversions

    // i64 -> Value -> i64
    let original: i64 = 42;
    let value = Value::integer(original);
    let back = i64::try_from(value).unwrap();
    assert_eq!(back, original);

    // String -> Value -> String
    let original = "hello".to_string();
    let value = Value::text(original.clone());
    let back = String::try_from(value).unwrap();
    assert_eq!(back, original);

    // Vec<u8> -> Value -> Vec<u8>
    let original = vec![1, 2, 3, 4, 5];
    let value = Value::bytes(original.clone());
    let back = Vec::<u8>::try_from(value).unwrap();
    assert_eq!(back, original);
}
