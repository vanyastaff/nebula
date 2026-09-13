//! Independent Draft 2020-12 oracle for the literal-data export contract.

#![cfg(feature = "schemars")]

use nebula_schema::{
    AuthoredValue, EngineExpressionContext, Field, HasSchema, Predicate, Rule, Schema,
    SerdeTagging, ValidSchema, ValidationError, ValidationReport, VisibilityMode, field_key,
};
use nebula_validator::ValueRule;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

fn exported_validator(schema: &ValidSchema) -> jsonschema::Validator {
    let exported = schema
        .json_schema()
        .expect("export must succeed")
        .to_value();
    jsonschema::validator_for(&exported).expect("export must be a valid Draft 2020-12 schema")
}

fn complete_wire(schema: &ValidSchema, wire: Value) -> Result<Value, ValidationReport> {
    let authored = schema.values_from_wire(wire)?;
    schema
        .validate(authored)?
        .resolve_data()?
        .into_typed()
        .map_err(Into::into)
}

#[track_caller]
fn assert_parity(schema: &ValidSchema, input: Value, accepted: bool) {
    let runtime = complete_wire(schema, input.clone());
    assert_eq!(
        runtime.is_ok(),
        accepted,
        "runtime for {input}: {runtime:?}"
    );
    let oracle = exported_validator(schema);
    let errors: Vec<_> = oracle
        .iter_errors(&input)
        .map(|error| error.to_string())
        .collect();
    assert_eq!(
        errors.is_empty(),
        accepted,
        "JSON Schema for {input}: {errors:?}"
    );
    if accepted {
        assert_eq!(
            runtime.unwrap(),
            input,
            "literal input must survive full resolution"
        );
    }
}

#[rstest]
#[case::root(json!({"name": "Ada", "extra": {"nested": [null, 3, true]}}))]
#[case::empty_key(json!({"name": "Ada", "": "empty key"}))]
#[case::escaped_keys(json!({"name": "Ada", "a/b~c": {"0": 4}}))]
fn root_record_keeps_arbitrary_extra_data(#[case] input: Value) {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).no_expression().required())
        .build()
        .unwrap();
    assert_parity(&schema, input, true);
}

#[rstest]
#[case::object(json!({"config": {"count": 2, "extra": [1, 2]}}))]
#[case::empty_object(json!({"opaque": {"": null, "a/b": true}}))]
#[case::list_item(json!({"rows": [{"id": 2, "extra": {"a~b": false}}]}))]
fn nested_object_payloads_are_open(#[case] input: Value) {
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("config")).add(
                Field::number(field_key!("count"))
                    .no_expression()
                    .required(),
            ),
        )
        .add(Field::object(field_key!("opaque")))
        .add(
            Field::list(field_key!("rows")).item(
                Field::object(field_key!("row"))
                    .add(Field::number(field_key!("id")).no_expression().required()),
            ),
        )
        .build()
        .unwrap();
    assert_parity(&schema, input, true);
}

#[rstest]
#[case::null(json!(null))]
#[case::boolean(json!(true))]
#[case::number(json!(42))]
#[case::string(json!("data"))]
#[case::array(json!([1, {"": null}]))]
#[case::object(json!({"data": {"$expr": "ordinary data"}}))]
fn any_schema_accepts_every_json_kind(#[case] input: Value) {
    let schema = <Value as HasSchema>::schema().unwrap();
    assert_parity(&schema, input, true);
}

#[rstest]
#[case::empty(json!({}), true)]
#[case::extras(json!({"data": 1}), true)]
#[case::null(json!(null), false)]
#[case::array(json!([]), false)]
fn empty_record_is_open_but_still_an_object(#[case] input: Value, #[case] accepted: bool) {
    assert_parity(&ValidSchema::empty(), input, accepted);
}

fn union_schema(tagging: SerdeTagging) -> ValidSchema {
    ValidSchema::union(
        Field::mode(field_key!("event"))
            .variant(
                "data",
                "Data",
                Field::object(field_key!("payload"))
                    .add(Field::number(field_key!("id")).no_expression().required()),
            )
            .variant_empty("none", "None"),
        tagging,
    )
    .unwrap()
}

#[rstest]
#[case::unit(json!("none"), true)]
#[case::payload(json!({"data": {"id": 1}}), true)]
#[case::open_payload(json!({"data": {"id": 1, "extra": [2]}}), true)]
#[case::envelope_extra(json!({"data": {"id": 1}, "extra": false}), false)]
#[case::unknown(json!({"other": {}}), false)]
#[case::unit_with_payload(json!({"none": {}}), false)]
#[case::data_without_payload(json!("data"), false)]
#[case::wrong_payload(json!({"data": {"id": "bad"}}), false)]
fn external_union_has_closed_envelope_and_open_payload(
    #[case] input: Value,
    #[case] accepted: bool,
) {
    assert_parity(&union_schema(SerdeTagging::External), input, accepted);
}

#[rstest]
#[case::unit(json!({"type": "none"}), true)]
#[case::payload(json!({"type": "data", "content": {"id": 1}}), true)]
#[case::open_payload(json!({"type": "data", "content": {"id": 1, "extra": true}}), true)]
#[case::envelope_extra(json!({"type": "data", "content": {"id": 1}, "extra": false}), false)]
#[case::missing_tag(json!({"content": {"id": 1}}), false)]
#[case::unknown(json!({"type": "other"}), false)]
#[case::unit_with_payload(json!({"type": "none", "content": {}}), false)]
#[case::data_without_payload(json!({"type": "data"}), false)]
fn adjacent_union_has_closed_envelope_and_open_payload(
    #[case] input: Value,
    #[case] accepted: bool,
) {
    assert_parity(
        &union_schema(SerdeTagging::Adjacent {
            tag: "type".to_owned(),
            content: "content".to_owned(),
        }),
        input,
        accepted,
    );
}

#[rstest]
#[case::unit(json!({"auth": {"mode": "none"}}), true)]
#[case::payload(json!({"auth": {"mode": "data", "value": {"id": 1}}}), true)]
#[case::open_payload(json!({"auth": {"mode": "data", "value": {"id": 1, "extra": 2}}}), true)]
#[case::envelope_extra(json!({"auth": {"mode": "none", "extra": false}}), false)]
#[case::missing_selector(json!({"auth": {"value": {"id": 1}}}), false)]
#[case::unknown_selector(json!({"auth": {"mode": "unknown"}}), false)]
#[case::wrong_payload(json!({"auth": {"mode": "data", "value": {"id": "bad"}}}), false)]
fn mode_has_closed_envelope_and_open_payload(#[case] input: Value, #[case] accepted: bool) {
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth"))
                .no_expression()
                .variant(
                    "data",
                    "Data",
                    Field::object(field_key!("payload"))
                        .required()
                        .add(Field::number(field_key!("id")).no_expression().required()),
                )
                .variant_empty("none", "None"),
        )
        .build()
        .unwrap();
    assert_parity(&schema, input, accepted);
}

#[rstest]
#[case::string(Field::string(field_key!("value")).no_expression().into(), json!("text"))]
#[case::code(Field::code(field_key!("value")).no_expression().into(), json!("code"))]
#[case::number(Field::number(field_key!("value")).no_expression().into(), json!(2.5))]
#[case::integer(Field::number(field_key!("value")).integer().no_expression().into(), json!(2.0))]
#[case::boolean(Field::boolean(field_key!("value")).into(), json!(false))]
#[case::file(Field::file(field_key!("value")).no_expression().into(), json!("file.txt"))]
fn scalar_shapes_match(#[case] field: Field, #[case] value: Value) {
    let schema = Schema::builder().add(field).build().unwrap();
    assert_parity(&schema, json!({"value": value}), true);
    for wrong in [json!({}), json!([]), Value::Null] {
        assert_parity(&schema, json!({"value": wrong}), false);
    }
    assert_parity(&schema, json!({}), true);
}

#[rstest]
#[case::valid(json!("ABC"), true)]
#[case::too_short(json!("A"), false)]
#[case::too_long(json!("ABCDE"), false)]
#[case::wrong_pattern(json!("aBC"), false)]
fn checked_pattern_and_length_rules_match(#[case] value: Value, #[case] accepted: bool) {
    let schema = Schema::builder()
        .add(
            Field::string(field_key!("value"))
                .no_expression()
                .min_length(2)
                .max_length(4)
                .with_rule(Rule::pattern("^[A-Z]+$").unwrap()),
        )
        .build()
        .unwrap();
    assert_parity(&schema, json!({"value": value}), accepted);
}

#[rstest]
#[case::valid(json!([1, 2]), true)]
#[case::short(json!([]), false)]
#[case::long(json!([1, 2, 3]), false)]
#[case::duplicate(json!([1, 1]), false)]
#[case::bad_item(json!(["bad"]), false)]
#[case::bad_bound(json!([0]), false)]
#[case::wrong_shape(json!({}), false)]
fn list_shape_and_constraints_match(#[case] value: Value, #[case] accepted: bool) {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("value"))
                .no_expression()
                .min_items(1)
                .max_items(2)
                .unique()
                .item(Field::number(field_key!("item")).no_expression().min(1)),
        )
        .build()
        .unwrap();
    assert_parity(&schema, json!({"value": value}), accepted);
}

#[rstest]
#[case::single_valid(false, json!("red"), true)]
#[case::single_unknown(false, json!("green"), false)]
#[case::single_array(false, json!(["red"]), false)]
#[case::multiple_valid(true, json!(["red", "blue"]), true)]
#[case::multiple_unknown(true, json!(["red", "green"]), false)]
#[case::multiple_scalar(true, json!("red"), false)]
fn static_select_shape_and_membership_match(
    #[case] multiple: bool,
    #[case] value: Value,
    #[case] accepted: bool,
) {
    let mut field = Field::select(field_key!("value"))
        .option("red", "Red")
        .option("blue", "Blue");
    if multiple {
        field = field.multiple();
    }
    let schema = Schema::builder().add(field).build().unwrap();
    assert_parity(&schema, json!({"value": value}), accepted);
}

#[rstest]
#[case::single_scalar(false, json!("custom"), true)]
#[case::single_object(false, json!({"custom": true}), true)]
#[case::single_null(false, Value::Null, true)]
#[case::single_array(false, json!(["custom"]), false)]
#[case::multiple_array(true, json!(["custom", {"value": 2}]), true)]
#[case::multiple_scalar(true, json!("custom"), false)]
fn custom_select_still_enforces_multiplicity(
    #[case] multiple: bool,
    #[case] value: Value,
    #[case] accepted: bool,
) {
    let mut field = Field::select(field_key!("value")).allow_custom();
    if multiple {
        field = field.multiple();
    }
    let schema = Schema::builder().add(field).build().unwrap();
    assert_parity(&schema, json!({"value": value}), accepted);
}

#[rstest]
#[case::string(Field::string(field_key!("value")).no_expression().required().into(), json!("ok"), json!(""))]
#[case::list(Field::list(field_key!("value")).no_expression().required().item(Field::number(field_key!("item")).no_expression()).into(), json!([1]), json!([]))]
#[case::multi_select(Field::select(field_key!("value")).multiple().allow_custom().required().into(), json!([1]), json!([]))]
#[case::single_select(Field::select(field_key!("value")).allow_custom().required().into(), json!("ok"), Value::Null)]
#[case::file(Field::file(field_key!("value")).no_expression().required().into(), json!("file.txt"), json!(""))]
#[case::multi_file(Field::file(field_key!("value")).no_expression().multiple().required().into(), json!(["file.txt"]), json!([]))]
fn static_required_checks_presence_and_nonempty_values(
    #[case] field: Field,
    #[case] valid: Value,
    #[case] empty: Value,
) {
    let schema = Schema::builder().add(field).build().unwrap();
    assert_parity(&schema, json!({"value": valid}), true);
    assert_parity(&schema, json!({}), false);
    assert_parity(&schema, json!({"value": Value::Null}), false);
    assert_parity(&schema, json!({"value": empty}), false);
}

#[test]
fn dynamic_predicates_stay_annotations_not_json_schema_proofs() {
    let condition = Rule::predicate(Predicate::eq("flag", json!(true)).unwrap()).unwrap();
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("flag")))
        .add(
            Field::string(field_key!("value"))
                .no_expression()
                .required_when(condition.clone()),
        )
        .root_rule(condition)
        .build()
        .unwrap();
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(
        exported["properties"]["value"]["x-nebula-required-mode"],
        json!("when")
    );
    assert_eq!(exported["x-nebula-root-rules"].as_array().unwrap().len(), 1);
    let input = json!({"flag": true});
    assert!(exported_validator(&schema).is_valid(&input));
    let report = complete_wire(&schema, input).unwrap_err();
    assert_eq!(
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>(),
        ["required"]
    );
}

#[rstest]
#[case::min(json!(9_007_199_254_740_993_u64), Rule::value(ValueRule::Min(9_007_199_254_740_992_u64.into())).unwrap(), true)]
#[case::max(json!(9_007_199_254_740_993_u64), Rule::value(ValueRule::Max(9_007_199_254_740_992_u64.into())).unwrap(), false)]
#[case::strict_min(json!(5), Rule::greater_than(5), false)]
#[case::strict_min_above(json!(6), Rule::greater_than(5), true)]
#[case::strict_max(json!(5), Rule::less_than(5), false)]
#[case::strict_max_below(json!(4), Rule::less_than(5), true)]
#[case::unsigned_max(json!(u64::MAX), Rule::value(ValueRule::Max(u64::MAX.into())).unwrap(), true)]
#[case::integer_above_float_max(json!(9_007_199_254_740_993_u64), Rule::max_value_f64(9_007_199_254_740_992.0).unwrap(), false)]
#[case::integer_below_float_min(json!(u64::MAX), Rule::min_value_f64(18_446_744_073_709_551_616.0).unwrap(), false)]
fn numeric_bounds_match_without_integer_rounding(
    #[case] value: Value,
    #[case] rule: Rule,
    #[case] accepted: bool,
) {
    let schema = Schema::builder()
        .add(
            Field::number(field_key!("value"))
                .no_expression()
                .with_rule(rule),
        )
        .build()
        .unwrap();
    assert_parity(&schema, json!({"value": value}), accepted);
}

#[rstest]
#[case::absent(json!({}), true)]
#[case::present(json!({"value": "data"}), true)]
#[case::empty(json!({"value": ""}), false)]
#[case::null(json!({"value": null}), false)]
fn hidden_required_field_is_optional_until_supplied(#[case] input: Value, #[case] accepted: bool) {
    let schema = Schema::builder()
        .add(
            Field::string(field_key!("value"))
                .no_expression()
                .required()
                .visible(VisibilityMode::Never),
        )
        .build()
        .unwrap();
    assert_parity(&schema, input, accepted);
}

#[rstest]
#[case::number_bounds(Field::number(field_key!("value")).no_expression().min(10).min(1).into(), json!(10), json!(5))]
#[case::list_bounds(Field::list(field_key!("value")).no_expression().item(Field::number(field_key!("item")).no_expression()).min_items(2).with_rule(Rule::min_items(1)).into(), json!([1, 2]), json!([1]))]
#[case::patterns(Field::string(field_key!("value")).no_expression().with_rule(Rule::pattern("^A").unwrap()).with_rule(Rule::pattern("Z$").unwrap()).into(), json!("AZ"), json!("BZ"))]
#[case::boolean_rules(Field::boolean(field_key!("value")).with_rule(Rule::one_of(vec![json!(true)]).unwrap()).into(), json!(true), json!(false))]
#[case::file_rules(Field::file(field_key!("value")).no_expression().with_rule(Rule::min_length(2)).into(), json!("ab"), json!("a"))]
fn all_declared_basic_constraints_remain_conjunctive(
    #[case] field: Field,
    #[case] accepted: Value,
    #[case] rejected: Value,
) {
    let schema = Schema::builder().add(field).build().unwrap();
    assert_parity(&schema, json!({"value": accepted}), true);
    assert_parity(&schema, json!({"value": rejected}), false);
}

#[rstest]
#[case::duplicate_option(Field::select(field_key!("value")).option("red", "Red").option("red", "Alternate label").into(), json!("red"), true)]
#[case::empty_option_set(Field::select(field_key!("value")).into(), json!("anything"), false)]
#[case::empty_multi_option_set(Field::select(field_key!("value")).multiple().into(), json!(["anything"]), false)]
#[case::empty_multi_selection(Field::select(field_key!("value")).multiple().into(), json!([]), true)]
fn static_select_membership_is_set_membership(
    #[case] field: Field,
    #[case] value: Value,
    #[case] accepted: bool,
) {
    let schema = Schema::builder().add(field).build().unwrap();
    assert_parity(&schema, json!({"value": value}), accepted);
}

#[rstest]
#[case::omitted_selector(json!({"auth": {"value": "data"}}), true)]
#[case::explicit_default(json!({"auth": {"mode": "token", "value": "data"}}), true)]
#[case::other_variant(json!({"auth": {"mode": "none"}}), true)]
#[case::unknown_selector(json!({"auth": {"mode": "unknown", "value": "data"}}), false)]
#[case::wrong_selector_type(json!({"auth": {"mode": null, "value": "data"}}), false)]
#[case::missing_payload(json!({"auth": {}}), false)]
fn mode_default_only_supplies_an_absent_selector(#[case] input: Value, #[case] accepted: bool) {
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth"))
                .no_expression()
                .variant(
                    "token",
                    "Token",
                    Field::string(field_key!("payload"))
                        .no_expression()
                        .required(),
                )
                .variant_empty("none", "None")
                .default_variant("token"),
        )
        .build()
        .unwrap();
    assert_parity(&schema, input, accepted);
}

#[test]
fn hidden_required_mode_payload_follows_presence_policy() {
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth")).no_expression().variant(
                "hidden",
                "Hidden",
                Field::string(field_key!("payload"))
                    .required()
                    .no_expression()
                    .visible(VisibilityMode::Never),
            ),
        )
        .build()
        .unwrap();
    assert_parity(&schema, json!({"auth": {"mode": "hidden"}}), true);
    assert_parity(
        &schema,
        json!({"auth": {"mode": "hidden", "value": "data"}}),
        true,
    );
    assert_parity(
        &schema,
        json!({"auth": {"mode": "hidden", "value": ""}}),
        false,
    );
}

#[test]
fn empty_enum_rule_exports_a_valid_unsatisfiable_constraint() {
    let schema = Schema::builder()
        .add(
            Field::boolean(field_key!("value"))
                .with_rule(Rule::one_of(Vec::<Value>::new()).unwrap()),
        )
        .build()
        .unwrap();
    assert_parity(&schema, json!({}), true);
    for value in [json!(false), json!(true)] {
        assert_parity(&schema, json!({"value": value}), false);
    }
}

#[test]
fn required_read_alias_is_checked_and_canonicalized() {
    let schema = Schema::builder()
        .add(
            Field::string(field_key!("name"))
                .no_expression()
                .required()
                .read_alias("legacy")
                .unwrap(),
        )
        .build()
        .unwrap();
    let input = json!({"legacy": "Ada"});
    assert_eq!(
        complete_wire(&schema, input.clone()).unwrap(),
        json!({"name": "Ada"})
    );
    assert!(exported_validator(&schema).is_valid(&input));
    for invalid in [json!({}), json!({"legacy": ""}), json!({"legacy": null})] {
        assert_parity(&schema, invalid, false);
    }
}

#[tokio::test]
async fn expression_wrapper_is_an_annotation_boundary_not_a_resolved_proof() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("value")).min(1))
        .build()
        .unwrap();
    let input = json!({"value": {"$expr": "{{ $input.value }}"}});
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(
        exported["properties"]["value"]["x-nebula-expression-mode"],
        json!("allowed")
    );
    assert_eq!(
        exported["properties"]["value"]["x-nebula-resolved-value-schema"]["minimum"],
        json!(1)
    );
    assert!(exported_validator(&schema).is_valid(&input));
    assert!(
        !exported_validator(&schema).is_valid(&json!({"value": {"$expr": "{{ 1 }}", "extra": 2}}))
    );

    let valid = schema
        .validate(AuthoredValue::from_template_json(input).unwrap())
        .unwrap();
    assert!(
        !valid.pending().is_empty(),
        "export acceptance is not a complete proof"
    );
    let resolved = valid
        .clone()
        .resolve(&EngineExpressionContext::with_input(json!({"value": 2})))
        .await
        .unwrap();
    assert_eq!(resolved.into_json(), json!({"value": 2}));
    let report = valid
        .resolve(&EngineExpressionContext::with_input(json!({"value": 0})))
        .await
        .unwrap_err();
    assert_eq!(
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>(),
        ["min"]
    );
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn undeclared_json_keys_do_not_close_records(key in "[^a-z]{0,12}", value in any::<i64>()) {
        let schema = Schema::builder().add(Field::boolean(field_key!("flag"))).build().unwrap();
        let input = json!({"flag": true, (key): value});
        let runtime = complete_wire(&schema, input.clone()).unwrap();
        prop_assert_eq!(&runtime, &input);
        prop_assert!(exported_validator(&schema).is_valid(&input));
    }
}
