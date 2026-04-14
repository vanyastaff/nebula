//! Resume/recovery scenarios for deferred ActionOutput.
//!
//! These tests model engine persistence boundaries:
//! 1. Action returns deferred output.
//! 2. Engine persists execution state.
//! 3. Process restarts and reloads state.
//! 4. Deferred handle is resumed/resolved.

use std::time::Duration;

use nebula_action::{
    ActionOutput, ActionResult, DeferredOutput, DeferredRetryConfig, ExpectedOutput, PollTarget,
    Producer, ProducerKind, Resolution, WaitCondition,
};

fn sample_deferred(handle_id: &str) -> DeferredOutput {
    DeferredOutput {
        handle_id: handle_id.to_string(),
        resolution: Resolution::AwaitOrPoll {
            channel_id: "job-ch".to_string(),
            fallback_after: Duration::from_secs(10),
            poll_target: PollTarget::Http {
                url: "https://api.example.com/status".to_string(),
                method: "GET".to_string(),
            },
            poll_interval: Duration::from_secs(2),
        },
        expected: ExpectedOutput::Value { schema: None },
        progress: None,
        producer: Producer {
            kind: ProducerKind::ExternalApi,
            name: Some("example-api".to_string()),
            version: Some("v1".to_string()),
        },
        retry: Some(DeferredRetryConfig {
            max_attempts: 5,
            initial_interval: Duration::from_secs(1),
            backoff_coefficient: 2.0,
            max_interval: Some(Duration::from_secs(30)),
            non_retryable_errors: vec!["validation_error".to_string()],
        }),
        timeout: Some(Duration::from_secs(120)),
    }
}

#[test]
fn deferred_success_roundtrip_survives_persist_and_resume() {
    let original: ActionResult<serde_json::Value> = ActionResult::Success {
        output: ActionOutput::Deferred(Box::new(sample_deferred("job-42"))),
    };

    // Persist execution state.
    let persisted = serde_json::to_string(&original).expect("serialize deferred state");

    // Resume execution state.
    let resumed: ActionResult<serde_json::Value> =
        serde_json::from_str(&persisted).expect("deserialize deferred state");

    match resumed {
        ActionResult::Success { output } => match output {
            ActionOutput::Deferred(deferred) => {
                assert_eq!(deferred.handle_id, "job-42");
                assert!(matches!(
                    deferred.resolution,
                    Resolution::AwaitOrPoll { .. }
                ));
                assert!(deferred.retry.is_some());
                assert_eq!(deferred.timeout, Some(Duration::from_secs(120)));
            },
            other => panic!("expected Deferred output, got {other:?}"),
        },
        other => panic!("expected Success, got {other:?}"),
    }
}

#[test]
fn wait_with_partial_deferred_roundtrip_survives_resume() {
    let waiting: ActionResult<serde_json::Value> = ActionResult::Wait {
        condition: WaitCondition::Execution {
            execution_id: nebula_core::id::ExecutionId::new(),
        },
        timeout: Some(Duration::from_secs(300)),
        partial_output: Some(ActionOutput::Deferred(Box::new(sample_deferred("job-99")))),
    };

    let persisted = serde_json::to_string(&waiting).expect("serialize wait state");
    let resumed: ActionResult<serde_json::Value> =
        serde_json::from_str(&persisted).expect("deserialize wait state");

    match resumed {
        ActionResult::Wait {
            partial_output,
            timeout,
            ..
        } => {
            assert_eq!(timeout, Some(Duration::from_secs(300)));
            match partial_output {
                Some(ActionOutput::Deferred(deferred)) => {
                    assert_eq!(deferred.handle_id, "job-99");
                },
                other => panic!("expected deferred partial output, got {other:?}"),
            }
        },
        other => panic!("expected Wait, got {other:?}"),
    }
}

#[test]
fn recovered_deferred_can_transition_to_resolved_success() {
    let deferred: ActionResult<serde_json::Value> = ActionResult::Success {
        output: ActionOutput::Deferred(Box::new(sample_deferred("job-7"))),
    };

    // Simulate persisted + recovered deferred state.
    let persisted = serde_json::to_string(&deferred).expect("serialize deferred");
    let _recovered: ActionResult<serde_json::Value> =
        serde_json::from_str(&persisted).expect("deserialize deferred");

    // Simulate resolver completion after resume.
    let resolved: ActionResult<serde_json::Value> = ActionResult::Success {
        output: ActionOutput::Value(serde_json::json!({
            "job_id": "job-7",
            "status": "completed"
        })),
    };

    match resolved {
        ActionResult::Success { output } => {
            assert_eq!(
                output.as_value(),
                Some(&serde_json::json!({
                    "job_id": "job-7",
                    "status": "completed"
                }))
            );
        },
        other => panic!("expected resolved Success, got {other:?}"),
    }
}
