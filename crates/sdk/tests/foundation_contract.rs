//! Supported schema and typed authoring contracts.

use nebula_sdk::{
    integration::credential::StaticResolveResult,
    prelude::*,
    serde_json::{from_value, to_value},
};

#[derive(Debug, Deserialize, Serialize, Schema, PartialEq, Eq)]
#[serde(crate = "nebula_sdk::serde")]
pub struct GreetingInput {
    name: String,
}

#[derive(Debug, Deserialize, Serialize, Schema, PartialEq, Eq)]
#[serde(crate = "nebula_sdk::serde")]
pub struct GreetingOutput {
    message: String,
}

#[derive(Debug, Deserialize, Serialize, Schema, PartialEq, Eq)]
#[serde(crate = "nebula_sdk::serde")]
struct UnitInput;

#[derive(Debug, Deserialize, Serialize, Schema, PartialEq, Eq)]
#[serde(crate = "nebula_sdk::serde")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde's empty object contract must remain distinct from a unit's null contract"
)]
struct EmptyRecordInput {}

#[test]
fn curated_schemas_preserve_scalar_unit_and_record_roots() {
    let integer = schema_of::<u8>().expect("u8 has a valid schema");
    let scalar = integer
        .scalar_schema()
        .expect("u8 publishes a scalar schema");
    assert_eq!(scalar.kind(), ScalarKind::Integer);
    assert_eq!(scalar.minimum(), json!(0).as_number());
    assert_eq!(scalar.maximum(), json!(255).as_number());

    for schema in [
        schema_of::<()>().expect("unit has a valid schema"),
        schema_of::<UnitInput>().expect("unit struct has a valid schema"),
    ] {
        let RootShape::Scalar(scalar) = schema.root_shape() else {
            panic!("unit types must publish their null serde shape");
        };
        assert_eq!(scalar.kind(), ScalarKind::Null);
        assert_eq!(
            to_value(&schema).expect("schema serializes"),
            json!({"kind": "scalar", "scalar": {"version": 1, "type": "null"}})
        );
    }

    let record = schema_of::<EmptyRecordInput>().expect("empty record has a valid schema");
    std::assert_matches!(record.root_shape(), RootShape::Record(_));
}

#[test]
fn params_data_preserves_literal_json_without_interpolation() {
    let authored = params! { data;
        "name" => "{{ $input.name }}",
        "object" => json!({"$expr": "{{ 1 / 0 }}"}),
        "" => json!({"a/b~c": null}),
    }
    .expect("literal JSON is valid authored data");

    assert_eq!(
        authored.to_json(),
        json!({
            "name": "{{ $input.name }}",
            "object": {"$expr": "{{ 1 / 0 }}"},
            "": {"a/b~c": null}
        })
    );
}

#[test]
fn params_template_retains_explicit_program_syntax() {
    let expression = Expression::template("{{ 7 }}");
    assert_eq!(expression.syntax(), ProgramSyntax::Template);
    expression.parse().expect("template expression parses");

    let authored = AuthoredValue::Expression(expression.clone());
    let wire = to_value(authored).expect("authored expression serializes");
    assert_eq!(wire["expressions"][0]["syntax"], json!("template"));
    let authored: AuthoredValue = from_value(wire).expect("authored expression deserializes");
    assert_eq!(authored, AuthoredValue::Expression(expression));

    let AuthoredValue::Expression(shorthand) =
        params! { template; "{{ 7 }}" }.expect("template shorthand is valid")
    else {
        panic!("template shorthand must author a program");
    };
    assert_eq!(shorthand.syntax(), ProgramSyntax::Auto);
}

#[test]
fn params_evaluates_inputs_once_and_reports_construction_limits() {
    let mut key_calls = 0;
    let mut value_calls = 0;
    let authored = params! { data;
        { key_calls += 1; "a/b~c" } => { value_calls += 1; 42 },
    }
    .expect("flat authored data is valid");
    assert_eq!((key_calls, value_calls), (1, 1));
    assert_eq!(
        authored.get("a/b~c").and_then(AuthoredValue::as_literal),
        Some(&json!(42))
    );

    let nested = (0..65).fold(Value::Null, |value, _| json!([value]));
    assert_eq!(
        params! { data; nested }
            .expect_err("excessive nesting is rejected")
            .code(),
        "recursion_limit"
    );
    assert_eq!(
        params! { data; }.expect("empty object is valid"),
        AuthoredValue::object()
    );
}

simple_action! {
    name: GreetAction,
    key: "sdk.greet",
    input: GreetingInput,
    output: GreetingOutput,
    async fn execute(&self, input, _context) {
        Ok(ActionResult::success(GreetingOutput {
            message: format!("Hello, {}!", input.name),
        }))
    }
}

#[tokio::test]
async fn simple_action_uses_typed_input_and_output_through_the_sdk_runtime() {
    let draft: ActionMetadataDraft = GreetAction::metadata();
    let _ = draft;
    let context = TestContextBuilder::new().with_input(json!({"name": "Nebula"}));
    let report = TestRuntime::new(context)
        .run_stateless(GreetAction)
        .await
        .expect("typed action runs through the SDK test runtime");

    assert_eq!(report.output, json!({"message": "Hello, Nebula!"}));
}

#[derive(Debug, Deserialize)]
#[serde(crate = "nebula_sdk::serde")]
pub struct InvalidInput;

impl HasSchema for InvalidInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::string(field_key!("duplicate")))
            .add(Field::number(field_key!("duplicate")))
            .build()
    }
}

simple_action! {
    name: InvalidAction,
    key: "sdk.invalid_input",
    input: InvalidInput,
    output: Value,
    async fn execute(&self, _input, _context) {
        Ok(ActionResult::success(Value::Null))
    }
}

#[tokio::test]
async fn test_runtime_preserves_factory_admission_failures() {
    let error = TestRuntime::new(TestContextBuilder::new())
        .run_stateless(InvalidAction)
        .await
        .expect_err("invalid input schema must fail runtime admission");
    assert!(error.is_fatal());
}

#[derive(Debug, Deserialize, Schema)]
#[serde(crate = "nebula_sdk::serde")]
struct TokenProperties {
    token: String,
}

struct TypedCredential;

#[credential(key = "sdk.typed", name = "Typed credential")]
impl TypedCredential {
    type Properties = TokenProperties;
    type Scheme = SecretToken;
    type State = SecretToken;

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        properties: &TokenProperties,
        _context: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            SecretString::new(properties.token.clone()),
        )))
    }
}

#[tokio::test]
async fn credential_author_receives_typed_properties_and_returns_draft_metadata() {
    let draft: CredentialMetadataDraft = TypedCredential::metadata();
    let _ = draft;
    let result = TypedCredential::resolve(
        &TokenProperties {
            token: "literal-token".to_owned(),
        },
        &CredentialContext::for_owner("sdk-test"),
    )
    .await
    .expect("typed credential resolves");

    let StaticResolveResult::Complete(token) = result else {
        panic!("static credential must complete immediately");
    };
    assert_eq!(token.token().expose_secret(), "literal-token");
}
