use nebula_schema::{EngineExpressionContext, Expression, ExpressionContext};

#[test]
fn typed_decode_cause_chain_does_not_publish_input() {
    #[derive(Debug, serde::Deserialize)]
    enum Choice {
        Known,
    }
    #[derive(Debug, serde::Deserialize)]
    struct Typed {
        value: Choice,
    }
    let schema = nebula_schema::Schema::builder()
        .add(nebula_schema::Field::string(nebula_schema::field_key!(
            "value"
        )))
        .build()
        .unwrap();
    let values = nebula_schema::AuthoredValue::from_data(
        serde_json::json!({"value": "PRIVATE_DECODE_SENTINEL"}),
    )
    .unwrap();
    let resolved = schema.validate(values).unwrap().resolve_data().unwrap();
    let error = resolved.into_typed::<Typed>().unwrap_err();
    let mut current: &dyn std::error::Error = &error;
    loop {
        assert!(!format!("{current:?} {current}").contains("PRIVATE_DECODE_SENTINEL"));
        match current.source() {
            Some(source) => current = source,
            None => break,
        }
    }
    let control: Typed = serde_json::from_value(serde_json::json!({"value": "Known"})).unwrap();
    assert!(matches!(control.value, Choice::Known));
}

#[test]
fn expression_debug_does_not_expose_source() {
    let expression = Expression::new("{{ 'SENSITIVE_LITERAL_SENTINEL' }}");
    assert!(!format!("{expression:?}").contains("SENSITIVE_LITERAL_SENTINEL"));
}

#[test]
fn parse_error_does_not_publish_source() {
    let expression = Expression::new("{{ 'SENSITIVE_LITERAL_SENTINEL' + }}");
    let error = expression.parse().unwrap_err();
    assert!(!format!("{error:?}").contains("SENSITIVE_LITERAL_SENTINEL"));
    assert!(!error.to_string().contains("SENSITIVE_LITERAL_SENTINEL"));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("SENSITIVE_LITERAL_SENTINEL")
    );
}

#[tokio::test]
async fn runtime_error_does_not_publish_source() {
    let expression = Expression::new("{{ 'SENSITIVE_LITERAL_SENTINEL' / 0 }}");
    let context = EngineExpressionContext::with_input(serde_json::json!({}));
    let error = context
        .evaluate(expression.parse().unwrap())
        .await
        .unwrap_err();
    assert!(!format!("{error:?}").contains("SENSITIVE_LITERAL_SENTINEL"));
    assert!(!error.to_string().contains("SENSITIVE_LITERAL_SENTINEL"));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("SENSITIVE_LITERAL_SENTINEL")
    );
}
