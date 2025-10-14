//! Integration test: Real-world workflow scenario
//!
//! Tests how nebula-value would be used in an actual workflow engine

use nebula_value::{Array, Object, Value};
use std::convert::TryFrom;

// Helper to parse JSON strings into Value for tests
#[allow(dead_code)]
fn parse_json(json_str: &str) -> Value {
    json_str.parse().expect("valid JSON")
}

#[test]
fn test_workflow_state_management() {
    // Scenario: A workflow tracks user processing state
    let mut workflow_state = Object::new();

    // Add user information
    workflow_state = workflow_state.insert("user_id".to_string(), Value::integer(12345));
    workflow_state = workflow_state.insert("username".to_string(), Value::text("alice"));
    workflow_state = workflow_state.insert("email".to_string(), Value::text("alice@example.com"));

    // Add status
    workflow_state = workflow_state.insert("status".to_string(), Value::text("processing"));

    // Add metadata
    let metadata = Object::new()
        .insert(
            "created_at".to_string(),
            Value::text("2024-01-01T00:00:00Z"),
        )
        .insert(
            "updated_at".to_string(),
            Value::text("2024-01-01T00:01:00Z"),
        )
        .insert("version".to_string(), Value::integer(1));
    workflow_state = workflow_state.insert("metadata".to_string(), Value::Object(metadata));

    // Verify state
    assert_eq!(workflow_state.get("user_id"), Some(&Value::integer(12345)));
    assert_eq!(workflow_state.get("username"), Some(&Value::text("alice")));
    assert!(workflow_state.contains_key("metadata"));

    // Update status
    workflow_state = workflow_state.insert("status".to_string(), Value::text("completed"));
    assert_eq!(
        workflow_state.get("status"),
        Some(&Value::text("completed"))
    );

    // Clone for history
    let workflow_snapshot = workflow_state.clone();
    assert_eq!(workflow_snapshot.len(), workflow_state.len());
}

#[test]
fn test_workflow_array_processing() {
    // Scenario: Process a list of tasks
    let task1 = Value::Object(
        Object::new()
            .insert("id".to_string(), Value::integer(1))
            .insert("name".to_string(), Value::text("task1"))
            .insert("completed".to_string(), Value::boolean(false)),
    );
    let task2 = Value::Object(
        Object::new()
            .insert("id".to_string(), Value::integer(2))
            .insert("name".to_string(), Value::text("task2"))
            .insert("completed".to_string(), Value::boolean(false)),
    );
    let task3 = Value::Object(
        Object::new()
            .insert("id".to_string(), Value::integer(3))
            .insert("name".to_string(), Value::text("task3"))
            .insert("completed".to_string(), Value::boolean(false)),
    );

    let tasks = Array::from_vec(vec![task1, task2, task3]);

    assert_eq!(tasks.len(), 3);

    // Mark first task as completed
    let _task1 = tasks.get(0).unwrap().clone();
    let updated_task = Value::Object(
        Object::new()
            .insert("id".to_string(), Value::integer(1))
            .insert("name".to_string(), Value::text("task1"))
            .insert("completed".to_string(), Value::boolean(true)),
    );

    // Create new array with updated task (persistent data structure)
    let updated_tasks = tasks.push(updated_task);
    assert_eq!(updated_tasks.len(), 4);

    // Original array unchanged
    assert_eq!(tasks.len(), 3);
}

#[test]
fn test_value_arithmetic_in_workflow() {
    // Scenario: Calculate total cost from items
    let item_costs = vec![Value::integer(100), Value::integer(250), Value::integer(75)];

    let mut total = Value::integer(0);
    for cost in item_costs {
        total = total.add(&cost).expect("Failed to add costs");
    }

    // Verify total
    if let Value::Integer(sum) = total {
        assert_eq!(sum.value(), 425);
    } else {
        panic!("Expected integer result");
    }

    // Apply tax (10%)
    let tax_rate = Value::float(0.1);
    let base = Value::integer(425);
    let tax = base.mul(&tax_rate).expect("Failed to calculate tax");

    if let Value::Float(tax_amount) = tax {
        assert_eq!(tax_amount.value(), 42.5);
    }
}

#[test]
fn test_nested_object_access() {
    // Scenario: Access deeply nested configuration
    let config = Object::from_iter(vec![
        (
            "app".to_string(),
            parse_json(
                r#"{
                "name": "workflow-engine",
                "version": "1.0.0",
                "features": {
                    "logging": true,
                    "monitoring": true,
                    "cache_size": 1000
                }
            }"#,
            ),
        ),
        (
            "database".to_string(),
            parse_json(
                r#"{
                "host": "localhost",
                "port": 5432,
                "pool_size": 10
            }"#,
            ),
        ),
    ]);

    // Access nested values
    assert_eq!(config.get("app").is_some(), true);
    assert_eq!(config.get("database").is_some(), true);

    // Clone preserves structure
    let config_copy = config.clone();
    assert_eq!(config.len(), config_copy.len());
}

#[test]
fn test_value_merging_in_workflow() {
    // Scenario: Merge default config with user config
    let default_config = Value::Object(Object::from_iter(vec![
        ("timeout".to_string(), Value::integer(30)),
        ("retries".to_string(), Value::integer(3)),
        ("log_level".to_string(), Value::text("info")),
    ]));

    let user_config = Value::Object(Object::from_iter(vec![
        ("timeout".to_string(), Value::integer(60)), // Override
        ("custom_option".to_string(), Value::boolean(true)), // New
    ]));

    // Merge (user config overrides defaults)
    let merged = default_config
        .merge(&user_config)
        .expect("Failed to merge configs");

    if let Value::Object(obj) = merged {
        // User override applied
        assert_eq!(obj.get("timeout"), Some(&Value::integer(60)));
        // Default retained
        assert_eq!(obj.get("retries"), Some(&Value::integer(3)));
        // User addition included
        assert_eq!(obj.get("custom_option"), Some(&Value::boolean(true)));
    } else {
        panic!("Expected object result");
    }
}

#[test]
fn test_type_conversion_workflow() {
    // Scenario: Convert values from external input
    let _user_age = Value::text("25");

    // Would need parsing in real scenario, but test conversion
    let age_value = Value::integer(25);
    let age_i64 = i64::try_from(age_value).expect("Failed to convert to i64");
    assert_eq!(age_i64, 25);

    // Boolean conversion
    let is_active = Value::boolean(true);
    let active_bool = bool::try_from(is_active).expect("Failed to convert to bool");
    assert_eq!(active_bool, true);

    // String conversion
    let username = Value::text("alice");
    let username_str = String::try_from(username).expect("Failed to convert to String");
    assert_eq!(username_str, "alice");
}

#[test]
#[cfg(feature = "serde")]
fn test_json_roundtrip_workflow() {
    // Scenario: Store/load workflow state as JSON
    let state = Value::Object(Object::from_iter(vec![
        ("workflow_id".to_string(), Value::text("wf-123")),
        ("status".to_string(), Value::text("running")),
        ("progress".to_string(), Value::float(0.75)),
        (
            "tasks".to_string(),
            parse_json(
                r#"[
                {"name": "init", "done": true},
                {"name": "process", "done": true},
                {"name": "finalize", "done": false}
            ]"#,
            ),
        ),
    ]));

    // Serialize to JSON
    let json = serde_json::to_string(&state).expect("Failed to serialize");

    // Deserialize back
    let restored: Value = serde_json::from_str(&json).expect("Failed to deserialize");

    // Verify structure is preserved
    if let Value::Object(obj) = restored {
        assert_eq!(obj.get("workflow_id"), Some(&Value::text("wf-123")));
        assert_eq!(obj.get("status"), Some(&Value::text("running")));
        assert!(obj.contains_key("tasks"));
    } else {
        panic!("Expected object result");
    }
}

#[test]
fn test_error_handling_workflow() {
    // Scenario: Handle errors gracefully in workflow

    // Division by zero should return error
    let a = Value::integer(100);
    let zero = Value::integer(0);
    assert!(a.div(&zero).is_err());

    // Type mismatch should return error
    let num = Value::integer(42);
    let text = Value::text("hello");
    assert!(num.add(&text).is_err());

    // Invalid conversion should return error
    let bool_val = Value::boolean(true);
    let result = i64::try_from(bool_val);
    assert!(result.is_err());
}

#[test]
fn test_clone_efficiency() {
    // Scenario: Clone large structures efficiently (structural sharing)
    let large_array = Array::from_vec((0..1000).map(|i| Value::integer(i as i64)).collect());

    // Clone should be cheap (structural sharing)
    let cloned = large_array.clone();

    // Both should have same data
    assert_eq!(large_array.len(), cloned.len());

    // Modifying clone doesn't affect original
    let modified = cloned.push(Value::integer(9999));
    assert_eq!(modified.len(), 1001);
    assert_eq!(large_array.len(), 1000); // Original unchanged
}
