use nebula_schema::{
    ExpressionMode, Field, HasSchema, Rule, Schema, Transformer, ValidSchema, ValidationReport,
    field_key,
};
use nebula_workflow::ParamValue;
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Deserialize, Serialize)]
struct TransformedRemoteInput {
    label: String,
    literal: String,
    token: String,
}

impl HasSchema for TransformedRemoteInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(
                Field::string(field_key!("label"))
                    .expression_mode(ExpressionMode::Required)
                    .with_transformer(Transformer::regex("^.(.*)$", 1)?)
                    .with_rule(
                        Rule::one_of([json!("ready")]).expect("bounded fixture rule must admit"),
                    ),
            )
            .add(
                Field::string(field_key!("literal"))
                    .with_transformer(Transformer::regex("^.(.*)$", 1)?),
            )
            .add(Field::secret(field_key!("token")).required())
            .build()
    }
}

#[derive(Deserialize, Serialize)]
struct BoundedAmountInput {
    amount: i64,
}

impl HasSchema for BoundedAmountInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::number(field_key!("amount")).integer().min(8))
            .build()
    }
}

fn reject_nine<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let amount = i64::deserialize(deserializer)?;
    if amount == 9 {
        return Err(serde::de::Error::custom(
            "nine is rejected by the Rust input contract",
        ));
    }
    Ok(amount)
}

#[derive(Deserialize, Serialize)]
struct SerdeRejectingRemoteInput {
    #[serde(deserialize_with = "reject_nine")]
    amount: i64,
}

impl HasSchema for SerdeRejectingRemoteInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::number(field_key!("amount")).integer().min(8))
            .build()
    }
}

#[tokio::test]
async fn remote_effect_consumes_resolved_input_with_transforms_once() {
    let fixture = Fixture::with_typed_input::<TransformedRemoteInput>(
        ProviderBehavior::Applied,
        Ports::memory(),
        &[
            ("label", ParamValue::expression("{{ 'xready' }}")),
            ("literal", ParamValue::literal(json!("x{{ 7 }}"))),
            ("token", ParamValue::expression("{{ $input.token }}")),
        ],
    )
    .await;
    let execution = fixture
        .start_with_input(json!({"token": "REMOTE_INPUT_SECRET_CANARY"}))
        .await;
    let result = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(
        fixture.provider.prepared_inputs.lock().as_slice(),
        &[json!({
            "label": "ready", "literal": "{{ 7 }}", "token": "REMOTE_INPUT_SECRET_CANARY"
        })]
    );
    assert_eq!(fixture.provider.calls.lock().len(), 1);
    assert_eq!(fixture.provider.committed.lock().len(), 1);
}

#[tokio::test]
async fn remote_raw_input_is_rejected_before_effect_preparation_or_invocation() {
    let fixture = Fixture::with_typed_input::<BoundedAmountInput>(
        ProviderBehavior::Applied,
        Ports::memory(),
        &[],
    )
    .await;
    let execution = fixture.start().await;
    let result = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Failed);
    assert_eq!(
        result.node_errors[&node_key!("send")],
        "parameter resolution failed for node send, param 'amount': input schema validation failed"
    );
    assert!(fixture.provider.prepared_inputs.lock().is_empty());
    assert!(fixture.provider.calls.lock().is_empty());
    assert!(fixture.provider.committed.lock().is_empty());
}

#[tokio::test]
async fn remote_schema_acceptance_is_not_typed_decode_proof() {
    let fixture = Fixture::with_typed_input::<SerdeRejectingRemoteInput>(
        ProviderBehavior::Applied,
        Ports::memory(),
        &[],
    )
    .await;
    let execution = fixture.start_with_input(json!({"amount": 9})).await;
    let result = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await
        .unwrap();

    assert_eq!(result.status, ExecutionStatus::Failed);
    assert!(fixture.provider.prepared_inputs.lock().is_empty());
    assert!(fixture.provider.calls.lock().is_empty());
    assert!(fixture.provider.committed.lock().is_empty());
}
