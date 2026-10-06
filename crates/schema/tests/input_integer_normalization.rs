//! Integer input preparation shares the exact scalar normalization boundary.

use nebula_schema::{
    AuthoredValue, EvalFuture, Expression, ExpressionContext, InputContract, OutputContract,
    ValueTree, schema_type,
};
use serde_json::{Value, json};

#[schema_type(both)]
struct NestedIntegers {
    values: Vec<Option<i64>>,
}

#[schema_type(input)]
struct EvaluatedInteger {
    #[field(expression_required)]
    value: i64,
}

struct Evaluation(Value);
impl ExpressionContext for Evaluation {
    fn evaluate<'a>(&'a self, _: &'a nebula_schema::CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

#[test]
fn integral_input_numbers_normalize_without_saturating_or_coercing_strings() {
    let input = InputContract::for_type::<i64>().unwrap();
    for (authored, expected) in [(json!(1.0), 1), (json!(i64::MIN as f64), i64::MIN)] {
        let proof = input.validate_data(authored).unwrap();
        assert_eq!(proof.into_typed::<i64>(&input).unwrap(), expected);
    }
    for invalid in [
        json!(1.5),
        json!(u64::MAX),
        json!(i64::MAX as f64),
        json!("1"),
    ] {
        assert!(input.validate_data(invalid).is_err());
    }
    let unsigned = InputContract::for_type::<u64>().unwrap();
    assert_eq!(
        unsigned
            .validate_data(json!(u64::MAX))
            .unwrap()
            .into_typed::<u64>(&unsigned)
            .unwrap(),
        u64::MAX
    );
    assert!(unsigned.validate_data(json!(u64::MAX as f64)).is_err());
}

#[test]
fn nested_optional_array_elements_normalize_before_validation_and_decode() {
    let input = InputContract::for_type::<NestedIntegers>().unwrap();
    let proof = input
        .validate_data(json!({"values":[1.0,null,-2.0]}))
        .unwrap();
    assert_eq!(
        proof.into_typed::<NestedIntegers>(&input).unwrap().values,
        vec![Some(1), None, Some(-2)]
    );
    for invalid in [json!(1.5), json!(u64::MAX), json!("1")] {
        assert!(input.validate_data(json!({"values":[invalid]})).is_err());
    }
}

#[tokio::test]
async fn evaluated_integer_numbers_use_the_same_preparation_and_range_checks() {
    let input = InputContract::for_type::<EvaluatedInteger>().unwrap();
    let authored = || {
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "value".to_owned(),
            ValueTree::Expression(Expression::new("{{ 1.0 }}")),
        );
        AuthoredValue::Object(fields)
    };
    let proof = input
        .validate(authored())
        .unwrap()
        .resolve(&Evaluation(json!(1.0)))
        .await
        .unwrap();
    assert_eq!(
        proof.into_typed::<EvaluatedInteger>(&input).unwrap().value,
        1
    );
    for invalid in [json!(1.5), json!(u64::MAX), json!("1")] {
        assert!(
            input
                .validate(authored())
                .unwrap()
                .resolve(&Evaluation(invalid))
                .await
                .is_err()
        );
    }
}

#[test]
fn output_integer_literals_remain_exact_and_number_input_stays_floating_point() {
    let output = OutputContract::for_type::<i64>().unwrap();
    assert!(output.validate_data(&json!(1.0)).is_err());
    output.validate_data(&json!(1)).unwrap();
    let input = InputContract::for_type::<f64>().unwrap();
    let wire = input
        .validate_data(json!(1.0))
        .unwrap()
        .into_wire_data()
        .unwrap();
    assert!(wire.as_number().unwrap().is_f64());
}
