use super::*;

#[test]
fn action_output_value() {
    let out = ActionOutput::Value(42);
    assert!(out.is_value());
    assert!(!out.is_binary());
    assert!(!out.is_reference());
    assert!(!out.is_deferred());
    assert!(!out.is_collection());
    assert!(!out.is_empty());
    assert_eq!(out.as_value(), Some(&42));
}

#[test]
fn action_output_binary() {
    let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
        content_type: "image/png".into(),
        data: BinaryStorage::Inline(vec![0x89, 0x50, 0x4e, 0x47]),
        size: 4,
        metadata: None,
    });
    assert!(out.is_binary());
    assert!(!out.is_value());
    assert_eq!(out.as_value(), None);
}

#[test]
fn action_output_reference() {
    let out: ActionOutput<i32> = ActionOutput::Reference(DataReference {
        storage_type: "s3".into(),
        path: "bucket/key".into(),
        size: Some(1024),
        content_type: Some("application/json".into()),
    });
    assert!(out.is_reference());
}

#[test]
fn action_output_deferred() {
    let out: ActionOutput<i32> = ActionOutput::Deferred(Box::new(DeferredOutput {
        handle_id: "handle-1".into(),
        resolution: Resolution::Await {
            channel_id: "ch-1".into(),
        },
        expected: ExpectedOutput::Value { schema: None },
        progress: None,
        producer: Producer {
            kind: ProducerKind::AiModel,
            name: Some("gpt-4".into()),
            version: None,
        },
        retry: None,
        timeout: Some(Duration::from_mins(1)),
    }));
    assert!(out.is_deferred());
    assert!(!out.is_value());
    assert!(out.needs_resolution());
}

#[test]
fn action_output_collection() {
    let out: ActionOutput<i32> = ActionOutput::Collection(vec![
        ActionOutput::Value(1),
        ActionOutput::Value(2),
        ActionOutput::Empty,
    ]);
    assert!(out.is_collection());
    assert!(!out.is_value());
    assert!(!out.needs_resolution());
}

#[test]
fn action_output_collection_with_deferred() {
    let out: ActionOutput<i32> = ActionOutput::Collection(vec![
        ActionOutput::Value(1),
        ActionOutput::Deferred(Box::new(DeferredOutput {
            handle_id: "h".into(),
            resolution: Resolution::Await {
                channel_id: "ch".into(),
            },
            expected: ExpectedOutput::Dynamic,
            progress: None,
            producer: Producer {
                kind: ProducerKind::ExternalApi,
                name: None,
                version: None,
            },
            retry: None,
            timeout: None,
        })),
    ]);
    assert!(out.needs_resolution());
}

#[test]
fn action_output_empty() {
    let out: ActionOutput<i32> = ActionOutput::Empty;
    assert!(out.is_empty());
    assert_eq!(out.into_value(), None);
}

#[test]
fn action_output_map() {
    let out = ActionOutput::Value(5);
    let mapped = out.map(&mut |n| n * 2);
    assert_eq!(mapped.into_value(), Some(10));
}

#[test]
fn action_output_map_preserves_binary() {
    let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
        content_type: "text/plain".into(),
        data: BinaryStorage::Inline(vec![]),
        size: 0,
        metadata: None,
    });
    let mapped: ActionOutput<String> = out.map(&mut |n| n.to_string());
    assert!(mapped.is_binary());
}

#[test]
fn action_output_map_collection() {
    let out: ActionOutput<i32> = ActionOutput::Collection(vec![
        ActionOutput::Value(1),
        ActionOutput::Value(2),
        ActionOutput::Empty,
    ]);
    let mapped = out.map(&mut |n| n * 10);
    match mapped {
        ActionOutput::Collection(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].as_value(), Some(&10));
            assert_eq!(items[1].as_value(), Some(&20));
            assert!(items[2].is_empty());
        },
        _ => panic!("expected Collection"),
    }
}

#[test]
fn action_output_try_map_ok() {
    let out = ActionOutput::Value(5);
    let mapped = out.try_map(&mut |n| Ok::<_, String>(n * 2));
    assert_eq!(mapped.unwrap().into_value(), Some(10));
}

#[test]
fn action_output_try_map_err() {
    let out = ActionOutput::Value(5);
    let mapped = out.try_map(&mut |_| Err::<i32, _>("fail"));
    assert_eq!(mapped.unwrap_err(), "fail");
}

#[test]
fn action_output_try_map_non_value() {
    let out: ActionOutput<i32> = ActionOutput::Empty;
    let mapped = out.try_map(&mut |_| Err::<i32, _>("should not be called"));
    assert!(mapped.unwrap().is_empty());
}

#[test]
fn action_output_try_map_collection() {
    let out: ActionOutput<i32> =
        ActionOutput::Collection(vec![ActionOutput::Value(1), ActionOutput::Value(2)]);
    let mapped = out.try_map(&mut |n| Ok::<_, String>(n * 3));
    match mapped.unwrap() {
        ActionOutput::Collection(items) => {
            assert_eq!(items[0].as_value(), Some(&3));
            assert_eq!(items[1].as_value(), Some(&6));
        },
        _ => panic!("expected Collection"),
    }
}

#[test]
fn action_output_try_map_collection_err() {
    let out: ActionOutput<i32> =
        ActionOutput::Collection(vec![ActionOutput::Value(1), ActionOutput::Value(2)]);
    let mapped = out.try_map(&mut |n| {
        if n == 2 { Err("bad") } else { Ok(n) }
    });
    assert_eq!(mapped.unwrap_err(), "bad");
}

#[test]
fn action_output_into_value() {
    assert_eq!(ActionOutput::Value(42).into_value(), Some(42));
    assert_eq!(ActionOutput::<i32>::Empty.into_value(), None);
}

#[test]
fn needs_resolution_value() {
    assert!(!ActionOutput::Value(42).needs_resolution());
}

#[test]
fn needs_resolution_binary() {
    let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
        content_type: "x".into(),
        data: BinaryStorage::Inline(vec![]),
        size: 0,
        metadata: None,
    });
    assert!(!out.needs_resolution());
}

#[test]
fn needs_resolution_empty() {
    assert!(!ActionOutput::<i32>::Empty.needs_resolution());
}

// ── Ergonomic constructor tests ─────────────────────────────────

#[test]
fn deferred_ai_constructor() {
    let out = ActionOutput::<serde_json::Value>::deferred_ai(
        "gen-img-123",
        "dall-e-3",
        "openai",
        Resolution::Poll {
            target: PollTarget::Http {
                url: "https://api.example.com/status".into(),
                method: "GET".into(),
            },
            interval: Duration::from_secs(2),
            backoff: 1.5,
            max_interval: Some(Duration::from_secs(15)),
        },
        ExpectedOutput::Binary {
            content_type: "image/png".into(),
        },
    );
    assert!(out.is_deferred());
    assert!(out.needs_resolution());
    match &out {
        ActionOutput::Deferred(d) => {
            assert_eq!(d.handle_id, "gen-img-123");
            assert_eq!(d.producer.kind, ProducerKind::AiModel);
            assert_eq!(d.producer.name.as_deref(), Some("dall-e-3"));
            assert!(d.retry.is_some());
            assert!(d.timeout.is_some());
        },
        _ => panic!("expected Deferred"),
    }
}

#[test]
fn deferred_document_constructor() {
    let out = ActionOutput::<serde_json::Value>::deferred_document(
        "doc-456",
        "application/pdf",
        Resolution::Await {
            channel_id: "ch-doc".into(),
        },
    );
    match &out {
        ActionOutput::Deferred(d) => {
            assert_eq!(d.handle_id, "doc-456");
            assert_eq!(d.producer.kind, ProducerKind::LocalCompute);
            assert!(d.progress.is_some());
            assert_eq!(d.timeout, Some(Duration::from_mins(5)));
        },
        _ => panic!("expected Deferred"),
    }
}

#[test]
fn deferred_callback_constructor() {
    let out = ActionOutput::<serde_json::Value>::deferred_callback(
        "cb-789",
        "https://hooks.example.com/callback",
        "tok-abc",
        ExpectedOutput::Value { schema: None },
        Some(Duration::from_hours(1)),
    );
    match &out {
        ActionOutput::Deferred(d) => {
            assert_eq!(d.handle_id, "cb-789");
            assert!(matches!(d.resolution, Resolution::Callback { .. }));
            assert_eq!(d.producer.kind, ProducerKind::ExternalApi);
            assert_eq!(d.timeout, Some(Duration::from_hours(1)));
        },
        _ => panic!("expected Deferred"),
    }
}

// ── OutputEnvelope tests ────────────────────────────────────────

#[test]
fn output_envelope_new() {
    let envelope = OutputEnvelope::new(ActionOutput::Value(42));
    assert_eq!(envelope.output.as_value(), Some(&42));
    assert!(envelope.meta.origin.is_none());
    assert!(envelope.meta.timing.is_none());
}

#[test]
fn output_envelope_with_meta() {
    let meta = OutputMeta {
        origin: Some(OutputOrigin::Computed),
        trace_id: Some("trace-1".into()),
        ..Default::default()
    };
    let envelope = OutputEnvelope::with_meta(ActionOutput::Value("data"), meta);
    assert!(matches!(envelope.meta.origin, Some(OutputOrigin::Computed)));
    assert_eq!(envelope.meta.trace_id.as_deref(), Some("trace-1"));
}

// ── Serde round-trip tests ──────────────────────────────────────

#[test]
fn serde_deferred_output_roundtrip() {
    let deferred = DeferredOutput {
        handle_id: "h-1".into(),
        resolution: Resolution::Poll {
            target: PollTarget::Http {
                url: "https://api.test/status".into(),
                method: "GET".into(),
            },
            interval: Duration::from_secs(5),
            backoff: 2.0,
            max_interval: Some(Duration::from_mins(1)),
        },
        expected: ExpectedOutput::Binary {
            content_type: "image/png".into(),
        },
        progress: Some(Progress {
            fraction: 0.5,
            message: Some("Half done".into()),
            eta_ms: Some(30_000),
        }),
        producer: Producer {
            kind: ProducerKind::AiModel,
            name: Some("dall-e-3".into()),
            version: Some("v1".into()),
        },
        retry: None,
        timeout: Some(Duration::from_mins(2)),
    };

    let json = serde_json::to_string(&deferred).unwrap();
    let back: DeferredOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(deferred, back);
}

#[test]
fn serde_resolution_variants() {
    let variants: Vec<Resolution> = vec![
        Resolution::Poll {
            target: PollTarget::Service {
                name: "svc".into(),
                operation: "check".into(),
            },
            interval: Duration::from_secs(1),
            backoff: 1.0,
            max_interval: None,
        },
        Resolution::Await {
            channel_id: "ch".into(),
        },
        Resolution::Callback {
            endpoint: "https://example.com".into(),
            token: "tok".into(),
        },
        Resolution::SubWorkflow {
            workflow_id: "wf-1".into(),
            input: Some(serde_json::json!({"key": "value"})),
        },
        Resolution::AwaitOrPoll {
            channel_id: "ch-2".into(),
            fallback_after: Duration::from_secs(10),
            poll_target: PollTarget::Action {
                action_key: "check_status".into(),
            },
            poll_interval: Duration::from_secs(5),
        },
    ];

    for variant in &variants {
        let json = serde_json::to_string(variant).unwrap();
        let back: Resolution = serde_json::from_str(&json).unwrap();
        assert_eq!(variant, &back);
    }
}

#[test]
fn serde_action_output_deferred_roundtrip() {
    let out: ActionOutput<serde_json::Value> = ActionOutput::Deferred(Box::new(DeferredOutput {
        handle_id: "test".into(),
        resolution: Resolution::Await {
            channel_id: "ch".into(),
        },
        expected: ExpectedOutput::Dynamic,
        progress: None,
        producer: Producer {
            kind: ProducerKind::Human,
            name: None,
            version: None,
        },
        retry: None,
        timeout: None,
    }));

    let json = serde_json::to_string(&out).unwrap();
    let back: ActionOutput<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(out, back);
}

// ── DeferredRetryConfig::validate ──────────────────────────────────

fn sane_retry_config() -> DeferredRetryConfig {
    DeferredRetryConfig {
        max_attempts: 3,
        initial_interval: Duration::from_secs(1),
        backoff_coefficient: 2.0,
        max_interval: Some(Duration::from_secs(30)),
        non_retryable_errors: vec![],
    }
}

fn assert_validation_field(result: Result<(), crate::ActionError>, expected_field: &str) {
    match result {
        Err(crate::ActionError::Validation { field, .. }) => assert_eq!(field, expected_field),
        other => panic!("expected Validation({expected_field}), got {other:?}"),
    }
}

#[test]
fn retry_config_accepts_sane_defaults() {
    assert!(sane_retry_config().validate().is_ok());
}

#[test]
fn retry_config_rejects_zero_max_attempts() {
    let cfg = DeferredRetryConfig {
        max_attempts: 0,
        ..sane_retry_config()
    };
    assert_validation_field(cfg.validate(), "deferred_retry.max_attempts");
}

#[test]
fn retry_config_rejects_zero_initial_interval() {
    let cfg = DeferredRetryConfig {
        initial_interval: Duration::ZERO,
        ..sane_retry_config()
    };
    assert_validation_field(cfg.validate(), "deferred_retry.initial_interval");
}

#[test]
fn retry_config_rejects_nan_backoff() {
    let cfg = DeferredRetryConfig {
        backoff_coefficient: f64::NAN,
        ..sane_retry_config()
    };
    assert_validation_field(cfg.validate(), "deferred_retry.backoff_coefficient");
}

#[test]
fn retry_config_rejects_infinite_backoff() {
    let cfg = DeferredRetryConfig {
        backoff_coefficient: f64::INFINITY,
        ..sane_retry_config()
    };
    assert_validation_field(cfg.validate(), "deferred_retry.backoff_coefficient");
}

#[test]
fn retry_config_rejects_non_positive_backoff() {
    for bad in [0.0_f64, -1.0, -f64::MIN_POSITIVE] {
        let cfg = DeferredRetryConfig {
            backoff_coefficient: bad,
            ..sane_retry_config()
        };
        assert_validation_field(cfg.validate(), "deferred_retry.backoff_coefficient");
    }
}

// ── TokenUsage Eq/Hash ─────────────────────────────────────────────

#[test]
fn token_usage_eq_and_hash_agree() {
    use std::collections::HashSet;

    let a = TokenUsage {
        input: 100,
        output: 50,
        cached: Some(10),
    };
    let b = a.clone();
    let c = TokenUsage {
        input: 100,
        output: 50,
        cached: None,
    };

    assert_eq!(a, b);
    assert_ne!(a, c);

    let mut set: HashSet<TokenUsage> = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
    assert!(!set.contains(&c));
}

// ── Timing Eq/Hash ─────────────────────────────────────────────────

#[test]
fn timing_eq_and_hash_agree() {
    use std::collections::HashSet;

    let started = chrono::Utc::now();
    let a = Timing {
        started_at: started,
        completed_at: None,
        wall_time_ms: Some(42),
        queue_time_ms: None,
    };
    let b = a.clone();
    let c = Timing {
        started_at: started,
        completed_at: None,
        wall_time_ms: Some(43),
        queue_time_ms: None,
    };

    assert_eq!(a, b);
    assert_ne!(a, c);

    let mut set: HashSet<Timing> = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
    assert!(!set.contains(&c));
}
