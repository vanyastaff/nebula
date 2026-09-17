use crate::{branch_key, port_key};

use super::*;

#[test]
fn success_result() {
    let result = ActionResult::success(42);
    assert!(result.is_success());
    assert!(!result.is_continue());
    assert!(!result.is_waiting());
}

#[test]
fn skip_result() {
    let result: ActionResult<()> = ActionResult::skip("no data");
    match &result {
        ActionResult::Skip { reason, output } => {
            assert_eq!(reason, "no data");
            assert!(output.is_none());
        },
        _ => panic!("expected Skip"),
    }
}

#[test]
fn skip_with_output() {
    let result = ActionResult::skip_with_output("filtered", vec![1, 2, 3]);
    match &result {
        ActionResult::Skip { reason, output } => {
            assert_eq!(reason, "filtered");
            assert_eq!(output.as_ref().unwrap().as_value().unwrap(), &vec![1, 2, 3]);
        },
        _ => panic!("expected Skip"),
    }
}

#[test]
fn continue_result() {
    let result: ActionResult<String> = ActionResult::Continue {
        output: ActionOutput::Value("partial".into()),
        progress: Some(0.5),
        delay: Some(Duration::from_secs(1)),
    };
    assert!(result.is_continue());
    assert!(!result.is_success());
}

#[test]
fn break_result() {
    let result: ActionResult<i32> = ActionResult::Break {
        output: ActionOutput::Value(100),
        reason: BreakReason::MaxIterations,
    };
    assert!(!result.is_continue());
    match &result {
        ActionResult::Break { reason, .. } => {
            assert_eq!(reason, &BreakReason::MaxIterations);
        },
        _ => panic!("expected Break"),
    }
}

#[test]
fn branch_result() {
    let mut alts = HashMap::new();
    alts.insert(branch_key!("true"), ActionOutput::Value("yes"));
    alts.insert(branch_key!("false"), ActionOutput::Value("no"));
    let result = ActionResult::Branch {
        selected: branch_key!("true"),
        output: ActionOutput::Value("yes"),
        alternatives: alts,
    };
    match result {
        ActionResult::Branch {
            selected,
            alternatives,
            ..
        } => {
            assert_eq!(selected.as_str(), "true");
            assert_eq!(alternatives.len(), 2);
        },
        _ => panic!("expected Branch"),
    }
}

#[test]
fn route_result() {
    let result = ActionResult::Route {
        port: port_key!("error"),
        data: ActionOutput::Value("something failed"),
    };
    assert!(!result.is_success());
}

#[test]
fn multi_output_result() {
    let mut outputs = HashMap::new();
    outputs.insert(port_key!("main"), ActionOutput::Value(1));
    outputs.insert(port_key!("audit"), ActionOutput::Value(2));
    let result = ActionResult::MultiOutput {
        outputs,
        main_output: Some(ActionOutput::Value(1)),
    };
    assert!(!result.is_success());
}

#[test]
fn wait_result() {
    let result: ActionResult<()> = ActionResult::Wait {
        condition: WaitCondition::Duration {
            duration: Duration::from_mins(1),
        },
        timeout: Some(Duration::from_mins(5)),
        partial_output: None,
    };
    assert!(result.is_waiting());
}

#[test]
fn break_reason_equality() {
    assert_eq!(BreakReason::Completed, BreakReason::Completed);
    assert_ne!(BreakReason::Completed, BreakReason::MaxIterations);
    assert_eq!(
        BreakReason::Custom("done".into()),
        BreakReason::Custom("done".into())
    );
}

// ── map_output tests ────────────────────────────────────────────

#[test]
fn map_output_success() {
    let r = ActionResult::success(5);
    let mapped = r.map_output(|n| n * 2);
    match mapped {
        ActionResult::Success { output } => assert_eq!(output.into_value(), Some(10)),
        _ => panic!("expected Success"),
    }
}

#[test]
fn map_output_skip() {
    let r = ActionResult::skip_with_output("skip", 3);
    let mapped = r.map_output(|n| n.to_string());
    match mapped {
        ActionResult::Skip { reason, output } => {
            assert_eq!(reason, "skip");
            assert_eq!(output.unwrap().as_value().map(String::as_str), Some("3"));
        },
        _ => panic!("expected Skip"),
    }
}

#[test]
fn map_output_skip_none() {
    let r: ActionResult<i32> = ActionResult::skip("no output");
    let mapped = r.map_output(|n| n.to_string());
    match mapped {
        ActionResult::Skip { output, .. } => assert!(output.is_none()),
        _ => panic!("expected Skip"),
    }
}

#[test]
fn map_output_continue() {
    let r: ActionResult<i32> = ActionResult::Continue {
        output: ActionOutput::Value(7),
        progress: Some(0.5),
        delay: Some(Duration::from_secs(1)),
    };
    let mapped = r.map_output(|n| n + 1);
    match mapped {
        ActionResult::Continue {
            output,
            progress,
            delay,
        } => {
            assert_eq!(output.into_value(), Some(8));
            assert_eq!(progress, Some(0.5));
            assert_eq!(delay, Some(Duration::from_secs(1)));
        },
        _ => panic!("expected Continue"),
    }
}

#[test]
fn map_output_break() {
    let r: ActionResult<i32> = ActionResult::Break {
        output: ActionOutput::Value(42),
        reason: BreakReason::Completed,
    };
    let mapped = r.map_output(|n| format!("result:{n}"));
    match mapped {
        ActionResult::Break { output, reason } => {
            assert_eq!(output.as_value().map(String::as_str), Some("result:42"));
            assert_eq!(reason, BreakReason::Completed);
        },
        _ => panic!("expected Break"),
    }
}

#[test]
fn map_output_branch() {
    let mut alts = HashMap::new();
    alts.insert(branch_key!("a"), ActionOutput::Value(1));
    alts.insert(branch_key!("b"), ActionOutput::Value(2));
    let r = ActionResult::Branch {
        selected: branch_key!("a"),
        output: ActionOutput::Value(10),
        alternatives: alts,
    };
    let mapped = r.map_output(|n| n * 10);
    match mapped {
        ActionResult::Branch {
            selected,
            output,
            alternatives,
        } => {
            assert_eq!(selected.as_str(), "a");
            assert_eq!(output.into_value(), Some(100));
            assert_eq!(alternatives["a"].as_value(), Some(&10));
            assert_eq!(alternatives["b"].as_value(), Some(&20));
        },
        _ => panic!("expected Branch"),
    }
}

#[test]
fn map_output_route() {
    let r = ActionResult::Route {
        port: port_key!("out"),
        data: ActionOutput::Value(99),
    };
    let mapped = r.map_output(|n| n as f64);
    match mapped {
        ActionResult::Route { port, data } => {
            assert_eq!(port.as_str(), "out");
            assert_eq!(data.into_value(), Some(99.0));
        },
        _ => panic!("expected Route"),
    }
}

#[test]
fn map_output_multi_output() {
    let mut outputs = HashMap::new();
    outputs.insert(port_key!("x"), ActionOutput::Value(1));
    let r = ActionResult::MultiOutput {
        outputs,
        main_output: Some(ActionOutput::Value(0)),
    };
    let mapped = r.map_output(|n| n + 100);
    match mapped {
        ActionResult::MultiOutput {
            outputs,
            main_output,
        } => {
            assert_eq!(outputs["x"].as_value(), Some(&101));
            assert_eq!(main_output.unwrap().into_value(), Some(100));
        },
        _ => panic!("expected MultiOutput"),
    }
}

#[test]
fn map_output_wait() {
    let r: ActionResult<String> = ActionResult::Wait {
        condition: WaitCondition::Duration {
            duration: Duration::from_mins(1),
        },
        timeout: Some(Duration::from_mins(5)),
        partial_output: Some(ActionOutput::Value("partial".into())),
    };
    let mapped = r.map_output(|s| s.len());
    match mapped {
        ActionResult::Wait {
            partial_output,
            timeout,
            ..
        } => {
            assert_eq!(partial_output.unwrap().into_value(), Some(7));
            assert_eq!(timeout, Some(Duration::from_mins(5)));
        },
        _ => panic!("expected Wait"),
    }
}

// ── try_map_output tests ─────────────────────────────────────────

#[test]
fn try_map_output_success_ok() {
    let r = ActionResult::success(5);
    let mapped = r.try_map_output(|n| Ok::<_, String>(n * 2));
    match mapped.unwrap() {
        ActionResult::Success { output } => assert_eq!(output.into_value(), Some(10)),
        _ => panic!("expected Success"),
    }
}

#[test]
fn try_map_output_success_err() {
    let r = ActionResult::success(5);
    let mapped = r.try_map_output(|_| Err::<i32, _>("serialization failed"));
    assert_eq!(mapped.unwrap_err(), "serialization failed");
}

#[test]
fn try_map_output_skip_with_output() {
    let r = ActionResult::skip_with_output("filtered", 3);
    let mapped = r.try_map_output(|n| Ok::<_, String>(n.to_string()));
    match mapped.unwrap() {
        ActionResult::Skip { reason, output } => {
            assert_eq!(reason, "filtered");
            assert_eq!(output.unwrap().as_value().map(String::as_str), Some("3"));
        },
        _ => panic!("expected Skip"),
    }
}

#[test]
fn try_map_output_skip_none() {
    let r: ActionResult<i32> = ActionResult::skip("no output");
    let mapped = r.try_map_output(|n| Ok::<_, String>(n.to_string()));
    match mapped.unwrap() {
        ActionResult::Skip { output, .. } => assert!(output.is_none()),
        _ => panic!("expected Skip"),
    }
}

#[test]
fn try_map_output_branch_partial_failure() {
    let mut alts = HashMap::new();
    alts.insert(branch_key!("a"), ActionOutput::Value(1));
    alts.insert(branch_key!("b"), ActionOutput::Value(2));
    let r = ActionResult::Branch {
        selected: branch_key!("a"),
        output: ActionOutput::Value(10),
        alternatives: alts,
    };
    // Fail on value 2 to test short-circuit
    let mapped = r.try_map_output(|n| if n == 2 { Err("bad value") } else { Ok(n * 10) });
    assert_eq!(mapped.unwrap_err(), "bad value");
}

// ── into_primary_output tests ────────────────────────────────────

#[test]
fn into_primary_output_success() {
    let r = ActionResult::success(42);
    let out = r.into_primary_output().unwrap();
    assert_eq!(out.into_value(), Some(42));
}

#[test]
fn into_primary_output_skip_some() {
    let r = ActionResult::skip_with_output("reason", 7);
    let out = r.into_primary_output().unwrap();
    assert_eq!(out.into_value(), Some(7));
}

#[test]
fn into_primary_output_skip_none() {
    let r: ActionResult<i32> = ActionResult::skip("no data");
    assert!(r.into_primary_output().is_none());
}

#[test]
fn into_primary_output_continue() {
    let r: ActionResult<i32> = ActionResult::Continue {
        output: ActionOutput::Value(99),
        progress: Some(0.5),
        delay: None,
    };
    let out = r.into_primary_output().unwrap();
    assert_eq!(out.into_value(), Some(99));
}

#[test]
fn into_primary_output_branch() {
    let r = ActionResult::Branch {
        selected: branch_key!("a"),
        output: ActionOutput::Value(10),
        alternatives: HashMap::new(),
    };
    let out = r.into_primary_output().unwrap();
    assert_eq!(out.into_value(), Some(10));
}

#[test]
fn into_primary_output_route() {
    let r = ActionResult::Route {
        port: port_key!("out"),
        data: ActionOutput::Value(55),
    };
    let out = r.into_primary_output().unwrap();
    assert_eq!(out.into_value(), Some(55));
}

#[test]
fn into_primary_output_wait_none() {
    let r: ActionResult<i32> = ActionResult::Wait {
        condition: WaitCondition::Duration {
            duration: Duration::from_mins(1),
        },
        timeout: None,
        partial_output: None,
    };
    assert!(r.into_primary_output().is_none());
}

#[test]
fn into_primary_output_wait_some() {
    let r: ActionResult<i32> = ActionResult::Wait {
        condition: WaitCondition::Duration {
            duration: Duration::from_mins(1),
        },
        timeout: None,
        partial_output: Some(ActionOutput::Value(33)),
    };
    let out = r.into_primary_output().unwrap();
    assert_eq!(out.into_value(), Some(33));
}

// ── success_binary / success_reference / success_empty tests ─────

#[test]
fn success_binary_result() {
    use crate::output::{BinaryData, BinaryStorage};
    let r: ActionResult<i32> = ActionResult::success_binary(BinaryData {
        content_type: "image/png".into(),
        data: BinaryStorage::Inline(vec![1, 2, 3]),
        size: 3,
        metadata: None,
    });
    assert!(r.is_success());
    match r {
        ActionResult::Success { output } => assert!(output.is_binary()),
        _ => panic!("expected Success"),
    }
}

#[test]
fn success_reference_result() {
    use crate::output::DataReference;
    let r: ActionResult<i32> = ActionResult::success_reference(DataReference {
        storage_type: "s3".into(),
        path: "bucket/key".into(),
        size: Some(1024),
        content_type: None,
    });
    assert!(r.is_success());
    match r {
        ActionResult::Success { output } => assert!(output.is_reference()),
        _ => panic!("expected Success"),
    }
}

#[test]
fn success_empty_result() {
    let r: ActionResult<i32> = ActionResult::success_empty();
    assert!(r.is_success());
    match r {
        ActionResult::Success { output } => assert!(output.is_empty()),
        _ => panic!("expected Success"),
    }
}

// ── success_output / success_deferred tests ─────────────────────

#[test]
fn success_output_result() {
    use crate::output::{DeferredOutput, ExpectedOutput, Producer, ProducerKind, Resolution};
    let deferred = ActionOutput::<serde_json::Value>::Deferred(Box::new(DeferredOutput {
        handle_id: "h-1".into(),
        resolution: Resolution::Await {
            channel_id: "ch".into(),
        },
        expected: ExpectedOutput::Dynamic,
        progress: None,
        producer: Producer {
            kind: ProducerKind::AiModel,
            name: None,
            version: None,
        },
        retry: None,
        timeout: None,
    }));
    let r = ActionResult::success_output(deferred);
    assert!(r.is_success());
    match r {
        ActionResult::Success { output } => assert!(output.is_deferred()),
        _ => panic!("expected Success"),
    }
}

#[test]
fn success_deferred_result() {
    use crate::output::{DeferredOutput, ExpectedOutput, Producer, ProducerKind, Resolution};
    let r: ActionResult<serde_json::Value> = ActionResult::success_deferred(DeferredOutput {
        handle_id: "h-2".into(),
        resolution: Resolution::Callback {
            endpoint: "https://example.com".into(),
            token: "tok".into(),
        },
        expected: ExpectedOutput::Value { schema: None },
        progress: None,
        producer: Producer {
            kind: ProducerKind::ExternalApi,
            name: None,
            version: None,
        },
        retry: None,
        timeout: None,
    });
    assert!(r.is_success());
    match r {
        ActionResult::Success { output } => {
            assert!(output.is_deferred());
            assert!(output.needs_resolution());
        },
        _ => panic!("expected Success"),
    }
}

#[test]
fn map_output_with_deferred() {
    use crate::output::{DeferredOutput, ExpectedOutput, Producer, ProducerKind, Resolution};
    let r: ActionResult<i32> = ActionResult::Success {
        output: ActionOutput::Deferred(Box::new(DeferredOutput {
            handle_id: "h".into(),
            resolution: Resolution::Await {
                channel_id: "ch".into(),
            },
            expected: ExpectedOutput::Dynamic,
            progress: None,
            producer: Producer {
                kind: ProducerKind::LocalCompute,
                name: None,
                version: None,
            },
            retry: None,
            timeout: None,
        })),
    };
    let mapped = r.map_output(|n| n.to_string());
    match mapped {
        ActionResult::Success { output } => assert!(output.is_deferred()),
        _ => panic!("expected Success"),
    }
}

#[test]
fn map_output_with_collection() {
    let r: ActionResult<i32> = ActionResult::Success {
        output: ActionOutput::Collection(vec![ActionOutput::Value(1), ActionOutput::Value(2)]),
    };
    let mapped = r.map_output(|n| n * 10);
    match mapped {
        ActionResult::Success { output } => match output {
            ActionOutput::Collection(items) => {
                assert_eq!(items[0].as_value(), Some(&10));
                assert_eq!(items[1].as_value(), Some(&20));
            },
            _ => panic!("expected Collection"),
        },
        _ => panic!("expected Success"),
    }
}

// ── continue/break constructor tests ────────────────────────────

#[test]
fn continue_with_constructor() {
    let result = ActionResult::continue_with(42, Some(0.5));
    assert!(result.is_continue());
    match result {
        ActionResult::Continue {
            output,
            progress,
            delay,
        } => {
            assert_eq!(output.as_value(), Some(&42));
            assert_eq!(progress, Some(0.5));
            assert!(delay.is_none());
        },
        _ => panic!("expected Continue"),
    }
}

#[test]
fn continue_with_delay_constructor() {
    let result = ActionResult::continue_with_delay(7, Some(0.8), Duration::from_secs(5));
    assert!(result.is_continue());
    match result {
        ActionResult::Continue {
            output,
            progress,
            delay,
        } => {
            assert_eq!(output.as_value(), Some(&7));
            assert_eq!(progress, Some(0.8));
            assert_eq!(delay, Some(Duration::from_secs(5)));
        },
        _ => panic!("expected Continue"),
    }
}

#[test]
fn break_completed_constructor() {
    let result = ActionResult::break_completed(String::from("done"));
    assert!(!result.is_continue());
    match result {
        ActionResult::Break { output, reason } => {
            assert_eq!(output.as_value().map(String::as_str), Some("done"));
            assert_eq!(reason, BreakReason::Completed);
        },
        _ => panic!("expected Break"),
    }
}

#[test]
fn break_with_reason_constructor() {
    let result = ActionResult::break_with_reason(99, BreakReason::MaxIterations);
    assert!(!result.is_continue());
    match result {
        ActionResult::Break { output, reason } => {
            assert_eq!(output.as_value(), Some(&99));
            assert_eq!(reason, BreakReason::MaxIterations);
        },
        _ => panic!("expected Break"),
    }
}

// ── Drop variant ────────────────────────────────────────────────

#[test]
fn drop_item_constructor() {
    let r: ActionResult<()> = ActionResult::drop_item();
    assert!(r.is_drop());
    match r {
        ActionResult::Drop { reason } => assert!(reason.is_none()),
        _ => panic!("expected Drop"),
    }
}

#[test]
fn drop_with_reason_constructor() {
    let r: ActionResult<()> = ActionResult::drop_with_reason("rate limit exceeded");
    assert!(r.is_drop());
    match r {
        ActionResult::Drop { reason } => {
            assert_eq!(reason.as_deref(), Some("rate limit exceeded"));
        },
        _ => panic!("expected Drop"),
    }
}

#[test]
fn drop_into_primary_output_is_none() {
    let r: ActionResult<i32> = ActionResult::drop_item();
    assert!(r.into_primary_output().is_none());
}

#[test]
fn drop_map_output_preserves_reason() {
    let r: ActionResult<i32> = ActionResult::drop_with_reason("bad item");
    let mapped = r.map_output(|n| n * 10);
    match mapped {
        ActionResult::Drop { reason } => {
            assert_eq!(reason.as_deref(), Some("bad item"));
        },
        _ => panic!("expected Drop"),
    }
}

#[test]
fn drop_serde_round_trip() {
    let original: ActionResult<i32> = ActionResult::drop_with_reason("filtered");
    let json = serde_json::to_string(&original).unwrap();
    let decoded: ActionResult<i32> = serde_json::from_str(&json).unwrap();
    match decoded {
        ActionResult::Drop { reason } => {
            assert_eq!(reason.as_deref(), Some("filtered"));
        },
        _ => panic!("expected Drop"),
    }
}

// ── Terminate variant ──────────────────────────────────────────

#[test]
fn terminate_success_constructor() {
    let r: ActionResult<()> = ActionResult::terminate_success(Some("done early".into()));
    assert!(r.is_terminate());
    match r {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Success { note } => {
                assert_eq!(note.as_deref(), Some("done early"));
            },
            TerminationReason::Failure { .. } => panic!("expected Success"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn terminate_failure_constructor() {
    let r: ActionResult<()> =
        ActionResult::terminate_failure("INVALID_STATE", "cannot proceed from current state");
    assert!(r.is_terminate());
    match r {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Failure { code, message } => {
                assert_eq!(code.as_str(), "INVALID_STATE");
                assert_eq!(message, "cannot proceed from current state");
            },
            TerminationReason::Success { .. } => panic!("expected Failure"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn terminate_into_primary_output_is_none() {
    let r: ActionResult<i32> = ActionResult::terminate_success(None);
    assert!(r.into_primary_output().is_none());
}

#[test]
fn terminate_map_output_preserves_reason() {
    let r: ActionResult<i32> = ActionResult::terminate_failure("CODE", "msg");
    let mapped = r.map_output(|n| n * 10);
    match mapped {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Failure { code, message } => {
                assert_eq!(code.as_str(), "CODE");
                assert_eq!(message, "msg");
            },
            TerminationReason::Success { .. } => panic!("expected Failure"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn terminate_success_serde_round_trip() {
    let original: ActionResult<i32> = ActionResult::terminate_success(Some("ok".into()));
    let json = serde_json::to_string(&original).unwrap();
    let decoded: ActionResult<i32> = serde_json::from_str(&json).unwrap();
    match decoded {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Success { note } => assert_eq!(note.as_deref(), Some("ok")),
            TerminationReason::Failure { .. } => panic!("expected Success"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn terminate_failure_serde_round_trip() {
    let original: ActionResult<i32> = ActionResult::terminate_failure("E_BAD", "something broke");
    let json = serde_json::to_string(&original).unwrap();
    let decoded: ActionResult<i32> = serde_json::from_str(&json).unwrap();
    match decoded {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Failure { code, message } => {
                assert_eq!(code.as_str(), "E_BAD");
                assert_eq!(message, "something broke");
            },
            TerminationReason::Success { .. } => panic!("expected Failure"),
        },
        _ => panic!("expected Terminate"),
    }
}
