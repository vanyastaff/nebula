//! Behavioral contracts for structured property authoring.

use nebula_schema::{AuthoredValue, HasSchema, Schema, SecretInput};
use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Schema, Deserialize)]
struct OptionalText {
    #[property(validate(non_empty, length(min = 3, max = 8)))]
    text: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct ProtectedText(String);

impl SecretInput for ProtectedText {}

#[derive(Schema, Deserialize)]
struct OptionalSecret {
    #[property(
        input(secret),
        validate(non_empty, length(min = 3, max = 8), pattern = "^[a-z]+$")
    )]
    text: Option<ProtectedText>,
}

fn authored(value: Value) -> AuthoredValue {
    AuthoredValue::from_data(value).expect("literal input")
}

#[test]
fn non_empty_preserves_optional_absence_and_enforces_both_bounds() {
    let schema = OptionalText::schema().expect("schema");
    let missing = schema
        .validate(authored(json!({})))
        .expect("optional absence")
        .resolve_data()
        .expect("resolve")
        .into_typed::<OptionalText>()
        .expect("decode");
    assert_eq!(missing.text, None);
    for value in ["", "ab", "abcdefghi"] {
        let errors = schema
            .validate(authored(json!({"text": value})))
            .expect_err("length constraint");
        assert!(
            errors
                .errors()
                .any(|error| ["min_length", "max_length"].contains(&error.code()))
        );
    }
    for value in ["abc", "abcdefgh"] {
        let decoded = schema
            .validate(authored(json!({"text": value})))
            .expect("in bounds")
            .resolve_data()
            .expect("resolve")
            .into_typed::<OptionalText>()
            .expect("decode");
        assert_eq!(decoded.text.as_deref(), Some(value));
    }
}

#[test]
fn typed_secret_rules_check_real_values_and_keep_optional_absence() {
    let schema = OptionalSecret::schema().expect("schema");
    let missing = schema
        .validate(authored(json!({})))
        .expect("optional absence")
        .resolve_data()
        .expect("resolve")
        .into_typed_exposing_secrets::<OptionalSecret>()
        .expect("decode");
    assert!(missing.text.is_none());
    for (value, code) in [
        ("", "min_length"),
        ("ab", "min_length"),
        ("abcdefghi", "max_length"),
        ("ABC", "invalid_format"),
    ] {
        let errors = schema
            .validate(authored(json!({"text": value})))
            .expect_err("secret rule");
        assert!(
            errors.errors().any(|error| error.code() == code),
            "{errors:?}"
        );
    }
    let resolved = schema
        .validate(authored(json!({"text": "abc"})))
        .expect("valid secret")
        .resolve_data()
        .expect("resolve");
    assert!(resolved.clone().into_typed::<OptionalSecret>().is_err());
    let decoded = resolved
        .into_typed_exposing_secrets::<OptionalSecret>()
        .expect("trusted decode");
    assert_eq!(
        decoded.text.as_ref().map(|text| text.0.as_str()),
        Some("abc")
    );
}

#[test]
fn invalid_property_declarations_compile_fail() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/property_*.rs");
}

#[test]
fn nested_derive_cannot_remint_historical_schema_policy() {
    struct Historical;
    impl HasSchema for Historical {
        fn schema() -> Result<nebula_schema::ValidSchema, nebula_schema::ValidationReport> {
            Ok(serde_json::from_value(json!({"fields": []})).expect("historical evidence"))
        }
    }

    #[derive(Schema)]
    #[expect(dead_code, reason = "only schema extraction is exercised")]
    struct Record {
        nested: Historical,
    }

    #[derive(Schema)]
    #[expect(dead_code, reason = "only schema extraction is exercised")]
    struct List {
        nested: Vec<Historical>,
    }

    #[derive(Schema)]
    #[expect(dead_code, reason = "only schema extraction is exercised")]
    enum Union {
        Nested(Historical),
    }

    for result in [Record::schema(), List::schema(), Union::schema()] {
        let errors = result.expect_err("historical evidence cannot be embedded into a new schema");
        assert!(
            errors
                .errors()
                .any(|error| error.code() == "schema.unsupported_policy")
        );
    }
}

#[derive(Schema, Deserialize)]
struct ExpressionBoolean {
    #[property(input(expressions = allowed))]
    enabled: bool,
}

#[derive(Debug, PartialEq, nebula_schema::EnumSelect, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Choice {
    First,
    Second,
}

#[derive(Schema, Deserialize)]
struct ExpressionSelect {
    #[field(enum_select)]
    #[property(input(expressions = allowed))]
    choice: Choice,
}

struct ConstantExpression(Value);

impl nebula_schema::ExpressionContext for ConstantExpression {
    fn evaluate<'a>(
        &'a self,
        _program: &'a nebula_schema::CompiledProgram,
    ) -> nebula_schema::EvalFuture<'a> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

#[tokio::test]
async fn explicit_allowed_boolean_expressions_validate_and_resolve() {
    let schema = ExpressionBoolean::schema().expect("boolean schema");
    let expression = || {
        AuthoredValue::from_template_json(json!({"enabled": {"$expr": "{{ $value }}"}}))
            .expect("expression input")
    };
    let checked = schema
        .validate(expression())
        .expect("explicitly allowed expression");
    let decoded = checked
        .resolve(&ConstantExpression(json!(true)))
        .await
        .expect("resolved boolean")
        .into_typed::<ExpressionBoolean>()
        .expect("decode");
    assert!(decoded.enabled);
    let errors = schema
        .validate(expression())
        .expect("allowed expression")
        .resolve(&ConstantExpression(json!("wrong type")))
        .await
        .expect_err("expression allowance must not waive the boolean contract");
    assert!(
        errors
            .errors()
            .any(|error| error.path().to_string() == "/enabled")
    );
}

#[tokio::test]
async fn explicit_allowed_select_expressions_validate_and_resolve() {
    let schema = ExpressionSelect::schema().expect("select schema");
    let expression = || {
        AuthoredValue::from_template_json(json!({"choice": {"$expr": "{{ $value }}"}}))
            .expect("expression input")
    };
    let checked = schema
        .validate(expression())
        .expect("explicitly allowed expression");
    let decoded = checked
        .resolve(&ConstantExpression(json!("second")))
        .await
        .expect("resolved selection")
        .into_typed::<ExpressionSelect>()
        .expect("decode");
    assert_eq!(decoded.choice, Choice::Second);
    let errors = schema
        .validate(expression())
        .expect("allowed expression")
        .resolve(&ConstantExpression(json!("not_a_choice")))
        .await
        .expect_err("expression allowance must not waive static membership");
    assert!(
        errors
            .errors()
            .any(|error| error.path().to_string() == "/choice")
    );
}

#[derive(Schema, Deserialize)]
struct HalfOpenFloat {
    #[property(validate(range(0..1)))]
    value: f64,
    #[validate(range(0..1))]
    narrow: Option<f32>,
}

#[derive(Schema, Deserialize)]
struct HalfOpenInteger {
    #[property(validate(range(0..2)))]
    value: u8,
}

#[test]
fn half_open_float_range_accepts_fractions_and_rejects_the_upper_endpoint() {
    let schema = HalfOpenFloat::schema().expect("float range schema");
    let decoded = schema
        .validate(authored(json!({"value": 0.5, "narrow": 0.5})))
        .expect("fraction below the exclusive endpoint")
        .resolve_data()
        .expect("resolve")
        .into_typed::<HalfOpenFloat>()
        .expect("decode");
    assert_eq!(decoded.value, 0.5);
    assert_eq!(decoded.narrow, Some(0.5));
    for (input, path) in [
        (json!({"value": 1.0}), "/value"),
        (json!({"value": 0.5, "narrow": 1.0}), "/narrow"),
    ] {
        let errors = schema
            .validate(authored(input))
            .expect_err("exclusive upper endpoint");
        assert!(
            errors
                .errors()
                .any(|error| error.code() == "less_than" && error.path().to_string() == path),
            "{errors:?}"
        );
    }
}

#[test]
fn half_open_integer_range_keeps_discrete_boundaries() {
    let schema = HalfOpenInteger::schema().expect("integer range schema");
    for value in [0, 1] {
        let decoded = schema
            .validate(authored(json!({"value": value})))
            .expect("integer in range")
            .resolve_data()
            .expect("resolve")
            .into_typed::<HalfOpenInteger>()
            .expect("decode");
        assert_eq!(decoded.value, value);
    }
    for value in [-1, 2] {
        let errors = schema
            .validate(authored(json!({"value": value})))
            .expect_err("integer outside the half-open range");
        assert!(
            errors
                .errors()
                .any(|error| error.path().to_string() == "/value")
        );
    }
}

#[test]
fn standalone_enum_select_accepts_property_display_metadata() {
    trybuild::TestCases::new().pass("tests/compile_pass/property_enum_select_display.rs");
}
