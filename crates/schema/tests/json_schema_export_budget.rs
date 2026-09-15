//! Resource rejection is separate from schema admission and export compatibility.

#![cfg(feature = "schemars")]

use std::{assert_matches, error::Error};

use nebula_schema::{
    ExpressionMode, Field, JsonSchemaExportError, Schema, SerdeTagging, ValidSchema, field_key,
};

fn nested_schema(depth: usize, mode: ExpressionMode, aliases: bool) -> ValidSchema {
    let mut field: Field = Field::string(field_key!("leaf"))
        .description("x".repeat(1024))
        .expression_mode(mode)
        .into();
    for _ in 0..depth {
        let mut object = Field::object(field_key!("nested"))
            .expression_mode(mode)
            .add(field);
        if aliases {
            object = object.read_alias("legacy").unwrap();
        }
        field = object.into();
    }
    Schema::builder().add(field).build().unwrap()
}

fn rejection(schema: &ValidSchema) -> JsonSchemaExportError {
    // Do not print a potentially huge successful document on a regression.
    let Err(error) = schema.json_schema() else {
        panic!("oversized export must return a bounded, typed rejection");
    };
    error
}

#[test]
fn depth_chain_rejects_expansion_in_both_copying_expression_modes() {
    for mode in [ExpressionMode::Forbidden, ExpressionMode::Allowed] {
        let schema = nested_schema(14, mode, false);
        assert_matches!(
            rejection(&schema),
            JsonSchemaExportError::CopyBudgetExceeded
        );
    }
}

#[test]
fn alias_fanout_rejects_cumulative_expansion_copies() {
    let schema = nested_schema(7, ExpressionMode::Forbidden, true);
    assert_matches!(
        rejection(&schema),
        JsonSchemaExportError::CopyBudgetExceeded
    );
}

#[test]
fn deepest_admitted_chain_rejects_expansion_without_materializing_it() {
    for mode in [ExpressionMode::Forbidden, ExpressionMode::Allowed] {
        let schema = nested_schema(64, mode, false);
        assert_matches!(
            rejection(&schema),
            JsonSchemaExportError::CopyBudgetExceeded
        );
    }
}

#[test]
fn deep_expression_required_chain_does_not_pay_for_nonexistent_copies() {
    let exported = nested_schema(64, ExpressionMode::Required, false)
        .json_schema()
        .unwrap()
        .to_value();
    let mut field = &exported["properties"]["nested"];
    for _ in 0..63 {
        assert_eq!(field["x-nebula-expression-mode"], "required");
        field = &field["x-nebula-resolved-value-schema"]["properties"]["nested"];
    }
    let leaf = &field["x-nebula-resolved-value-schema"]["properties"]["leaf"];
    assert_eq!(leaf["x-nebula-resolved-value-schema"]["type"], "string");
}

#[test]
fn escaped_metadata_rejects_serialized_source_size_without_disclosure() {
    let description = format!("private-export-sentinel{}", "\0".repeat(200_000));
    let schema = Schema::builder()
        .add(Field::string(field_key!("text")).description(description))
        .build()
        .unwrap();
    let error = rejection(&schema);
    assert_matches!(error, JsonSchemaExportError::SourceBudgetExceeded);
    assert_eq!(
        error.to_string(),
        "JSON Schema source descriptor exceeds the export budget"
    );
    assert_eq!(format!("{error:?}"), "SourceBudgetExceeded");
    assert!(error.source().is_none());
}

#[test]
fn defaults_and_option_metadata_share_the_source_budget() {
    let oversized = "\0".repeat(200_000);
    for field in [
        Field::from(Field::string(field_key!("text")).default(oversized.clone().into())),
        Field::from(Field::select(field_key!("choice")).option("value", oversized)),
    ] {
        let schema = Schema::builder().add(field).build().unwrap();
        assert_matches!(
            rejection(&schema),
            JsonSchemaExportError::SourceBudgetExceeded
        );
    }
}

#[test]
fn deeply_nested_metadata_is_not_recursively_copied_or_measured_without_a_depth_guard() {
    let mut value = serde_json::json!(null);
    for _ in 0..128 {
        value = serde_json::json!([value]);
    }
    for field in [
        Field::from(
            Field::list(field_key!("list"))
                .item(Field::string(field_key!("item")))
                .default(value.clone()),
        ),
        Field::from(
            Field::select(field_key!("select"))
                .option(serde_json::json!({"nested": value}), "Nested"),
        ),
    ] {
        let schema = Schema::builder().add(field).build().unwrap();
        assert_matches!(
            rejection(&schema),
            JsonSchemaExportError::BudgetSerialization
        );
    }
}

#[test]
fn repeated_adjacent_union_names_share_the_copy_budget() {
    let mut mode = Field::mode(field_key!("choice"));
    for index in 0..64 {
        mode = mode.variant_empty(format!("variant_{index}"), "Variant");
    }
    let schema = ValidSchema::union(
        mode,
        SerdeTagging::Adjacent {
            tag: "t".repeat(100_000),
            content: "content".to_owned(),
        },
    )
    .unwrap();
    assert_matches!(
        rejection(&schema),
        JsonSchemaExportError::CopyBudgetExceeded
    );
}

#[test]
fn useful_nested_schema_retains_its_projection() {
    let exported = nested_schema(2, ExpressionMode::Forbidden, false)
        .json_schema()
        .unwrap()
        .to_value();
    assert_eq!(exported["x-nebula-schema-version"], 2);
    assert_eq!(
        exported.pointer("/properties/nested/properties/nested/properties/leaf/type"),
        Some(&serde_json::json!("string"))
    );
    let validator = jsonschema::validator_for(&exported).unwrap();
    assert!(validator.is_valid(&serde_json::json!({"nested": {"nested": {"leaf": "text"}}})));
    assert!(!validator.is_valid(&serde_json::json!({"nested": {"nested": {"leaf": 42}}})));
}
