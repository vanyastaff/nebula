use super::{Presentation, Property, Requirement, Schema};
use crate::{
    AuthoredValue, ExpressionMode, Field, FieldKey, Predicate, RequiredMode, Rule, ScalarSchema,
    SerdeTagging, Transformer, ValidSchema, ValidationReport, VisibilityMode, field_key,
};
use serde_json::{Value, json};

fn string() -> ValidSchema {
    ValidSchema::scalar(ScalarSchema::string()).expect("checked string domain")
}

fn property(key: FieldKey, value: ValidSchema, requirement: Requirement) -> Property {
    Property {
        key,
        value,
        requirement,
        presentation: Presentation {
            label: None,
            visibility: VisibilityMode::Always,
        },
    }
}

fn endpoint_record() -> ValidSchema {
    Schema::builder()
        .property(property(
            field_key!("endpoint"),
            string(),
            Requirement::Required,
        ))
        .property(property(
            field_key!("enabled"),
            ValidSchema::scalar(ScalarSchema::boolean()).expect("checked boolean domain"),
            Requirement::Required,
        ))
        .build()
        .unwrap_or_else(|report| panic!("endpoint record: {report:?}"))
        .schema
}

fn diagnostics(report: &ValidationReport) -> Vec<(String, String)> {
    report
        .errors()
        .map(|error| (error.path().to_string(), error.code().to_owned()))
        .collect()
}

fn resolve(schema: &ValidSchema, data: Value) -> Result<Value, ValidationReport> {
    schema
        .validate(AuthoredValue::from_data(data).expect("bounded literal data"))?
        .resolve_data()
        .map(crate::ResolvedValues::into_json)
}

#[test]
fn reused_record_has_occurrence_annotations_and_direct_nested_paths() {
    let endpoint = endpoint_record();
    let mut primary = property(
        field_key!("primary"),
        endpoint.clone(),
        Requirement::Required,
    );
    primary.presentation.label = Some("Primary connection".into());
    let mut backup = property(field_key!("backup"), endpoint, Requirement::Required);
    backup.presentation.label = Some("Standby connection".into());
    backup.presentation.visibility = VisibilityMode::Never;
    let lowered = Schema::builder()
        .property(primary)
        .property(backup)
        .build()
        .unwrap_or_else(|report| panic!("reused record: {report:?}"));
    let primary = lowered
        .annotations
        .iter()
        .find(|annotation| annotation.path.to_string() == "/primary")
        .expect("primary occurrence annotation");
    let backup = lowered
        .annotations
        .iter()
        .find(|annotation| annotation.path.to_string() == "/backup")
        .expect("backup occurrence annotation");
    assert_eq!(
        primary.presentation.label.as_deref(),
        Some("Primary connection")
    );
    assert_eq!(primary.presentation.visibility, VisibilityMode::Always);
    assert_eq!(
        backup.presentation.label.as_deref(),
        Some("Standby connection")
    );
    assert_eq!(backup.presentation.visibility, VisibilityMode::Never);
    let data = json!({
        "primary": {"endpoint": "https://primary.test", "enabled": true},
        "backup": {"endpoint": "https://backup.test", "enabled": false}
    });
    assert_eq!(resolve(&lowered.schema, data.clone()).unwrap(), data);
    let mut invalid = data;
    invalid["backup"]["endpoint"] = json!(42);
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, invalid).unwrap_err()),
        vec![("/backup/endpoint".into(), "type_mismatch".into())]
    );
    assert!(!lowered.schema.flags().uses_expressions);
}

#[test]
fn cosmetic_changes_leave_runtime_schema_and_outcomes_equal() {
    let build = |label: &str, visibility| {
        let mut occurrence = property(
            field_key!("connection"),
            endpoint_record(),
            Requirement::Required,
        );
        occurrence.presentation.label = Some(label.into());
        occurrence.presentation.visibility = visibility;
        Schema::builder()
            .property(occurrence)
            .build()
            .unwrap_or_else(|report| panic!("cosmetic schema: {report:?}"))
    };
    let shown = build("Connection", VisibilityMode::Always);
    let hidden = build("Renamed", VisibilityMode::Never);
    assert_eq!(shown.schema, hidden.schema);
    let Field::Object(field) = &hidden.schema.fields()[0] else {
        panic!("record field")
    };
    assert_eq!(field.label, None);
    assert_eq!(field.visible, VisibilityMode::Always);
    for data in [
        json!({"connection": {"endpoint": "ok", "enabled": true}}),
        json!({}),
        json!({"connection": {"endpoint": 2, "enabled": true}}),
    ] {
        let outcome = |schema| resolve(schema, data.clone()).map_err(|report| diagnostics(&report));
        assert_eq!(outcome(&shown.schema), outcome(&hidden.schema));
    }
}

#[test]
fn hidden_required_omission_is_one_required_diagnostic_and_optional_omission_succeeds() {
    let mut hidden = property(field_key!("endpoint"), string(), Requirement::Required);
    hidden.presentation.visibility = VisibilityMode::Never;
    let lowered = Schema::builder()
        .property(hidden)
        .property(property(
            field_key!("optional"),
            string(),
            Requirement::Optional,
        ))
        .build()
        .unwrap_or_else(|report| panic!("requirement schema: {report:?}"));
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, json!({})).unwrap_err()),
        vec![("/endpoint".into(), "required".into())]
    );
    assert_eq!(
        resolve(&lowered.schema, json!({"endpoint": "ok"})).unwrap(),
        json!({"endpoint": "ok"})
    );
}

#[test]
fn independently_lowered_equal_schemas_keep_distinct_proof_origins() {
    let first = endpoint_record();
    let second = endpoint_record();
    assert_eq!(first, second);
    assert!(!first.ptr_eq(&second));
    for (origin, other) in [(&first, &second), (&second, &first)] {
        let valid = origin
            .validate(AuthoredValue::from_data(json!({"endpoint": "ok", "enabled": true})).unwrap())
            .unwrap();
        assert!(valid.schema().ptr_eq(origin));
        assert!(!valid.schema().ptr_eq(other));
        let resolved = valid.resolve_data().unwrap();
        assert!(resolved.schema().ptr_eq(origin));
        assert!(!resolved.schema().ptr_eq(other));
        assert_eq!(
            resolved.into_json(),
            json!({"endpoint": "ok", "enabled": true})
        );
    }
}

#[test]
fn local_string_rule_survives_both_scalar_mounting_and_nested_reuse() {
    let value = ValidSchema::scalar(ScalarSchema::string().root_rule(Rule::min_length(3))).unwrap();
    let record = Schema::builder()
        .property(property(
            field_key!("endpoint"),
            value,
            Requirement::Required,
        ))
        .build()
        .unwrap_or_else(|report| panic!("string rule: {report:?}"));
    let nested = Schema::builder()
        .property(property(
            field_key!("backup"),
            record.schema,
            Requirement::Required,
        ))
        .build()
        .unwrap_or_else(|report| panic!("nested string rule: {report:?}"));
    assert_eq!(
        resolve(&nested.schema, json!({"backup": {"endpoint": "abc"}})).unwrap(),
        json!({"backup": {"endpoint": "abc"}})
    );
    assert_eq!(
        diagnostics(&resolve(&nested.schema, json!({"backup": {"endpoint": "ab"}})).unwrap_err()),
        vec![("/backup/endpoint".into(), "min_length".into())]
    );
    let field = nested
        .schema
        .find_by_path(&crate::FieldPath::parse("backup.endpoint").unwrap())
        .unwrap();
    assert_eq!(field.rules(), &[Rule::min_length(3)]);
}

#[test]
fn expressions_are_rejected_at_property_and_nested_leaf() {
    let lowered = Schema::builder()
        .property(property(
            field_key!("backup"),
            endpoint_record(),
            Requirement::Required,
        ))
        .build()
        .unwrap_or_else(|report| panic!("expression schema: {report:?}"));
    for (data, path) in [
        (json!({"backup": {"$expr": "1 + 1"}}), "/backup"),
        (
            json!({"backup": {"endpoint": {"$expr": "1 + 1"}, "enabled": true}}),
            "/backup/endpoint",
        ),
    ] {
        let report = lowered
            .schema
            .validate(AuthoredValue::from_template_json(data).unwrap())
            .unwrap_err();
        assert_eq!(
            diagnostics(&report),
            vec![(path.into(), "expression.forbidden".into())]
        );
    }
}

#[test]
fn duplicate_keys_use_real_builder_diagnostics() {
    let report = Schema::builder()
        .property(property(
            field_key!("endpoint"),
            string(),
            Requirement::Required,
        ))
        .property(property(
            field_key!("endpoint"),
            string(),
            Requirement::Optional,
        ))
        .build()
        .err()
        .expect("duplicate rejected");
    assert_eq!(
        diagnostics(&report),
        vec![("/endpoint".into(), "duplicate_key".into())]
    );
    assert_eq!(FieldKey::new("bad-key").unwrap_err().code(), "invalid_key");
}

fn assert_rejected(value: ValidSchema, code: &str, path: &str) {
    let report = Schema::builder()
        .property(property(field_key!("value"), value, Requirement::Required))
        .build()
        .err()
        .expect("unsupported definition rejected");
    assert_eq!(diagnostics(&report), vec![(path.into(), code.into())]);
}

#[test]
fn excludes_any_union_null_and_numeric_roots() {
    for value in [
        ValidSchema::any(),
        ValidSchema::scalar(ScalarSchema::null()).unwrap(),
        ValidSchema::scalar(ScalarSchema::integer(0, 10).unwrap()).unwrap(),
        ValidSchema::scalar(ScalarSchema::number(0, 10).unwrap()).unwrap(),
        ValidSchema::union(
            Field::mode(field_key!("kind")).required().variant(
                "one",
                "One",
                Field::string(field_key!("payload")),
            ),
            SerdeTagging::External,
        )
        .unwrap(),
    ] {
        assert_rejected(value, "property.unsupported_shape", "/value");
    }
}

#[test]
fn excludes_record_root_rules_and_boolean_rules() {
    assert_rejected(
        crate::Schema::builder()
            .root_rule(Rule::custom("check").unwrap())
            .build()
            .unwrap(),
        "property.record_rules",
        "/value",
    );
    assert_rejected(
        ValidSchema::scalar(
            ScalarSchema::boolean().root_rule(Rule::one_of([json!(true)]).unwrap()),
        )
        .unwrap(),
        "property.boolean_rules",
        "/value",
    );
}

fn contextual_rule() -> Rule {
    Rule::predicate(Predicate::eq("switch", json!(true)).unwrap()).unwrap()
}

#[test]
fn excludes_contextual_custom_composite_and_nonstring_rules() {
    for rule in [
        contextual_rule(),
        Rule::custom("check").unwrap(),
        Rule::all([Rule::min_length(3), contextual_rule()]).unwrap(),
        Rule::not(Rule::custom("check").unwrap()).unwrap(),
        Rule::described(contextual_rule(), "message").unwrap(),
        Rule::any([Rule::min_length(1)]).unwrap(),
        Rule::min_value(1),
        Rule::min_items(1),
        Rule::one_of([json!(1)]).unwrap(),
    ] {
        let path = crate::ValuePath::single("value");
        let report = super::check_string_rules(std::slice::from_ref(&rule), &path).unwrap_err();
        assert_eq!(
            diagnostics(&report),
            vec![("/value".into(), "property.string_rule".into())]
        );
        let mut references = Vec::new();
        rule.field_references(&mut references);
        if references.is_empty() {
            assert_rejected(
                ValidSchema::scalar(ScalarSchema::string().root_rule(rule)).unwrap(),
                "property.string_rule",
                "/value",
            );
        } else {
            // Scalar admission already refuses lookups; do not forge a checked schema.
            let report = ValidSchema::scalar(ScalarSchema::string().root_rule(rule)).unwrap_err();
            assert_eq!(
                diagnostics(&report),
                vec![(String::new(), "dangling_reference".into())]
            );
        }
    }
}

#[test]
fn excludes_nonconstant_occurrence_visibility() {
    let mut occurrence = property(field_key!("value"), string(), Requirement::Required);
    occurrence.presentation.visibility = VisibilityMode::When(contextual_rule());
    let report = Schema::builder()
        .property(occurrence)
        .build()
        .err()
        .expect("condition rejected");
    assert_eq!(
        diagnostics(&report),
        vec![("/value".into(), "property.visibility".into())]
    );
}

#[test]
fn excludes_unsupported_field_shapes() {
    let key = field_key!("excluded");
    for field in [
        Field::number(key.clone()).into_field(),
        Field::secret(key.clone()).into_field(),
        Field::select(key.clone()).into_field(),
        Field::list(key.clone())
            .item(Field::string(field_key!("item")))
            .into_field(),
        Field::mode(key.clone())
            .variant_empty("empty", "Empty")
            .into_field(),
        Field::code(key.clone()).into_field(),
        Field::file(key.clone()).into_field(),
        Field::computed(key.clone()).into_field(),
        Field::dynamic(key.clone()).into_field(),
        Field::notice(key).into_field(),
        serde_json::from_value::<Field>(json!({"type": "future_kind", "key": "excluded"})).unwrap(),
    ] {
        let schema = crate::Schema::builder()
            .add(field)
            .build()
            .expect("checked excluded shape");
        assert_rejected(schema, "property.unsupported_field", "/value/excluded");
    }
}

#[test]
fn excludes_field_policies_including_nested_ones() {
    let field = || Field::string(field_key!("endpoint")).no_expression();
    let cases = [
        (field().read_alias("old").unwrap(), "property.aliases"),
        (field().emit_as("new").unwrap(), "property.aliases"),
        (
            field().with_transformer(Transformer::Trim),
            "property.transforms",
        ),
        (field().default(json!("default")), "property.defaults"),
        (
            field().expression_mode(ExpressionMode::Allowed),
            "property.expressions",
        ),
        (
            field().expression_mode(ExpressionMode::Required),
            "property.expressions",
        ),
        (
            field().required_when(contextual_rule()),
            "property.presence",
        ),
        (
            field().visible_when(contextual_rule()),
            "property.visibility",
        ),
        (field().description("help"), "property.field_metadata"),
        (field().placeholder("hint"), "property.field_metadata"),
        (field().group("group"), "property.field_metadata"),
        (
            field().hint(crate::InputHint::Email),
            "property.field_metadata",
        ),
        (
            field().widget(crate::StringWidget::Multiline),
            "property.field_metadata",
        ),
        (field().with_rule(contextual_rule()), "property.string_rule"),
        (
            field().with_rule(Rule::custom("check").unwrap()),
            "property.string_rule",
        ),
    ];
    for (field, code) in cases {
        let schema = crate::Schema::builder()
            .add(Field::boolean(field_key!("switch")).no_expression())
            .add(
                Field::object(field_key!("nested"))
                    .no_expression()
                    .add(field),
            )
            .build()
            .expect("checked excluded policy");
        assert_rejected(schema, code, "/value/nested/endpoint");
    }
}

#[test]
fn empty_records_and_nested_constant_annotations_remain_separate() {
    let value = crate::Schema::builder()
        .add(
            Field::string(field_key!("endpoint"))
                .no_expression()
                .required()
                .label("Endpoint")
                .visible(VisibilityMode::Never),
        )
        .build()
        .unwrap();
    let lowered = Schema::builder()
        .property(property(field_key!("backup"), value, Requirement::Required))
        .property(property(
            field_key!("empty"),
            ValidSchema::empty(),
            Requirement::Optional,
        ))
        .build()
        .unwrap_or_else(|report| panic!("nested presentation: {report:?}"));
    let annotation = lowered
        .annotations
        .iter()
        .find(|annotation| annotation.path.to_string() == "/backup/endpoint")
        .unwrap();
    assert_eq!(annotation.presentation.label.as_deref(), Some("Endpoint"));
    assert_eq!(annotation.presentation.visibility, VisibilityMode::Never);
    let field = lowered
        .schema
        .find_by_path(&crate::FieldPath::parse("backup.endpoint").unwrap())
        .unwrap();
    let Field::String(field) = field else {
        panic!("string field")
    };
    assert_eq!(field.label, None);
    assert_eq!(field.visible, VisibilityMode::Always);
    assert_eq!(field.required, RequiredMode::Always);
    assert_eq!(
        resolve(
            &lowered.schema,
            json!({"backup": {"endpoint": "ok"}, "empty": {}})
        )
        .unwrap(),
        json!({"backup": {"endpoint": "ok"}, "empty": {}})
    );
}

#[test]
fn optional_missing_succeeds_but_explicit_null_is_a_type_error() {
    let lowered = Schema::builder()
        .property(property(
            field_key!("endpoint"),
            string(),
            Requirement::Optional,
        ))
        .build()
        .unwrap_or_else(|report| panic!("optional schema: {report:?}"));
    assert_eq!(resolve(&lowered.schema, json!({})).unwrap(), json!({}));
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, json!({"endpoint": null})).unwrap_err()),
        vec![("/endpoint".into(), "type_mismatch".into())]
    );
    assert_eq!(
        resolve(&lowered.schema, json!({"endpoint": ""})).unwrap(),
        json!({"endpoint": ""})
    );
}

#[test]
fn required_empty_string_and_null_retain_current_required_mode_policy() {
    let lowered = Schema::builder()
        .property(property(
            field_key!("endpoint"),
            string(),
            Requirement::Required,
        ))
        .build()
        .unwrap_or_else(|report| panic!("required schema: {report:?}"));
    for data in [json!({"endpoint": ""}), json!({"endpoint": null})] {
        assert_eq!(
            diagnostics(&resolve(&lowered.schema, data).unwrap_err()),
            vec![("/endpoint".into(), "required".into())]
        );
    }
}
