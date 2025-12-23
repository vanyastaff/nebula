//! Tests for ParameterCollection

use nebula_core::ParameterKey;
use nebula_parameter::prelude::*;
use nebula_parameter::types::TextParameter;
use nebula_value::Text;

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
            .value(Text::from("hello"))
            .build(),
    );

    // Type-safe access
    let param: Option<&TextParameter> = collection.get(key("test"));
    assert!(param.is_some());

    let param = param.unwrap();
    assert_eq!(param.get().map(|t| t.as_str()), Some("hello"));
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
            .value(Text::from("hello"))
            .build(),
    );

    // Type-erased value access
    let value = collection.value(key("test"));
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
            .value(Text::from("initial"))
            .build(),
    );

    // Take snapshot
    let snapshot = collection.snapshot();
    assert_eq!(snapshot.len(), 1);

    // Modify value
    if let Some(p) = collection.get_mut::<TextParameter>(key("test")) {
        let _ = p.set(Text::from("modified"));
    }

    // Verify modification
    assert_eq!(
        collection
            .value(key("test"))
            .unwrap()
            .as_text()
            .unwrap()
            .as_str(),
        "modified"
    );

    // Restore
    collection.restore(&snapshot).unwrap();

    // Verify restoration
    assert_eq!(
        collection
            .value(key("test"))
            .unwrap()
            .as_text()
            .unwrap()
            .as_str(),
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
