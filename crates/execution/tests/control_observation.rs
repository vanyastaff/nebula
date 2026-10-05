use chrono::Utc;
use nebula_core::{WorkerFlavorRevisionId, node_key};
use nebula_execution::{
    ExecutionControlObservationV1, ExecutionControlOutcome, ExecutionControlQueueKind,
    ExecutionControlReason, ExecutionControlSource, JournalEntry,
};
use serde::Deserialize;

/// Capture the actual derived wire vocabulary, so a seventh enum variant
/// cannot hide behind fixtures that still construct only the original six.
struct OutcomeVariants<'a>(&'a mut Vec<&'static str>);

impl<'de> serde::Deserializer<'de> for OutcomeVariants<'_> {
    type Error = serde::de::value::Error;

    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        _name: &'static str,
        variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.extend_from_slice(variants);
        Err(serde::de::Error::custom("wire vocabulary captured"))
    }

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom("only enum inventory is supported"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

#[test]
fn outcome_wire_inventory_is_exactly_the_required_six_values() {
    let mut actual = Vec::new();
    let _ = ExecutionControlOutcome::deserialize(OutcomeVariants(&mut actual));
    assert_eq!(
        actual,
        [
            "accepted",
            "fenced",
            "deferred",
            "throttled",
            "recovered",
            "flavor-mismatch"
        ]
    );
}

fn observation(reason: ExecutionControlReason) -> ExecutionControlObservationV1 {
    ExecutionControlObservationV1::new(
        ExecutionControlSource::ControlQueue {
            row_id: [7; 16],
            queue_claim_generation: 3,
        },
        11,
        reason,
    )
}

#[test]
fn six_outcomes_are_derived_from_closed_reasons_and_roundtrip() {
    let cases = [
        (ExecutionControlReason::ControlAccepted, "accepted"),
        (
            ExecutionControlReason::LeaseFenced {
                attempted_execution_lease_generation: 10,
                current_execution_lease_generation: 11,
            },
            "fenced",
        ),
        (
            ExecutionControlReason::ExecutionVersionConflict {
                expected_version: 4,
                actual_version: 5,
            },
            "deferred",
        ),
        (ExecutionControlReason::AdmissionThrottled, "throttled"),
        (ExecutionControlReason::AcceptedTurnRecovered, "recovered"),
        (
            ExecutionControlReason::ExactFlavorMismatch {
                expected: WorkerFlavorRevisionId::from_bytes([1; 32]),
                actual: WorkerFlavorRevisionId::from_bytes([2; 32]),
            },
            "flavor-mismatch",
        ),
    ];
    for (reason, expected) in cases {
        let record = observation(reason);
        assert_eq!(record.outcome().as_str(), expected);
        let wire = serde_json::to_value(&record).unwrap();
        assert_eq!(wire["version"], 1);
        assert_eq!(wire["outcome"], expected);
        assert_eq!(
            serde_json::from_value::<ExecutionControlObservationV1>(wire).unwrap(),
            record
        );
    }
}

#[test]
fn durable_decoding_refuses_unknown_versions_labels_fields_and_incoherent_outcome() {
    let original =
        serde_json::to_value(observation(ExecutionControlReason::ControlAccepted)).unwrap();
    for (key, invalid) in [
        ("version", serde_json::json!(0)),
        ("version", serde_json::json!(2)),
        ("outcome", serde_json::json!("completed")),
        ("outcome", serde_json::json!("fenced")),
        ("unexpected", serde_json::json!(true)),
        ("attempt", serde_json::json!({"attempt": 4})),
        ("error", serde_json::json!("provider failure with secret")),
        (
            "reason",
            serde_json::json!({"code":"control_accepted", "provider":"secret"}),
        ),
        (
            "source",
            serde_json::json!({"kind":"control_queue", "row_id":vec![7u8;16], "generation":3}),
        ),
    ] {
        let mut invalid_wire = original.clone();
        invalid_wire[key] = invalid;
        assert!(
            serde_json::from_value::<ExecutionControlObservationV1>(invalid_wire).is_err(),
            "{key}"
        );
    }
    assert!(serde_json::from_str::<ExecutionControlObservationV1>(r#""accepted""#).is_err());
}

#[test]
fn all_fenced_causes_retain_their_distinct_generation_axis() {
    let reasons = [
        ExecutionControlReason::ClaimSuperseded {
            attempted_queue_claim_generation: 2,
            current_queue_claim_generation: 3,
        },
        ExecutionControlReason::LeaseFenced {
            attempted_execution_lease_generation: 10,
            current_execution_lease_generation: 11,
        },
        ExecutionControlReason::LeaseExpired {
            attempted_execution_lease_generation: 11,
            current_execution_lease_generation: 11,
        },
        ExecutionControlReason::LeaseAbsent {
            attempted_execution_lease_generation: 11,
            current_execution_lease_generation: 11,
        },
    ];
    for reason in reasons {
        let record = observation(reason.clone());
        assert_eq!(record.outcome(), ExecutionControlOutcome::Fenced);
        let wire = serde_json::to_value(&record).unwrap();
        assert_eq!(wire["reason"]["code"], reason.as_str());
        assert_eq!(
            serde_json::from_value::<ExecutionControlObservationV1>(wire).unwrap(),
            record
        );
    }
}

#[test]
fn historical_journal_stays_readable_and_new_nested_version_fails_closed() {
    let timestamp = Utc::now();
    let old = JournalEntry::ExecutionStarted { timestamp };
    assert_eq!(
        JournalEntry::from_json(&old.to_json().unwrap())
            .unwrap()
            .timestamp(),
        timestamp
    );
    let new = JournalEntry::ControlObserved {
        timestamp,
        observation: observation(ExecutionControlReason::ControlAccepted)
            .with_attempt(node_key!("node"), 2),
    };
    let mut wire = serde_json::to_value(&new).unwrap();
    assert_eq!(wire["event"], "control_observed");
    let read = JournalEntry::from_json(&wire.to_string()).unwrap();
    assert!(read.is_execution_event());
    assert_eq!(read.timestamp(), timestamp);
    wire["observation"]["version"] = serde_json::json!(255);
    assert!(JournalEntry::from_json(&wire.to_string()).is_err());
}

#[test]
fn recovery_preserves_original_source_and_keeps_lease_and_claim_generations_distinct() {
    let source = ExecutionControlSource::AcceptedTurn {
        source_kind: ExecutionControlQueueKind::JobDispatch,
        source_row_id: [9; 16],
        accepted_execution_lease_generation: 41,
    };
    let record = ExecutionControlObservationV1::new(
        source.clone(),
        42,
        ExecutionControlReason::AcceptedTurnRecovered,
    );
    assert_eq!(record.source(), &source);
    assert_eq!(record.execution_lease_generation(), 42);
    assert_eq!(record.outcome(), ExecutionControlOutcome::Recovered);
    assert_eq!(
        serde_json::from_str::<ExecutionControlObservationV1>(
            &serde_json::to_string(&record).unwrap()
        )
        .unwrap(),
        record
    );
}

#[test]
fn checkpoint_version_conflict_is_deferred_without_relabeling_versions_as_fences() {
    let record = observation(ExecutionControlReason::ExecutionVersionConflict {
        expected_version: 5,
        actual_version: 6,
    });
    assert_eq!(record.outcome(), ExecutionControlOutcome::Deferred);
    assert_eq!(record.execution_lease_generation(), 11);
    let wire = serde_json::to_value(&record).unwrap();
    assert_eq!(wire["reason"]["code"], "execution_version_conflict");
    assert_eq!(wire["reason"]["expected_version"], 5);
    assert_eq!(wire["reason"]["actual_version"], 6);
    assert_eq!(
        serde_json::from_value::<ExecutionControlObservationV1>(wire).unwrap(),
        record
    );
}

/// Journal readers hand payloads around as `serde_json::Value`; an attributed
/// observation must decode from that owned form, not only from a JSON string.
#[test]
fn attributed_observation_decodes_from_an_owned_json_value() {
    let entry = JournalEntry::ControlObserved {
        timestamp: Utc::now(),
        observation: observation(ExecutionControlReason::AdmissionThrottled)
            .with_attempt(node_key!("admission_a"), 0),
    };
    let payload = serde_json::to_value(&entry).unwrap();
    let decoded = serde_json::from_value::<JournalEntry>(payload).unwrap();
    let JournalEntry::ControlObserved { observation, .. } = decoded else {
        panic!("control observation must keep its journal variant");
    };
    let attempt = observation
        .attempt()
        .expect("attempt survives the owned decode");
    assert_eq!(attempt.node_key, node_key!("admission_a"));
    assert_eq!(attempt.attempt, 0);

    let mut invalid = serde_json::to_value(
        observation
            .clone()
            .with_attempt(node_key!("admission_a"), 0),
    )
    .unwrap();
    invalid["attempt"]["node_key"] = serde_json::json!("not a key!");
    assert!(serde_json::from_value::<ExecutionControlObservationV1>(invalid).is_err());
}
