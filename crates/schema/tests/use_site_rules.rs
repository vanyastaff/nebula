//! Authored occurrence rules retain contextual obligations and existing facets.

use nebula_schema::{
    InputCodec, InputContract, Predicate, Property, PropertyType, Rule, SchemaTypeBuilder,
    SchemaTypeUse, Transformer, ValidationReport, field_key,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize, PartialEq)]
struct ContextualInput {
    approved: bool,
    label: String,
}

impl PropertyType for ContextualInput {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        let root = builder.define::<Self>(|builder| {
            let boolean = bool::define_schema_type(builder)?;
            let string = String::define_schema_type(builder)?;
            let approved = builder.property(boolean, Property::boolean(field_key!("approved")).required().into())?;
            let label = builder.property(string, Property::string(field_key!("label"))
                .required().read_alias("old_label")?.with_transformer(Transformer::Trim).into())?;
            Ok(json!({"kind":"record","properties":[approved,label],"additional_properties":"closed"}))
        })?;
        root.allow_null()
            .with_rule(Rule::predicate(Predicate::eq("/approved", true).unwrap()).unwrap())?
            .with_rule(Rule::predicate(Predicate::eq("/label", "ready").unwrap()).unwrap())
    }
}
impl InputCodec for ContextualInput {}

#[derive(Debug, Deserialize)]
struct ChildRulesInput {
    value: String,
    bounded: String,
}
impl PropertyType for ChildRulesInput {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder.define::<Self>(|builder| {
            let string = String::define_schema_type(builder)?.with_rule(Rule::min_length(5))?;
            let value = builder.property(
                string.clone(),
                Property::string(field_key!("value"))
                    .required()
                    .read_alias("legacy_value")?
                    .with_transformer(Transformer::Trim)
                    .into(),
            )?;
            let bounded = builder.property(
                string,
                Property::string(field_key!("bounded"))
                    .required()
                    .with_rule(Rule::max_length(8))
                    .into(),
            )?;
            Ok(json!({"kind":"record","properties":[value,bounded],"additional_properties":"closed"}))
        })
    }
}
impl InputCodec for ChildRulesInput {}

#[test]
fn attaching_a_property_retains_occurrence_rules_and_appends_property_rules() {
    let input = InputContract::for_type::<ChildRulesInput>().unwrap();
    let resolved = input
        .validate_data(json!({"legacy_value":" ready ","bounded":"ready"}))
        .unwrap();
    let typed = resolved.into_typed::<ChildRulesInput>(&input).unwrap();
    assert_eq!(typed.value, "ready");
    assert_eq!(typed.bounded, "ready");
    for invalid in [
        json!({"value":"a","bounded":"ready"}),
        json!({"value":"ready","bounded":"a"}),
        json!({"value":"ready","bounded":"too-long-value"}),
    ] {
        assert!(input.validate_data(invalid).is_err());
    }
}

#[test]
fn appended_contextual_rules_validate_prepared_values_and_retain_facets() {
    let input = InputContract::for_type::<ContextualInput>().unwrap();
    let document = serde_json::to_value(input.graph().to_document()).unwrap();
    assert_eq!(document["root"]["null"], "allow");
    assert_eq!(document["root"]["rules"].as_array().unwrap().len(), 2);
    let resolved = input
        .validate_data(json!({"approved":true,"old_label":" ready "}))
        .unwrap();
    assert_eq!(
        resolved.into_typed::<ContextualInput>(&input).unwrap(),
        ContextualInput {
            approved: true,
            label: "ready".into(),
        }
    );
    for invalid in [
        json!({"approved":false,"label":"ready"}),
        json!({"approved":true,"label":"other"}),
    ] {
        assert!(input.validate_data(invalid).is_err());
    }
}

#[test]
fn intrinsic_contextual_rule_refusal_remains_unchanged() {
    let document = serde_json::from_value::<nebula_schema::SchemaGraphDocument>(json!({
        "version":3,"root":{"target":"record","null":"reject"},
        "definitions":[{"key":"record","body":{"kind":"record","properties":[],
            "additional_properties":"open","intrinsic_rules":[
                Rule::predicate(Predicate::eq("/approved", true).unwrap()).unwrap()
            ]}}]
    }))
    .unwrap();
    let error = document.admit().unwrap_err();
    assert!(
        error
            .report()
            .errors()
            .any(|error| error.code() == "schema.graph.inapplicable_facet")
    );
}
