//! Root shapes checked against the runtime and an independent JSON Schema validator.

#![cfg(feature = "schemars")]

use nebula_schema::{
    ResolvedValues, ScalarSchema, Schema, ValidSchema, ValidationReport, schema_of,
};
use nebula_validator::{Rule, ValueRule};
use rstest::rstest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize, Serialize, Schema)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "braced and unit structs have distinct serde root shapes"
)]
struct EmptyRecord {}

#[derive(Debug, Deserialize, Serialize, Schema)]
struct UnitStruct;

#[rstest]
#[case::unit(schema_of::<()>, "null")]
#[case::unit_struct(schema_of::<UnitStruct>, "null")]
#[case::empty_record(schema_of::<EmptyRecord>, "object")]
#[case::boolean(schema_of::<bool>, "boolean")]
#[case::string(schema_of::<String>, "string")]
#[case::i8(schema_of::<i8>, "integer")]
#[case::i16(schema_of::<i16>, "integer")]
#[case::i32(schema_of::<i32>, "integer")]
#[case::i64(schema_of::<i64>, "integer")]
#[case::i128(schema_of::<i128>, "integer")]
#[case::isize(schema_of::<isize>, "integer")]
#[case::u8(schema_of::<u8>, "integer")]
#[case::u16(schema_of::<u16>, "integer")]
#[case::u32(schema_of::<u32>, "integer")]
#[case::u64(schema_of::<u64>, "integer")]
#[case::u128(schema_of::<u128>, "integer")]
#[case::usize(schema_of::<usize>, "integer")]
#[case::f32(schema_of::<f32>, "number")]
#[case::f64(schema_of::<f64>, "number")]
fn root_exports_its_declared_type(
    #[case] schema: fn() -> Result<ValidSchema, ValidationReport>,
    #[case] expected_type: &str,
) {
    let exported = schema().unwrap().json_schema().unwrap().to_value();
    assert_eq!(exported["type"], expected_type);
    assert_eq!(
        exported["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    jsonschema::validator_for(&exported).expect("export must be valid Draft 2020-12");
}

fn complete_wire(schema: &ValidSchema, input: Value) -> Result<Value, ValidationReport> {
    let authored = schema.values_from_wire(input)?;
    Ok(schema.validate(authored)?.resolve_data()?.to_wire_json())
}

#[track_caller]
fn assert_parity(schema: &ValidSchema, input: Value, accepted: bool) {
    let exported = schema.json_schema().unwrap().to_value();
    let oracle = jsonschema::validator_for(&exported).unwrap();
    assert_eq!(
        oracle.is_valid(&input),
        accepted,
        "export for {input}: {exported}"
    );
    let runtime = complete_wire(schema, input.clone());
    assert_eq!(
        runtime.is_ok(),
        accepted,
        "runtime for {input}: {runtime:?}"
    );
    if accepted {
        assert_eq!(
            runtime.unwrap(),
            input,
            "already canonical data must remain unchanged"
        );
    }
}

#[rstest]
#[case::null(Value::Null, true)]
#[case::object(json!({}), false)]
#[case::array(json!([]), false)]
#[case::boolean(json!(false), false)]
#[case::string(json!(""), false)]
#[case::integer(json!(0), false)]
#[case::float(json!(0.0), false)]
fn unit_is_null_not_an_empty_record(#[case] input: Value, #[case] accepted: bool) {
    assert_eq!(serde_json::to_value(()).unwrap(), Value::Null);
    assert_parity(&schema_of::<()>().unwrap(), input, accepted);
}

#[rstest]
#[case::null(Value::Null, true)]
#[case::object(json!({}), false)]
#[case::array(json!([]), false)]
#[case::boolean(json!(false), false)]
#[case::string(json!(""), false)]
#[case::integer(json!(0), false)]
#[case::float(json!(0.0), false)]
fn unit_struct_is_null_not_an_empty_record(#[case] input: Value, #[case] accepted: bool) {
    assert_eq!(serde_json::to_value(UnitStruct).unwrap(), Value::Null);
    assert_eq!(
        serde_json::from_value::<UnitStruct>(input.clone()).is_ok(),
        accepted
    );
    assert_parity(&schema_of::<UnitStruct>().unwrap(), input, accepted);
}

#[rstest]
#[case::object(json!({}), true)]
#[case::extras(json!({"": [null, true], "a/b~c": 1}), true)]
#[case::null(Value::Null, false)]
#[case::array(json!([]), false)]
#[case::boolean(json!(false), false)]
#[case::string(json!(""), false)]
#[case::integer(json!(0), false)]
fn empty_braced_struct_remains_an_open_record(#[case] input: Value, #[case] accepted: bool) {
    assert_eq!(serde_json::to_value(EmptyRecord {}).unwrap(), json!({}));
    assert_parity(&schema_of::<EmptyRecord>().unwrap(), input, accepted);
}

#[rstest]
#[case::i8(schema_of::<i8>, json!(i8::MIN), json!(i8::MAX))]
#[case::i16(schema_of::<i16>, json!(i16::MIN), json!(i16::MAX))]
#[case::i32(schema_of::<i32>, json!(i32::MIN), json!(i32::MAX))]
#[case::i64(schema_of::<i64>, json!(i64::MIN), json!(i64::MAX))]
#[case::isize(schema_of::<isize>, json!(isize::MIN), json!(isize::MAX))]
#[case::u8(schema_of::<u8>, json!(u8::MIN), json!(u8::MAX))]
#[case::u16(schema_of::<u16>, json!(u16::MIN), json!(u16::MAX))]
#[case::u32(schema_of::<u32>, json!(u32::MIN), json!(u32::MAX))]
#[case::u64(schema_of::<u64>, json!(u64::MIN), json!(u64::MAX))]
#[case::usize(schema_of::<usize>, json!(usize::MIN), json!(usize::MAX))]
fn integer_bound_keywords_preserve_exact_json_integers(
    #[case] schema: fn() -> Result<ValidSchema, ValidationReport>,
    #[case] minimum: Value,
    #[case] maximum: Value,
) {
    let exported = schema().unwrap().json_schema().unwrap().to_value();
    assert_eq!(exported["minimum"], minimum);
    assert_eq!(exported["maximum"], maximum);
    for keyword in ["minimum", "maximum"] {
        assert!(
            !exported[keyword].as_number().unwrap().is_f64(),
            "integer bounds must never round through f64: {exported}"
        );
    }
}

#[rstest]
#[case::minimum(json!(-128), true)]
#[case::below_minimum(json!(-129), false)]
#[case::maximum(json!(127), true)]
#[case::above_maximum(json!(128), false)]
#[case::fraction(json!(1.5), false)]
#[case::string(json!("1"), false)]
#[case::null(Value::Null, false)]
fn signed_integer_boundaries_match(#[case] input: Value, #[case] accepted: bool) {
    assert_eq!(
        serde_json::from_value::<i8>(input.clone()).is_ok(),
        accepted
    );
    assert_parity(&schema_of::<i8>().unwrap(), input, accepted);
}

#[rstest]
#[case::minimum(json!(0), true)]
#[case::negative(json!(-1), false)]
#[case::maximum(json!(255), true)]
#[case::above_maximum(json!(256), false)]
#[case::fraction(json!(1.5), false)]
#[case::string(json!("1"), false)]
#[case::null(Value::Null, false)]
fn unsigned_integer_boundaries_match(#[case] input: Value, #[case] accepted: bool) {
    assert_eq!(
        serde_json::from_value::<u8>(input.clone()).is_ok(),
        accepted
    );
    assert_parity(&schema_of::<u8>().unwrap(), input, accepted);
}

#[rstest]
#[case::signed_minimum(schema_of::<i64>, json!(i64::MIN), true)]
#[case::signed_maximum(schema_of::<i64>, json!(i64::MAX), true)]
#[case::signed_overflow(schema_of::<i64>, json!(1_u64 << 63), false)]
#[case::signed_float_overflow(schema_of::<i64>, json!(9_223_372_036_854_775_808.0), false)]
#[case::signed_float_underflow(schema_of::<i64>, json!((-9_223_372_036_854_775_808.0_f64).next_down()), false)]
#[case::signed_fraction(schema_of::<i64>, json!(-0.5), false)]
#[case::fraction_near_integer(schema_of::<i64>, json!(1.0_f64.next_up()), false)]
#[case::unsigned_maximum(schema_of::<u64>, json!(u64::MAX), true)]
#[case::unsigned_previous(schema_of::<u64>, json!(u64::MAX - 1), true)]
#[case::unsigned_negative(schema_of::<u64>, json!(-1), false)]
#[case::unsigned_overflow(schema_of::<u64>, json!(18_446_744_073_709_551_616.0), false)]
#[case::unsigned_negative_float(schema_of::<u64>, json!(-1.0), false)]
#[case::unsigned_fraction(schema_of::<u64>, json!(0.5), false)]
#[case::json_integer_envelope_overflow(schema_of::<i128>, json!(18_446_744_073_709_551_616.0), false)]
#[case::json_integer_envelope_underflow(schema_of::<i128>, json!((-9_223_372_036_854_775_808.0_f64).next_down()), false)]
#[case::beyond_f64_integer_precision(schema_of::<u64>, json!(9_007_199_254_740_993_u64), true)]
fn large_integer_boundaries_do_not_round(
    #[case] schema: fn() -> Result<ValidSchema, ValidationReport>,
    #[case] input: Value,
    #[case] accepted: bool,
) {
    assert_parity(&schema().unwrap(), input, accepted);
}

#[track_caller]
fn assert_normalized_integer(
    schema: &ValidSchema,
    wire: &str,
    expected: &Value,
    expected_wire: &str,
) -> ResolvedValues {
    let input: Value = serde_json::from_str(wire).unwrap();
    assert!(input.as_number().unwrap().is_f64());
    let exported = schema.json_schema().unwrap().to_value();
    let oracle = jsonschema::validator_for(&exported).unwrap();
    assert!(
        oracle.is_valid(&input),
        "JSON Schema integers are mathematical values"
    );
    assert_eq!(exported["type"], "integer");
    let authored = schema.values_from_wire(input).unwrap();
    let resolved = schema.validate(authored).unwrap().resolve_data().unwrap();
    let output = resolved.to_wire_json();
    assert_eq!(&output, expected);
    assert!(!output.as_number().unwrap().is_f64());
    assert_eq!(
        serde_json::to_vec(&output).unwrap(),
        expected_wire.as_bytes()
    );
    assert!(oracle.is_valid(&output));
    assert_eq!(complete_wire(schema, output.clone()).unwrap(), output);
    resolved
}

#[rstest]
#[case::zero("0.0", 0, "0")]
#[case::negative_zero("-0.0", 0, "0")]
#[case::positive("1.0", 1, "1")]
#[case::negative("-1.0", -1, "-1")]
#[case::exponent("1e0", 1, "1")]
#[case::minimum("-9223372036854775808.0", i64::MIN, "-9223372036854775808")]
#[case::largest_float_within_domain(
    "9223372036854774784.0",
    9_223_372_036_854_774_784,
    "9223372036854774784"
)]
#[case::f64_integer_precision_boundary(
    "9007199254740992.0",
    9_007_199_254_740_992,
    "9007199254740992"
)]
fn signed_integral_floats_normalize_before_typed_decoding(
    #[case] wire: &str,
    #[case] expected: i64,
    #[case] expected_wire: &str,
) {
    let schema = schema_of::<i64>().unwrap();
    let resolved = assert_normalized_integer(&schema, wire, &json!(expected), expected_wire);
    let typed = resolved.into_typed::<i64>().unwrap();
    assert_eq!(typed, expected);
    assert_eq!(serde_json::from_str::<i64>(expected_wire).unwrap(), typed);
    assert_eq!(serde_json::to_string(&typed).unwrap(), expected_wire);
}

#[rstest]
#[case::negative_zero("-0.0", 0, "0")]
#[case::exponent("1e0", 1, "1")]
#[case::beyond_signed_domain(
    "9223372036854775808.0",
    9_223_372_036_854_775_808,
    "9223372036854775808"
)]
#[case::largest_float_within_domain(
    "18446744073709549568.0",
    18_446_744_073_709_549_568,
    "18446744073709549568"
)]
fn unsigned_integral_floats_normalize_before_typed_decoding(
    #[case] wire: &str,
    #[case] expected: u64,
    #[case] expected_wire: &str,
) {
    let schema = schema_of::<u64>().unwrap();
    let resolved = assert_normalized_integer(&schema, wire, &json!(expected), expected_wire);
    let typed = resolved.into_typed::<u64>().unwrap();
    assert_eq!(typed, expected);
    assert_eq!(serde_json::from_str::<u64>(expected_wire).unwrap(), typed);
    assert_eq!(serde_json::to_string(&typed).unwrap(), expected_wire);
}

#[rstest]
#[case::integer(json!(1))]
#[case::unsigned(json!(u64::MAX))]
#[case::whole_float(json!(1.0))]
#[case::fraction(json!(0.25))]
#[case::negative_zero(json!(-0.0))]
#[case::maximum(json!(f64::MAX))]
#[case::minimum(json!(-f64::MAX))]
fn number_root_accepts_both_numeric_representations(#[case] input: Value) {
    assert_parity(&schema_of::<f64>().unwrap(), input.clone(), true);
    let output = complete_wire(&schema_of::<f64>().unwrap(), input.clone()).unwrap();
    assert_eq!(
        output.as_number().unwrap().is_f64(),
        input.as_number().unwrap().is_f64()
    );
    assert_eq!(
        output.as_f64().unwrap().to_bits(),
        input.as_f64().unwrap().to_bits()
    );
}

#[rstest]
#[case::boolean_true(schema_of::<bool>, json!(true), true)]
#[case::boolean_false(schema_of::<bool>, json!(false), true)]
#[case::boolean_null(schema_of::<bool>, Value::Null, false)]
#[case::boolean_integer(schema_of::<bool>, json!(1), false)]
#[case::boolean_string(schema_of::<bool>, json!("true"), false)]
#[case::boolean_object(schema_of::<bool>, json!({}), false)]
#[case::boolean_list(schema_of::<bool>, json!([]), false)]
#[case::string_empty(schema_of::<String>, json!(""), true)]
#[case::string_plain(schema_of::<String>, json!("text"), true)]
#[case::string_template_is_data(schema_of::<String>, json!("{{ $input.value }}"), true)]
#[case::string_null(schema_of::<String>, Value::Null, false)]
#[case::string_integer(schema_of::<String>, json!(1), false)]
#[case::string_boolean(schema_of::<String>, json!(true), false)]
#[case::string_object(schema_of::<String>, json!({}), false)]
#[case::string_list(schema_of::<String>, json!([]), false)]
fn nonnumeric_scalar_roots_match(
    #[case] schema: fn() -> Result<ValidSchema, ValidationReport>,
    #[case] input: Value,
    #[case] accepted: bool,
) {
    assert_parity(&schema().unwrap(), input, accepted);
}

#[rstest]
#[case::null(Value::Null)]
#[case::boolean(json!(true))]
#[case::string(json!("{{ $input.value }}"))]
#[case::integer(json!(u64::MAX))]
#[case::float(json!(1.0))]
#[case::object(json!({"": [null, true], "a/b~c": 1}))]
#[case::list(json!([false, {}, "data"]))]
fn unknown_root_remains_any(#[case] input: Value) {
    let schema = schema_of::<Value>().unwrap();
    assert_parity(&schema, input, true);
    let exported = schema.json_schema().unwrap().to_value();
    assert!(exported.get("type").is_none());
    assert!(exported.get("minimum").is_none());
    assert!(exported.get("maximum").is_none());
}

#[rstest]
#[case::minimum(json!(-f64::from(f32::MAX)), true)]
#[case::below_minimum(json!((-f64::from(f32::MAX)).next_down()), false)]
#[case::maximum(json!(f64::from(f32::MAX)), true)]
#[case::above_maximum(json!(f64::from(f32::MAX).next_up()), false)]
#[case::integer(json!(0), true)]
#[case::fraction(json!(0.25), true)]
#[case::negative_zero(json!(-0.0), true)]
#[case::null(Value::Null, false)]
#[case::boolean(json!(false), false)]
#[case::string(json!("0.25"), false)]
fn narrow_float_bounds_are_finite(#[case] input: Value, #[case] accepted: bool) {
    let schema = schema_of::<f32>().unwrap();
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(exported["minimum"], json!(-f64::from(f32::MAX)));
    assert_eq!(exported["maximum"], json!(f64::from(f32::MAX)));
    assert_parity(&schema, input, accepted);
}

#[rstest]
#[case::integer_below(json!(9_007_199_254_740_992_u64), false)]
#[case::float_below(json!(9_007_199_254_740_992.0), false)]
#[case::minimum(json!(9_007_199_254_740_993_u64), true)]
#[case::float_within(json!(9_007_199_254_740_994.0), true)]
#[case::maximum(json!(9_007_199_254_740_995_u64), true)]
#[case::integer_above(json!(9_007_199_254_740_996_u64), false)]
#[case::float_above(json!(9_007_199_254_740_996.0), false)]
fn number_domains_keep_integer_bound_precision(#[case] input: Value, #[case] accepted: bool) {
    let schema = ValidSchema::scalar(
        ScalarSchema::number(9_007_199_254_740_993_u64, 9_007_199_254_740_995_u64).unwrap(),
    )
    .unwrap();
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(exported["type"], "number");
    assert_eq!(exported["minimum"].as_u64(), Some(9_007_199_254_740_993));
    assert_eq!(exported["maximum"].as_u64(), Some(9_007_199_254_740_995));
    assert_parity(&schema, input, accepted);
}

#[rstest]
#[case::minimum(json!(9_007_199_254_740_993_u64), false)]
#[case::effective_minimum(json!(9_007_199_254_740_994_u64), true)]
#[case::effective_maximum(json!(9_007_199_254_740_995_u64), true)]
#[case::maximum(json!(9_007_199_254_740_996_u64), false)]
#[case::out_of_domain(json!(9_007_199_254_740_997_u64), false)]
fn scalar_rules_cannot_replace_domain_bounds(#[case] input: Value, #[case] accepted: bool) {
    let rules = [
        Rule::value(ValueRule::Min(9_007_199_254_740_994_u64.into())).unwrap(),
        Rule::min_value(0),
        Rule::value(ValueRule::Max(9_007_199_254_740_995_u64.into())).unwrap(),
        Rule::value(ValueRule::Max(u64::MAX.into())).unwrap(),
    ];
    let mut scalar =
        ScalarSchema::integer(9_007_199_254_740_993_u64, 9_007_199_254_740_996_u64).unwrap();
    for rule in &rules {
        scalar = scalar.root_rule(rule.clone());
    }
    let schema = ValidSchema::scalar(scalar).unwrap();
    assert_parity(&schema, input, accepted);
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(exported["minimum"], json!(9_007_199_254_740_993_u64));
    assert_eq!(exported["maximum"], json!(9_007_199_254_740_996_u64));
    assert_eq!(
        exported["x-nebula-root-rules"],
        serde_json::to_value(rules).unwrap()
    );
}

#[rstest]
#[case::valid(json!("abc"), true)]
#[case::short(json!("ab"), false)]
#[case::long(json!("abcde"), false)]
#[case::pattern(json!("ABC"), false)]
fn scalar_string_rules_are_projected(#[case] input: Value, #[case] accepted: bool) {
    let rules = [
        Rule::min_length(3),
        Rule::max_length(4),
        Rule::pattern("^[a-z]+$").unwrap(),
    ];
    let mut scalar = ScalarSchema::string();
    for rule in &rules {
        scalar = scalar.root_rule(rule.clone());
    }
    let schema = ValidSchema::scalar(scalar).unwrap();
    assert_parity(&schema, input, accepted);
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(exported["minLength"], 3);
    assert_eq!(exported["maxLength"], 4);
    assert_eq!(exported["pattern"], "^[a-z]+$");
    assert_eq!(
        exported["x-nebula-root-rules"],
        serde_json::to_value(rules).unwrap()
    );
}

#[test]
fn compound_scalar_rules_stay_annotations_not_complete_proofs() {
    let rule = Rule::not(Rule::one_of(["denied"]).unwrap()).unwrap();
    let scalar = ScalarSchema::string().root_rule(rule.clone());
    let schema = ValidSchema::scalar(scalar).unwrap();
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(
        exported["x-nebula-root-rules"],
        serde_json::to_value([rule]).unwrap()
    );
    assert!(
        jsonschema::validator_for(&exported)
            .unwrap()
            .is_valid(&json!("denied"))
    );
    assert!(complete_wire(&schema, json!("denied")).is_err());
    assert_eq!(
        complete_wire(&schema, json!("allowed")).unwrap(),
        json!("allowed")
    );
}
