use super::*;
use nebula_core::OperationCallId;

fn source() -> OutcomeEvidenceSource {
    OutcomeEvidenceSource::Invocation(OperationCallId::from_bytes([7; 16]))
}

fn operation() -> OperationId {
    OperationId::from_bytes([9; 16])
}

#[test]
fn exact_json_outputs_preserve_null_and_nested_values() {
    for value in [
        Value::Null,
        serde_json::json!({"nested": [null, 42, "value"]}),
    ] {
        let recorded = applied(
            operation(),
            source(),
            Some(Box::new(ActionResult::Success {
                output: ActionOutput::Value(value.clone()),
            })),
        )
        .unwrap();
        let replayed = replay(operation(), &recorded).unwrap();
        assert!(
            matches!(replayed, ActionResult::Success { output: ActionOutput::Value(output) } if output == value)
        );
    }
}

#[test]
fn oversized_applied_output_retains_terminal_fact() {
    let recorded = applied(
        operation(),
        source(),
        Some(Box::new(ActionResult::Success {
            output: ActionOutput::Value(Value::String("x".repeat(MAX_EVIDENCE_BYTES))),
        })),
    )
    .unwrap();
    assert_eq!(recorded.outcome(), KnownOutcome::Succeeded);
    assert!(recorded.payload().len() < 100);
    assert_eq!(
        replay(operation(), &recorded).unwrap_err(),
        EffectExecutionError::OutputUnavailable {
            operation_id: operation()
        }
    );
}

#[test]
fn known_applied_without_output_never_replays_as_success() {
    let recorded = applied(operation(), source(), None).unwrap();
    assert_eq!(recorded.outcome(), KnownOutcome::Succeeded);
    assert_eq!(
        replay(operation(), &recorded).unwrap_err(),
        EffectExecutionError::OutputUnavailable {
            operation_id: operation()
        }
    );
}

#[test]
fn replay_rejects_contradictory_and_unknown_evidence() {
    for payload in [
        br#"{"type":"Rejected","code":"Rejected"}"#.to_vec(),
        br#"{"type":"OutputUnavailable","unexpected":true}"#.to_vec(),
        br#"{"type":"FutureOutcome"}"#.to_vec(),
    ] {
        let recorded =
            FrozenOutcomeEvidence::v1_json(source(), KnownOutcome::Succeeded, payload).unwrap();
        assert_eq!(
            replay(operation(), &recorded).unwrap_err(),
            EffectExecutionError::InvalidEvidence
        );
    }
}

#[test]
fn rejection_replays_original_bounded_code() {
    let recorded = rejected(operation(), source(), EffectFailureCode::InvalidRequest).unwrap();
    assert_eq!(recorded.outcome(), KnownOutcome::Failed);
    assert_eq!(
        replay(operation(), &recorded).unwrap_err(),
        EffectExecutionError::Rejected {
            operation_id: operation(),
            code: EffectFailureCode::InvalidRequest,
        }
    );
}

#[test]
fn replay_rejects_evidence_bound_to_another_operation() {
    let recorded = applied(
        operation(),
        source(),
        Some(Box::new(ActionResult::Success {
            output: ActionOutput::Value(serde_json::json!({"status": "applied"})),
        })),
    )
    .unwrap();

    assert_eq!(
        replay(OperationId::from_bytes([8; 16]), &recorded).unwrap_err(),
        EffectExecutionError::InvalidEvidence
    );
}
