//! Outbound values cannot be repaired by the authored-input preparation path.

use nebula_schema::{ExpressionMode, Property, Schema, Transformer, field_key};
use serde_json::json;

#[test]
fn output_does_not_apply_aliases_transforms_or_defaults() {
    let schema = Schema::builder()
        .property(
            Property::string(field_key!("name"))
                .required()
                .read_alias("legacy")
                .unwrap()
                .with_transformer(Transformer::Trim)
                .min_length(3)
                .default(json!("default")),
        )
        .build()
        .unwrap();
    assert!(
        schema
            .validate_output_data(json!({"legacy": "valid"}))
            .is_err()
    );
    assert!(schema.validate_output_data(json!({})).is_err());
    let proof = schema.validate_output_data(json!({"name": " a "})).unwrap();
    assert_eq!(proof.get(&field_key!("name")), Some(&json!(" a ")));
    assert!(proof.schema().ptr_eq(&schema));
}

#[test]
fn output_remains_literal_even_when_input_requires_expression() {
    let schema = Schema::builder()
        .property(Property::string(field_key!("name")).expression_mode(ExpressionMode::Required))
        .build()
        .unwrap();
    let proof = schema
        .validate_output_data(json!({"name": "{{ secret }}"}))
        .unwrap();
    assert_eq!(proof.get(&field_key!("name")), Some(&json!("{{ secret }}")));
}

#[test]
fn absent_nested_and_anonymous_protected_domains_reject_before_values() {
    let fields: [Property; 2] = [
        Property::object(field_key!("optional"))
            .property(Property::secret(field_key!("password")))
            .into(),
        Property::list(field_key!("empty"))
            .item(Property::secret(field_key!("item")))
            .into(),
    ];
    for field in fields {
        let schema = Schema::builder().property(field).build().unwrap();
        let error = schema.ensure_public_output_domain().unwrap_err();
        assert!(
            error
                .errors()
                .any(|error| error.code() == "schema.output.protected_domain")
        );
        assert!(schema.validate_output_data(json!({})).is_err());
    }
}
