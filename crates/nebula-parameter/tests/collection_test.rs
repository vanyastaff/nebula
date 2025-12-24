//! Tests for ParameterCollection

use nebula_core::ParameterKey;
use nebula_parameter::core::values::ParameterValues;
use nebula_parameter::prelude::*;
use nebula_parameter::types::TextParameter;
use nebula_value::Value;

/// Helper to create a ParameterKey for tests
fn key(s: &str) -> ParameterKey {
    ParameterKey::new(s).expect("invalid test key")
}

#[test]
fn test_collection_new() {
    let collection = ParameterCollection::new();
    assert!(collection.is_empty());
    assert_eq!(collection.len(), 0);
}

#[test]
fn test_collection_add_single() {
    let mut collection = ParameterCollection::new();

    let param = TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("username")
                .name("Username")
                .description("")
                .build()
                .unwrap(),
        )
        .build();

    collection.add(param);

    assert_eq!(collection.len(), 1);
    assert!(collection.contains(key("username")));
}

#[test]
fn test_collection_with_builder_pattern() {
    let collection = ParameterCollection::new()
        .with(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("username")
                        .name("Username")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        )
        .with(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("email")
                        .name("Email")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        );

    assert_eq!(collection.len(), 2);
    assert!(collection.contains(key("username")));
    assert!(collection.contains(key("email")));
}

#[test]
fn test_collection_get_typed() {
    let mut collection = ParameterCollection::new();

    collection.add(
        TextParameter::builder()
            .metadata(
                ParameterMetadata::builder()
                    .key("test")
                    .name("Test")
                    .description("")
                    .build()
                    .unwrap(),
            )
            .build(),
    );

    // Type-safe access to parameter definition
    let param: Option<&TextParameter> = collection.get(key("test"));
    assert!(param.is_some());

    // Use ParameterValues to store and access actual values
    let mut values = ParameterValues::new();
    values.set(key("test"), Value::text("hello"));

    assert_eq!(
        values
            .get(key("test"))
            .and_then(|v| v.as_text())
            .map(|t| t.as_str()),
        Some("hello")
    );
}

#[test]
fn test_collection_value_access() {
    let mut collection = ParameterCollection::new();

    collection.add(
        TextParameter::builder()
            .metadata(
                ParameterMetadata::builder()
                    .key("test")
                    .name("Test")
                    .description("")
                    .build()
                    .unwrap(),
            )
            .build(),
    );

    // Use ParameterValues for value storage
    let mut values = ParameterValues::new();
    values.set(key("test"), Value::text("hello"));

    let value = values.get(key("test"));
    assert!(value.is_some());

    let value = value.unwrap();
    assert_eq!(value.as_text().unwrap().as_str(), "hello");
}

#[test]
fn test_collection_snapshot_restore() {
    let mut collection = ParameterCollection::new();

    collection.add(
        TextParameter::builder()
            .metadata(
                ParameterMetadata::builder()
                    .key("test")
                    .name("Test")
                    .description("")
                    .build()
                    .unwrap(),
            )
            .build(),
    );

    // Use ParameterValues for value storage and snapshot/restore
    let mut values = ParameterValues::new();
    values.set(key("test"), Value::text("initial"));

    // Take snapshot
    let snapshot = values.snapshot();
    assert_eq!(snapshot.len(), 1);

    // Modify value
    values.set(key("test"), Value::text("modified"));

    // Verify modification
    assert_eq!(
        values.get(key("test")).unwrap().as_text().unwrap().as_str(),
        "modified"
    );

    // Restore
    values.restore(&snapshot);

    // Verify restoration
    assert_eq!(
        values.get(key("test")).unwrap().as_text().unwrap().as_str(),
        "initial"
    );
}

#[test]
fn test_collection_remove() {
    let mut collection = ParameterCollection::new();

    collection.add(
        TextParameter::builder()
            .metadata(
                ParameterMetadata::builder()
                    .key("test")
                    .name("Test")
                    .description("")
                    .build()
                    .unwrap(),
            )
            .build(),
    );

    assert_eq!(collection.len(), 1);

    let removed = collection.remove(key("test"));
    assert!(removed.is_some());
    assert_eq!(collection.len(), 0);
    assert!(!collection.contains(key("test")));
}

#[test]
fn test_collection_keys() {
    let collection = ParameterCollection::new()
        .with(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("key1")
                        .name("Key 1")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        )
        .with(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("key2")
                        .name("Key 2")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        )
        .with(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("key3")
                        .name("Key 3")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        );

    let keys: Vec<_> = collection.keys().map(|k| k.as_str()).collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&"key1"));
    assert!(keys.contains(&"key2"));
    assert!(keys.contains(&"key3"));
}

#[test]
fn test_collection_clear() {
    let mut collection = ParameterCollection::new()
        .with(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("key1")
                        .name("Key 1")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        )
        .with(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("key2")
                        .name("Key 2")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        );

    assert_eq!(collection.len(), 2);

    collection.clear();

    assert_eq!(collection.len(), 0);
    assert!(collection.is_empty());
}
