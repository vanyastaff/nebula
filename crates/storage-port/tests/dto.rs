use nebula_storage_port::dto::{ControlMsg, ExecutionRecord, JournalEntry, NodeResultRecord};
use nebula_storage_port::{FencingToken, Scope};

#[test]
fn node_result_record_is_action_result_free_and_roundtrips() {
    let r = NodeResultRecord {
        kind_tag: "Value".into(),
        json: serde_json::json!({"k":1}),
        schema_version: 1,
    };
    let s = serde_json::to_string(&r).expect("serialize");
    let back: NodeResultRecord = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.schema_version, 1);
    assert_eq!(back.kind_tag, "Value");
}

#[test]
fn execution_record_roundtrips() {
    let rec = ExecutionRecord {
        id: "exe_1".into(),
        workflow_id: "wf_1".into(),
        scope: Scope::new("ws_1", "org_1"),
        version: 3,
        status: "Running".into(),
        state: serde_json::json!({"s":"running"}),
        lease_holder: Some("nbl_1".into()),
        fencing: Some(7),
        created_at: "2026-05-15T00:00:00Z".into(),
        updated_at: "2026-05-15T00:00:01Z".into(),
    };
    let s = serde_json::to_string(&rec).expect("serialize");
    let back: ExecutionRecord = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back, rec);
}

#[test]
fn control_msg_roundtrips_with_typed_16_byte_id() {
    use nebula_storage_port::dto::ControlCommand;
    let msg = ControlMsg {
        id: [7u8; 16],
        execution_id: "exe_1".into(),
        command: ControlCommand::Cancel,
        scope: Scope::new("ws_1", "org_1"),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let s = serde_json::to_string(&msg).expect("serialize");
    let back: ControlMsg = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.id, [7u8; 16]);
    assert_eq!(back.command, ControlCommand::Cancel);
    assert_eq!(back.resume_target, None);
}

#[test]
fn journal_entry_roundtrips() {
    let je = JournalEntry {
        seq: Some(1),
        payload: serde_json::json!({"event":"started"}),
    };
    let s = serde_json::to_string(&je).expect("serialize");
    let back: JournalEntry = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.seq, Some(1));
}

mod iteration_checkpoint {
    use nebula_storage_port::{
        IterationCheckpoint, IterationCheckpointError, IterationCheckpointKey,
        MAX_CHECKPOINT_ITERATION, MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES,
        MAX_ITERATION_CHECKPOINT_STATE_BYTES, Scope,
    };

    fn record(
        iteration: u32,
        state: Vec<u8>,
    ) -> Result<IterationCheckpoint, IterationCheckpointError> {
        IterationCheckpoint::new(iteration, state, [7; 32], Some(250), 3, 2)
    }

    #[test]
    fn iteration_is_bounded_to_the_runtime_cap() {
        assert_eq!(
            record(0, b"{}".to_vec()),
            Err(IterationCheckpointError::InvalidRecord)
        );
        assert!(record(1, b"{}".to_vec()).is_ok());
        assert!(record(MAX_CHECKPOINT_ITERATION, b"{}".to_vec()).is_ok());
        assert_eq!(
            record(MAX_CHECKPOINT_ITERATION + 1, b"{}".to_vec()),
            Err(IterationCheckpointError::InvalidRecord)
        );
    }

    #[test]
    fn state_is_bounded_and_kept_byte_exact() {
        let largest = vec![b'x'; MAX_ITERATION_CHECKPOINT_STATE_BYTES];
        let kept = record(4, largest.clone()).expect("the bound itself is admitted");
        assert_eq!(kept.state(), largest.as_slice());
        assert_eq!(
            record(4, vec![b'x'; MAX_ITERATION_CHECKPOINT_STATE_BYTES + 1]),
            Err(IterationCheckpointError::TooLarge)
        );
    }

    #[test]
    fn delay_and_generation_stay_in_the_portable_range() {
        let too_far = u64::try_from(i64::MAX).unwrap() + 1;
        assert_eq!(
            IterationCheckpoint::new(1, Vec::new(), [0; 32], Some(too_far), 0, 0),
            Err(IterationCheckpointError::InvalidRecord)
        );
        assert_eq!(
            IterationCheckpoint::new(1, Vec::new(), [0; 32], None, 0, too_far),
            Err(IterationCheckpointError::InvalidRecord)
        );
    }

    #[test]
    fn provenance_is_adapter_written() {
        let fresh = record(2, b"{\"n\":1}".to_vec()).unwrap();
        assert_eq!((fresh.fencing_generation(), fresh.written_at_ms()), (0, 0));
        let stored = fresh.with_write_provenance(9, 1_700_000_000_000);
        assert_eq!(stored.fencing_generation(), 9);
        assert_eq!(stored.written_at_ms(), 1_700_000_000_000);
        assert_eq!(stored.iteration(), 2);
        assert_eq!(stored.resume_delay_ms(), Some(250));
        assert_eq!(stored.attested_positions(), 3);
        assert_eq!(stored.attempt_generation(), 2);
    }

    #[test]
    fn debug_prints_length_and_digest_never_the_state() {
        let secret = b"{\"token\":\"hunter2-very-secret\"}".to_vec();
        let printed = format!("{:?}", record(3, secret.clone()).unwrap());
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(
            printed.contains(&format!("state_bytes: {}", secret.len())),
            "{printed}"
        );
        assert!(printed.contains(&"07".repeat(32)), "{printed}");
    }

    #[test]
    fn key_parts_are_bounded_and_the_version_canonical() {
        let scope = Scope::new("ws", "org");
        assert!(IterationCheckpointKey::new(&scope, "exe", "node", "a.b", "1.2.3").is_ok());
        assert!(IterationCheckpointKey::new(&scope, "exe", "node", "a.b", "1.2.3-rc.1+7").is_ok());
        for version in ["", "1.2", "01.2.3", "v1.2.3", "1.2.3-"] {
            assert_eq!(
                IterationCheckpointKey::new(&scope, "exe", "node", "a.b", version),
                Err(IterationCheckpointError::InvalidRecord),
                "{version}"
            );
        }
        assert_eq!(
            IterationCheckpointKey::new(&scope, "", "node", "a.b", "1.0.0"),
            Err(IterationCheckpointError::InvalidRecord)
        );
        let long = "n".repeat(MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES + 1);
        assert_eq!(
            IterationCheckpointKey::new(&scope, "exe", &long, "a.b", "1.0.0"),
            Err(IterationCheckpointError::InvalidRecord)
        );
        // The scope is taken as the execution was admitted under it: neither
        // `Scope` nor `port_executions` bounds it, so neither does the key.
        let empty_scope = Scope::new("", "org");
        assert!(IterationCheckpointKey::new(&empty_scope, "exe", "node", "a.b", "1.0.0").is_ok());
    }

    #[test]
    fn rescoping_keeps_every_other_part() {
        let caller = Scope::new("ws-a", "org-a");
        let bound = Scope::new("ws-b", "org-b");
        let key = IterationCheckpointKey::new(&caller, "exe", "node", "a.b", "1.0.0").unwrap();
        let rescoped = key.rescoped(&bound);
        assert_eq!(rescoped.scope(), &bound);
        assert_eq!(
            (
                rescoped.execution_id(),
                rescoped.node_key(),
                rescoped.action_key(),
                rescoped.action_version()
            ),
            ("exe", "node", "a.b", "1.0.0")
        );
    }

    #[test]
    fn deferred_failures_say_nothing_about_the_stored_row() {
        for deferred in [
            IterationCheckpointError::Unavailable,
            IterationCheckpointError::AcknowledgementUnknown,
            IterationCheckpointError::ExecutionLeaseRejected,
        ] {
            assert!(deferred.is_deferred(), "{deferred:?}");
        }
        for definitive in [
            IterationCheckpointError::Conflict,
            IterationCheckpointError::Regressed { stored: 4 },
            IterationCheckpointError::InvalidRecord,
            IterationCheckpointError::TooLarge,
        ] {
            assert!(!definitive.is_deferred(), "{definitive:?}");
        }
    }
}

// Compile-time guard: a fresh FencingToken generation is comparable, proving
// the id seam stays usable from DTO-consuming code.
#[test]
fn fencing_token_seam_visible() {
    assert!(FencingToken::from_generation(0) < FencingToken::from_generation(1));
}
