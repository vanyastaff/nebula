use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use nebula_schema::{
    AuthoredValue, FieldPath, LoaderContext, LoaderRegistry, LoaderResult, Property, Schema,
    SelectOption, ValidSchema, ValidationError,
    context::{predicate_context_for, root_predicate_context_for},
    field_key,
    schema::MAX_SCHEMA_DEPTH,
};
use rstest::rstest;
use serde_json::{Value, json};

const PAYLOAD: &str = "unknown-private-value";
const DESCRIPTOR: &str = "unknown-private-descriptor";
const KIND: &str = "vendor.future_secret";

#[derive(Debug, Clone, Copy)]
enum Shape {
    Root,
    List,
    InactiveMode,
}

fn unknown() -> Property {
    serde_json::from_value(json!({
        "type": KIND,
        "key": "future",
        "vendor_payload": DESCRIPTOR
    }))
    .unwrap()
}

fn scenario(shape: Shape) -> (Property, AuthoredValue, &'static str) {
    let (field, input, path) = match shape {
        Shape::Root => (unknown(), json!({"future": PAYLOAD}), "/future"),
        Shape::List => (
            Property::list(field_key!("items")).item(unknown()).into(),
            json!({"items": [PAYLOAD]}),
            "/items/0",
        ),
        Shape::InactiveMode => (
            Property::mode(field_key!("auth"))
                .variant_empty("none", "None")
                .variant("future", "Future", unknown())
                .into(),
            json!({"auth": {"mode": "none", "value": PAYLOAD}}),
            "/auth/future",
        ),
    };
    (field, AuthoredValue::from_data(input).unwrap(), path)
}

fn schema(field: Property, historical: bool) -> ValidSchema {
    let current = Schema::builder().property(field).build().unwrap();
    if !historical {
        return current;
    }
    let mut wire = serde_json::to_value(current).unwrap();
    wire.as_object_mut().unwrap().remove("policy_version");
    let historical: ValidSchema = serde_json::from_value(wire).unwrap();
    assert_eq!(historical.policy_version(), 1);
    historical
}

fn assert_private_error(error: &ValidationError, code: &str, path: &str) {
    assert_eq!(error.code(), code);
    assert_eq!(error.path().to_string(), path);
    let diagnostics = [
        error.to_string(),
        format!("{error:?}"),
        serde_json::to_string(error).unwrap(),
    ];
    for diagnostic in diagnostics {
        for private in [PAYLOAD, DESCRIPTOR, KIND] {
            assert!(!diagnostic.contains(private), "{diagnostic}");
        }
    }
    assert!(std::error::Error::source(error).is_none());
}

#[rstest]
fn projection_rejects_unknown_declarations_before_copying_values(
    #[values(Shape::Root, Shape::List, Shape::InactiveMode)] shape: Shape,
    #[values(false, true)] historical: bool,
) {
    let (field, values, path) = scenario(shape);
    let error = schema(field, historical).project(&values).unwrap_err();
    assert_private_error(&error, "schema.unsupported_property_kind", path);
}

#[rstest]
fn loader_snapshot_rejects_unknown_declarations_before_copying_values(
    #[values(Shape::Root, Shape::List, Shape::InactiveMode)] shape: Shape,
    #[values(false, true)] historical: bool,
) {
    let (field, values, path) = scenario(shape);
    let error = LoaderContext::new("choice", values)
        .with_secrets_redacted(&schema(field, historical))
        .unwrap_err();
    assert_private_error(&error, "schema.unsupported_property_kind", path);
}

#[rstest]
fn raw_predicate_snapshots_reject_unknown_declarations(
    #[values(Shape::Root, Shape::List, Shape::InactiveMode)] shape: Shape,
) {
    let (field, values, path) = scenario(shape);
    for result in [
        predicate_context_for(std::slice::from_ref(&field), &values),
        root_predicate_context_for(std::slice::from_ref(&field), &values),
    ] {
        assert_private_error(
            &result.unwrap_err(),
            "schema.unsupported_property_kind",
            path,
        );
    }
}

#[rstest]
#[tokio::test]
async fn raw_schema_loaders_reject_unknown_declarations_without_dispatch(
    #[values(Shape::Root, Shape::List, Shape::InactiveMode)] shape: Shape,
) {
    let (field, values, path) = scenario(shape);
    let draft: Schema = serde_json::from_value(json!({"fields": [
        field,
        Property::from(Property::select(field_key!("choice")).dynamic().loader("options")),
        Property::from(Property::dynamic(field_key!("record")).loader("records"))
    ]}))
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let option_calls = Arc::clone(&calls);
    let record_calls = Arc::clone(&calls);
    let registry = LoaderRegistry::new()
        .register_option("options", move |_| {
            option_calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(LoaderResult::<SelectOption>::done(vec![])) }
        })
        .register_record("records", move |_| {
            record_calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(LoaderResult::<Value>::done(vec![])) }
        });
    let options = LoaderContext::new("choice", values.clone());
    let records = LoaderContext::new("record", values);
    let errors = [
        draft
            .load_select_options("choice", &registry, options.clone())
            .await
            .map(|_| ()),
        draft
            .load_select_options_at(&FieldPath::parse("choice").unwrap(), &registry, options)
            .await
            .map(|_| ()),
        draft
            .load_dynamic_records("record", &registry, records.clone())
            .await
            .map(|_| ()),
        draft
            .load_dynamic_records_at(&FieldPath::parse("record").unwrap(), &registry, records)
            .await
            .map(|_| ()),
    ];
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for error in errors {
        assert_private_error(
            &error.unwrap_err(),
            "schema.unsupported_property_kind",
            path,
        );
    }
}

#[rstest]
#[case::at_limit(MAX_SCHEMA_DEPTH, "schema.unsupported_property_kind")]
#[case::over_limit(MAX_SCHEMA_DEPTH + 1, "schema.depth_limit")]
fn raw_schema_depth_is_checked_before_recursive_support_scan(
    #[case] depth: u8,
    #[case] code: &str,
) {
    let mut field = unknown();
    for _ in 0..depth {
        field = Property::list(field_key!("items")).item(field).into();
    }
    let values = AuthoredValue::from_data(json!({"items": PAYLOAD})).unwrap();
    let error = predicate_context_for(&[field], &values).unwrap_err();
    assert_eq!(error.code(), code);
    assert!(!format!("{error:?}").contains(PAYLOAD));
    assert!(!format!("{error:?}").contains(DESCRIPTOR));
}

#[test]
fn unindexable_mode_variant_cannot_hide_overdeep_unknown_subtree() {
    let mut field = unknown();
    for _ in 0..=MAX_SCHEMA_DEPTH {
        field = Property::list(field_key!("items")).item(field).into();
    }
    let field = Property::mode(field_key!("auth"))
        .variant("", "Unindexable", field)
        .into();
    let values = AuthoredValue::from_data(json!({"auth": PAYLOAD})).unwrap();
    let error = predicate_context_for(&[field], &values).unwrap_err();
    assert_private_error(&error, "schema.depth_limit", "");
}

#[tokio::test]
async fn overdeep_raw_schema_loader_stops_before_support_scan_and_dispatch() {
    let mut field = unknown();
    for _ in 0..=MAX_SCHEMA_DEPTH {
        field = Property::list(field_key!("items")).item(field).into();
    }
    let draft: Schema = serde_json::from_value(json!({"fields": [
        Property::from(Property::dynamic(field_key!("record")).loader("records")),
        field
    ]}))
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = Arc::clone(&calls);
    let registry = LoaderRegistry::new().register_record("records", move |_| {
        callback_calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(LoaderResult::<Value>::done(vec![])) }
    });
    let result = draft
        .load_dynamic_records(
            "record",
            &registry,
            LoaderContext::new(
                "record",
                AuthoredValue::from_data(json!({"items": PAYLOAD})).unwrap(),
            ),
        )
        .await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_private_error(&result.unwrap_err(), "schema.depth_limit", "");
}
