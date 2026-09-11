//! Contracts at the authored-input, expression-result, and schema boundaries.

use nebula_schema::{
    AuthoredValue, EngineExpressionContext, ExpressionMode, Field, ScalarValue, Schema,
    Transformer, field_key,
};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::string(Field::string(field_key!("value")).into())]
#[case::secret(Field::secret(field_key!("value")).into())]
#[case::code(Field::code(field_key!("value")).into())]
#[case::number(Field::number(field_key!("value")).into())]
#[case::boolean(Field::boolean(field_key!("value")).into())]
fn scalar_fields_reject_structured_values(
    #[case] field: Field,
    #[values(json!({}), json!([]))] value: Value,
) {
    let schema = Schema::builder().add(field).build().unwrap();
    let values = AuthoredValue::from_template_json(json!({"value": value})).unwrap();

    let report = schema.validate(values).unwrap_err();
    let errors: Vec<_> = report
        .errors()
        .map(|issue| (issue.path().to_string(), issue.code().to_string()))
        .collect();
    assert_eq!(errors, [("/value".to_owned(), "type_mismatch".to_owned())]);
}

#[test]
fn literal_json_list_cannot_bypass_item_validation() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("items")).item(Field::number(field_key!("item"))))
        .build()
        .unwrap();
    assert!(ScalarValue::try_from(json!(["wrong"])).is_err());
    let values = AuthoredValue::from_data(json!({"items": ["wrong"]})).unwrap();

    let report = schema.validate(values).unwrap_err();
    let errors: Vec<_> = report
        .errors()
        .map(|issue| (issue.path().to_string(), issue.code().to_string()))
        .collect();
    assert_eq!(
        errors,
        [("/items/0".to_owned(), "type_mismatch".to_owned())]
    );
}

#[tokio::test]
async fn required_expression_is_checked_as_data_after_evaluation() {
    let schema = Schema::builder()
        .add(
            Field::number(field_key!("value"))
                .expression_mode(ExpressionMode::Required)
                .min(1),
        )
        .build()
        .unwrap();
    let values =
        AuthoredValue::from_template_json(json!({"value": {"$expr": "{{ $input.value }}"}}))
            .unwrap();
    let valid = schema.validate(values).unwrap();
    let context = EngineExpressionContext::with_input(json!({"value": 7}));

    let resolved = valid.resolve(&context).await.unwrap();
    assert_eq!(resolved.get(&field_key!("value")), Some(&json!(7)));
}

#[tokio::test]
async fn required_expression_still_checks_the_result_type() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("value")).expression_mode(ExpressionMode::Required))
        .build()
        .unwrap();
    let values =
        AuthoredValue::from_template_json(json!({"value": {"$expr": "{{ $input.value }}"}}))
            .unwrap();
    let valid = schema.validate(values).unwrap();
    let context = EngineExpressionContext::with_input(json!({"value": "wrong"}));

    let report = valid.resolve(&context).await.unwrap_err();
    let errors: Vec<_> = report
        .errors()
        .map(|issue| (issue.path().to_string(), issue.code().to_string()))
        .collect();
    assert_eq!(
        errors,
        [("/value".to_owned(), "expression.type_mismatch".to_owned())]
    );
}

#[tokio::test]
async fn expression_object_is_checked_recursively_without_reinterpreting_strings() {
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("config"))
                .expression_mode(ExpressionMode::Allowed)
                .add(
                    Field::string(field_key!("template"))
                        .no_expression()
                        .required(),
                )
                .add(Field::number(field_key!("count")).required()),
        )
        .build()
        .unwrap();
    let values =
        AuthoredValue::from_template_json(json!({"config": {"$expr": "{{ $input.config }}"}}))
            .unwrap();
    let valid = schema.validate(values).unwrap();
    let context = EngineExpressionContext::with_input(json!({
        "config": {"template": "{{ literal }}", "count": 3}
    }));

    let resolved = valid.resolve(&context).await.unwrap();
    assert_eq!(
        resolved.into_json(),
        json!({"config": {"template": "{{ literal }}", "count": 3}})
    );
}

#[tokio::test]
async fn expression_list_cannot_bypass_item_validation() {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("items"))
                .expression_mode(ExpressionMode::Allowed)
                .item(Field::number(field_key!("item"))),
        )
        .build()
        .unwrap();
    let values =
        AuthoredValue::from_template_json(json!({"items": {"$expr": "{{ $input.items }}"}}))
            .unwrap();
    let valid = schema.validate(values).unwrap();
    let context = EngineExpressionContext::with_input(json!({"items": ["wrong"]}));

    let report = valid.resolve(&context).await.unwrap_err();
    let errors: Vec<_> = report
        .errors()
        .map(|issue| (issue.path().to_string(), issue.code().to_string()))
        .collect();
    assert_eq!(
        errors,
        [("/items/0".to_owned(), "expression.type_mismatch".to_owned())]
    );
}

#[tokio::test]
async fn normalization_retains_transformations_and_does_not_repeat_them() {
    let replacement = Transformer::Replace {
        from: "a".into(),
        to: "aa".into(),
    };
    let schema = Schema::builder()
        .add(Field::string(field_key!("literal")).with_transformer(replacement.clone()))
        .add(Field::string(field_key!("evaluated")).with_transformer(replacement))
        .build()
        .unwrap();
    let values = AuthoredValue::from_template_json(json!({
        "literal": "a",
        "evaluated": {"$expr": "{{ $input.value }}"}
    }))
    .unwrap();
    let valid = schema.validate(values).unwrap();
    assert_eq!(
        valid
            .get(&field_key!("literal"))
            .and_then(|value| value.as_literal()),
        Some(&json!("aa"))
    );

    let context = EngineExpressionContext::with_input(json!({"value": "a"}));
    let resolved = valid.resolve(&context).await.unwrap();
    assert_eq!(
        resolved.into_json(),
        json!({"literal": "aa", "evaluated": "aa"})
    );
}

#[test]
fn validated_secret_material_is_already_redacted() {
    let schema = Schema::builder()
        .add(Field::secret(field_key!("token")).required())
        .build()
        .unwrap();
    let values =
        AuthoredValue::from_template_json(json!({"token": "foundation-secret-material"})).unwrap();

    let valid = schema.validate(values).unwrap();
    assert!(!format!("{valid:?}").contains("foundation-secret-material"));
    assert_eq!(valid.values().to_json(), json!({"token": "<redacted>"}));
}
