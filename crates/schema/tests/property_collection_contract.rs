//! Property collection rules preserve the existing list validation contract.

use std::assert_matches;

use nebula_schema::{AuthoredValue, HasSchema, Property, RequiredMode, Schema, ValidationReport};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Schema, Deserialize)]
struct BoundedBytes {
    #[property(validate(items(min = 1, max = 8), unique))]
    values: Option<Vec<u8>>,
}

#[derive(Schema, Deserialize)]
struct EmptyAllowed {
    #[property(validate(items(min = 0, max = 2), unique))]
    values: Option<Vec<u8>>,
}

#[derive(Schema, Deserialize)]
struct MinimumOnly {
    #[property(validate(items(min = 2)))]
    values: Option<Vec<String>>,
}

#[derive(Schema, Deserialize)]
struct MaximumOnly {
    #[property(validate(items(max = 4294967295)))]
    values: Option<Vec<u8>>,
}

#[derive(Schema, Deserialize)]
struct UniqueOnly {
    #[property(validate(unique))]
    values: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Schema, Deserialize)]
struct Item {
    id: u8,
    #[property(validate(non_empty))]
    name: String,
}

#[derive(Schema, Deserialize)]
struct NestedItems {
    #[property(validate(items(min = 1, max = 2), unique))]
    values: Option<Vec<Item>>,
}

#[derive(Schema, Deserialize)]
struct RequiredItems {
    #[property(validate(items(min = 0, max = 8), unique))]
    values: Vec<u8>,
}

#[derive(Schema, Deserialize)]
struct ExplicitRequiredItems {
    #[property(input(required), validate(items(min = 0, max = 8), unique))]
    values: Option<Vec<u8>>,
}

fn resolve<T: HasSchema>(value: Value) -> Result<nebula_schema::ResolvedValues, ValidationReport> {
    T::schema()?
        .validate(AuthoredValue::from_data(value).expect("literal input"))?
        .resolve_data()
}

fn rejects<T: HasSchema>(value: Value, code: &str, path: &str) {
    let report = resolve::<T>(value).expect_err("invalid collection must fail before typed decode");
    assert!(
        report
            .errors()
            .any(|error| error.code() == code && error.path().to_string() == path),
        "{report:?}"
    );
}

#[test]
fn count_bounds_preserve_optional_absence_and_accept_both_boundaries() {
    let missing = resolve::<BoundedBytes>(json!({}))
        .expect("optional absence")
        .into_typed::<BoundedBytes>()
        .expect("decode");
    assert_eq!(missing.values, None);
    rejects::<BoundedBytes>(json!({"values": []}), "items.min", "/values");
    rejects::<BoundedBytes>(
        json!({"values": [0, 1, 2, 3, 4, 5, 6, 7, 8]}),
        "items.max",
        "/values",
    );
    for values in [vec![0], vec![0, 1, 2, 3, 4, 5, 6, 7]] {
        let decoded = resolve::<BoundedBytes>(json!({"values": values}))
            .expect("count boundary")
            .into_typed::<BoundedBytes>()
            .expect("decode");
        assert_eq!(decoded.values, Some(values));
    }
}

#[test]
fn zero_minimum_and_unique_do_not_require_optional_values() {
    for values in [None, Some(vec![]), Some(vec![0, 255])] {
        let input = match &values {
            Some(values) => json!({"values": values}),
            None => json!({}),
        };
        let decoded = resolve::<EmptyAllowed>(input)
            .expect("optional zero minimum")
            .into_typed::<EmptyAllowed>()
            .expect("decode");
        assert_eq!(decoded.values, values);
    }
    rejects::<EmptyAllowed>(json!({"values": [1, 2, 3]}), "items.max", "/values");
}

#[test]
fn single_count_bounds_keep_existing_unbounded_side() {
    rejects::<MinimumOnly>(json!({"values": ["a"]}), "items.min", "/values");
    let minimum = resolve::<MinimumOnly>(json!({"values": ["a", "a", "a"]}))
        .expect("duplicates allowed without unique")
        .into_typed::<MinimumOnly>()
        .expect("decode");
    assert_eq!(minimum.values, Some(vec!["a".to_owned(); 3]));
    let schema = MaximumOnly::schema().expect("full u32 maximum");
    let Property::List(list) = &schema.properties()[0] else {
        panic!("collection must lower to a list");
    };
    assert_eq!(list.min_items, None);
    assert_eq!(list.max_items, Some(u32::MAX));
    let maximum = resolve::<MaximumOnly>(json!({"values": []}))
        .expect("no minimum")
        .into_typed::<MaximumOnly>()
        .expect("decode");
    assert_eq!(maximum.values, Some(vec![]));
}

#[test]
fn uniqueness_reports_duplicate_item_without_imposing_a_count() {
    rejects::<UniqueOnly>(
        json!({"values": ["same", "same"]}),
        "items.unique",
        "/values/1",
    );
    for values in [vec![], vec!["a".to_owned(), "b".to_owned()]] {
        let decoded = resolve::<UniqueOnly>(json!({"values": values}))
            .expect("distinct items")
            .into_typed::<UniqueOnly>()
            .expect("decode");
        assert_eq!(decoded.values, Some(values));
    }
}

#[test]
fn collection_rules_are_conjunctive_with_numeric_item_domains() {
    rejects::<BoundedBytes>(json!({"values": [-1]}), "min", "/values/0");
    rejects::<BoundedBytes>(json!({"values": [256]}), "max", "/values/0");
    rejects::<BoundedBytes>(json!({"values": [255, 255]}), "items.unique", "/values/1");
    let decoded = resolve::<BoundedBytes>(json!({"values": [0, 255]}))
        .expect("numeric boundaries")
        .into_typed::<BoundedBytes>()
        .expect("decode");
    assert_eq!(decoded.values, Some(vec![0, 255]));
}

#[test]
fn nested_dto_lists_enforce_counts_uniqueness_and_item_contracts() {
    rejects::<NestedItems>(json!({"values": []}), "items.min", "/values");
    rejects::<NestedItems>(
        json!({"values": [{"id": 1, "name": "a"}, {"name": "a", "id": 1}]}),
        "items.unique",
        "/values/1",
    );
    rejects::<NestedItems>(
        json!({"values": [{"id": 1, "name": "a"}, {"id": 2, "name": "b"}, {"id": 3, "name": "c"}]}),
        "items.max",
        "/values",
    );
    rejects::<NestedItems>(
        json!({"values": [{"id": 256, "name": "a"}]}),
        "max",
        "/values/0/id",
    );
    rejects::<NestedItems>(json!({"values": [{"id": 1}]}), "required", "/values/0/name");
    let decoded = resolve::<NestedItems>(
        json!({"values": [{"id": 0, "name": "a"}, {"id": 255, "name": "b"}]}),
    )
    .expect("distinct nested DTOs")
    .into_typed::<NestedItems>()
    .expect("decode");
    assert_eq!(
        decoded.values,
        Some(vec![
            Item {
                id: 0,
                name: "a".to_owned()
            },
            Item {
                id: 255,
                name: "b".to_owned()
            }
        ])
    );
}

#[test]
fn zero_count_minimum_does_not_change_requiredness() {
    for schema in [RequiredItems::schema(), ExplicitRequiredItems::schema()] {
        let schema = schema.expect("required collection");
        assert_matches!(schema.properties()[0].required(), RequiredMode::Always);
    }
    for input in [json!({}), json!({"values": []})] {
        rejects::<RequiredItems>(input.clone(), "required", "/values");
        rejects::<ExplicitRequiredItems>(input, "required", "/values");
    }
    let required = resolve::<RequiredItems>(json!({"values": [0]}))
        .expect("present required list")
        .into_typed::<RequiredItems>()
        .expect("decode");
    assert_eq!(required.values, vec![0]);
    let explicit = resolve::<ExplicitRequiredItems>(json!({"values": [0]}))
        .expect("explicit required list")
        .into_typed::<ExplicitRequiredItems>()
        .expect("decode");
    assert_eq!(explicit.values, Some(vec![0]));
}

#[test]
fn invalid_collection_declarations_compile_fail() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/property_collection_*.rs");
}
