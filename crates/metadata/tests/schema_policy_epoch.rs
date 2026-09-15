use nebula_metadata::{MetadataBuildError, MetadataDraft, RecordedBaseMetadata};
use nebula_schema::{Property, Schema, ValidSchema, field_key};
use serde_json::json;

#[test]
fn historical_schema_cannot_bind_fresh_metadata() {
    let schema = serde_json::from_value::<ValidSchema>(json!({"fields": []})).unwrap();
    let error = MetadataDraft::try_new("example", "Example", "description")
        .unwrap()
        .bind_schema(schema)
        .expect_err("historical schema cannot acquire catalog authority");
    let MetadataBuildError::Schema(report) = error else {
        panic!("expected schema admission rejection");
    };
    assert!(
        report
            .errors()
            .any(|error| error.code() == "schema.unsupported_policy")
    );
}

#[test]
fn unknown_declarations_cannot_bind_fresh_metadata_even_when_inactive() {
    let unknown: Property = serde_json::from_value(json!({
        "type": "vendor.future_kind", "key": "future", "payload": "must-not-leak"
    }))
    .unwrap();
    for (field, path) in [
        (unknown.clone(), "/future"),
        (
            Property::list(field_key!("items"))
                .item(unknown.clone())
                .into(),
            "/items/0",
        ),
        (
            Property::list(field_key!("items"))
                .item(Property::list(field_key!("row")).item(unknown.clone()))
                .into(),
            "/items/0/0",
        ),
        (
            Property::mode(field_key!("auth"))
                .variant_empty("none", "None")
                .variant("future", "Future", unknown)
                .into(),
            "/auth/future",
        ),
    ] {
        let schema = Schema::builder().property(field).build().unwrap();
        let wire = serde_json::to_value(&schema).unwrap();
        let decoded: ValidSchema = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
        let result = MetadataDraft::try_new("example", "Example", "description")
            .unwrap()
            .bind_schema(decoded);
        std::assert_matches!(result, Err(MetadataBuildError::Schema(report))
            if report.errors().any(|error| error.code() == "schema.unsupported_property_kind"
                && error.path().to_string() == path)
                && !format!("{report:?}").contains("must-not-leak"));
    }
}

#[test]
fn recorded_legacy_schema_cannot_readmit_against_identical_current_root() {
    let fresh = MetadataDraft::try_new("example".to_owned(), "Example", "description")
        .unwrap()
        .bind_schema(ValidSchema::empty())
        .unwrap();
    let mut wire = serde_json::to_value(&fresh).unwrap();
    wire["schema"] = json!({"fields": []});
    let recorded: RecordedBaseMetadata<String> = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&recorded).unwrap(), wire);
    assert!(recorded.readmit_against(&fresh).is_err());
}
