//! First-party RED conformance profile (evidence-only; non-deployment; non-SDK).
//!
//! This module exists only under the non-default `runtime-repair-red` feature
//! of the unpublished `nebula-server` app package. It is not an SDK or
//! embedding surface and cannot be promoted into a deployment root.

mod evidence;
mod profile;

pub use evidence::{
    ArtifactContentClassification, ArtifactDigest, ArtifactEntry, ArtifactRecorder,
    ArtifactRecorderSnapshot, ArtifactReference, ArtifactSlot, EffectCall, EffectProbe,
    EffectProbeSnapshot, EvidenceControls, EvidenceIntegrityError, EvidencePoint,
    ExecutionObservationOutcome, FileSqliteReopenController, LifecycleObservationRegistry,
    LifecycleObservationSnapshot, LivePostgresIntegrityProbe, ManualClock, OneShotFailpoint,
    PhaseGate, PostgresIntegritySnapshot,
};
pub use profile::{
    PROFILE_LABEL, RuntimeRepairHarness, RuntimeRepairProfileConfig, RuntimeRepairProfileError,
    RuntimeRepairProfileHandle,
};

#[cfg(test)]
mod integrity_tests {
    use std::time::Duration;

    use nebula_core::{NodeKey, accessor::Clock as _, id::ExecutionId};
    use nebula_engine::ExecutionEvent;
    #[cfg(not(feature = "postgres"))]
    use secrecy::SecretString;

    use super::{
        ArtifactContentClassification, ArtifactDigest, ArtifactEntry, ArtifactRecorder,
        ArtifactReference, EffectProbe, EvidenceControls, EvidenceIntegrityError, EvidencePoint,
        ExecutionObservationOutcome, LifecycleObservationRegistry, ManualClock, PROFILE_LABEL,
        RuntimeRepairProfileConfig,
    };

    /// Red→green: the profile's engine clock must share an era with the
    /// durable adapters composed beside it.
    ///
    /// Those adapters stamp `not_before`/`cutoff` with `Utc::now()`, so a clock
    /// left at `DateTime::<Utc>::UNIX_EPOCH` left every engine-computed
    /// deadline decades in the past and made lease reclaim, visibility
    /// timeouts and retry gating fire immediately or never — turning harness
    /// artifacts into what reads as product RED evidence.
    #[test]
    fn evidence_clock_shares_an_era_with_durable_timestamps() {
        let controls = EvidenceControls::in_memory();
        let observed = controls.clock().now();
        let durable_stamp = chrono::Utc::now();

        let skew_seconds = (durable_stamp - observed).num_seconds().abs();
        assert!(
            skew_seconds < 60,
            "the injected clock reads {observed} while durable rows stamp {durable_stamp} \
             ({skew_seconds}s apart); engine deadlines and persisted deadlines must be comparable"
        );
    }

    #[test]
    fn profile_label_matches_the_accepted_spec_exactly() {
        assert_eq!(
            PROFILE_LABEL,
            "first-party RED conformance profile (evidence-only; non-deployment; non-SDK)"
        );
    }

    #[tokio::test]
    async fn manual_clock_observes_advance_before_wait_without_sleeping() {
        let clock = ManualClock::new(100).expect("bounded clock instant");
        let wall_before = clock.now();
        let monotonic_before = clock.monotonic();
        assert_eq!(clock.advance_by_millis(25), Ok(125));
        assert_eq!(clock.wait_until_millis(125).await, Ok(()));
        assert_eq!(clock.now_millis(), 125);
        assert_eq!(
            clock.now() - wall_before,
            chrono::Duration::milliseconds(25)
        );
        assert_eq!(
            clock.monotonic().checked_duration_since(monotonic_before),
            Some(Duration::from_millis(25))
        );
        assert_eq!(
            ManualClock::new(u64::MAX).expect_err("wall mapping is bounded"),
            EvidenceIntegrityError::ClockOverflow
        );
    }

    #[tokio::test]
    async fn lifecycle_registry_retains_sanitized_facts_before_waiters_subscribe() {
        let harness = RuntimeRepairProfileConfig::in_memory().into_harness();
        let observations = harness.evidence_controls().observations();
        let execution_id = ExecutionId::new();
        let node_key = NodeKey::new("wait_node").expect("valid node identity");

        observations
            .record_event(ExecutionEvent::NodeStarted {
                execution_id,
                node_key: node_key.clone(),
                action_key: "not-retained".to_owned(),
            })
            .expect("started fact records");
        observations
            .record_event(ExecutionEvent::NodeParked {
                execution_id,
                node_key: node_key.clone(),
                wake_at: None,
            })
            .expect("parked fact records");
        observations
            .record_event(ExecutionEvent::NodeWaitCompleted {
                execution_id,
                node_key: node_key.clone(),
            })
            .expect("wait-completed fact records");
        observations
            .record_event(ExecutionEvent::ExecutionFinished {
                execution_id,
                success: true,
                elapsed: Duration::from_millis(5),
                termination_reason: None,
            })
            .expect("terminal fact records");
        assert_eq!(
            observations.record_event(ExecutionEvent::ExecutionFinished {
                execution_id,
                success: false,
                elapsed: Duration::from_millis(6),
                termination_reason: None,
            }),
            Err(EvidenceIntegrityError::ConflictingTerminalObservation),
            "a contradictory terminal fact must fail closed"
        );

        let bound = Duration::from_secs(1);
        assert_eq!(
            observations
                .await_node_started(execution_id, node_key.clone(), bound)
                .await,
            Ok(())
        );
        assert_eq!(
            observations
                .await_node_parked(execution_id, node_key.clone(), bound)
                .await,
            Ok(())
        );
        assert_eq!(
            observations
                .await_node_wait_completed(execution_id, node_key, bound)
                .await,
            Ok(())
        );
        assert_eq!(
            observations
                .await_execution_finished(execution_id, bound)
                .await,
            Ok(ExecutionObservationOutcome::Succeeded)
        );
        let snapshot = observations.snapshot().expect("count snapshot");
        assert_eq!(snapshot.node_started_count, 1);
        assert_eq!(snapshot.node_parked_count, 1);
        assert_eq!(snapshot.node_wait_completed_count, 1);
        assert_eq!(snapshot.execution_finished_count, 1);
    }

    #[test]
    fn lifecycle_registry_rejects_new_distinct_facts_at_its_capacity() {
        let observations = LifecycleObservationRegistry::with_capacity_for_integrity(1);
        let execution_id = ExecutionId::new();
        let node_key = NodeKey::new("bounded_node").expect("valid node identity");
        let started = ExecutionEvent::NodeStarted {
            execution_id,
            node_key: node_key.clone(),
            action_key: "not-retained".to_owned(),
        };
        observations
            .record_event(started.clone())
            .expect("first distinct fact fits");
        observations
            .record_event(started)
            .expect("duplicate fact does not consume capacity");
        assert_eq!(
            observations.record_event(ExecutionEvent::NodeParked {
                execution_id,
                node_key,
                wake_at: None,
            }),
            Err(EvidenceIntegrityError::ObservationCapacityExceeded)
        );
        let snapshot = observations.snapshot().expect("bounded snapshot");
        assert_eq!(snapshot.node_started_count, 1);
        assert_eq!(snapshot.node_parked_count, 0);
    }

    #[tokio::test]
    async fn named_failpoint_and_phase_gate_do_not_lose_early_signals() {
        let controls = RuntimeRepairProfileConfig::in_memory().into_harness();
        let failpoint = controls.evidence_controls().one_shot_failpoint(
            EvidencePoint::parse("after-provider-acceptance").expect("valid evidence name"),
        );
        failpoint.arm().expect("first arm succeeds");
        assert!(failpoint.trip());
        assert_eq!(failpoint.wait_fired().await, Ok(()));
        assert!(!failpoint.trip(), "one-shot failpoint cannot fire twice");

        let phase_gate = controls.evidence_controls().phase_gate(
            EvidencePoint::parse("before-outcome-commit").expect("valid evidence name"),
        );
        assert_eq!(phase_gate.advance_to(3), Ok(()));
        assert_eq!(phase_gate.wait_for(2).await, Ok(()));
        assert_eq!(
            phase_gate.advance_to(1),
            Err(EvidenceIntegrityError::PhaseRegression {
                current: 3,
                requested: 1,
            })
        );
    }

    #[test]
    fn effect_probe_counts_calls_and_distinct_committed_effects_independently() {
        let probe = EffectProbe::default();
        let operation = EvidencePoint::parse("operation-alpha").expect("valid evidence name");
        let first_call = probe
            .record_call(operation.clone())
            .expect("first call records");
        let repeated_call = probe.record_call(operation).expect("repeat call records");
        assert!(probe.commit_effect(&first_call).expect("commit records"));
        assert!(
            !probe
                .commit_effect(&repeated_call)
                .expect("repeat commit is classified")
        );
        assert_eq!(
            probe.snapshot().expect("snapshot records"),
            super::EffectProbeSnapshot {
                call_count: 2,
                committed_effect_count: 1,
            }
        );
    }

    #[test]
    fn artifact_recorder_is_bounded_and_rejects_secret_or_raw_payload_entries() {
        let recorder = ArtifactRecorder::new(5).expect("bounded recorder");
        for reference in [
            ArtifactReference::RuntimeEnvironment,
            ArtifactReference::ScenarioInputRevision,
            ArtifactReference::ScenarioDenominator,
            ArtifactReference::SanitizedLifecycleObservations,
            ArtifactReference::DatabaseQueryPlan,
        ] {
            let entry = ArtifactEntry::new(
                ArtifactContentClassification::SanitizedReference,
                reference,
                1,
                Some(ArtifactDigest::new([0xabu8; 32])),
            )
            .expect("sanitized entry constructs");
            let debug = format!("{entry:?}");
            assert!(!debug.contains("abab"), "digest bytes stay out of Debug");
            recorder.record(entry).expect("sanitized entry records");
        }
        assert_eq!(
            ArtifactEntry::new(
                ArtifactContentClassification::ContainsSecret,
                ArtifactReference::ScenarioInputRevision,
                1,
                None,
            )
            .expect_err("secret classification cannot construct"),
            EvidenceIntegrityError::SecretArtifactRejected
        );
        assert_eq!(
            ArtifactEntry::new(
                ArtifactContentClassification::RawBusinessPayload,
                ArtifactReference::SanitizedLifecycleObservations,
                1,
                None,
            )
            .expect_err("raw classification cannot construct"),
            EvidenceIntegrityError::RawPayloadArtifactRejected
        );
        let snapshot = recorder.snapshot().expect("snapshot records");
        assert_eq!(snapshot.entry_count, 5);
        assert!(
            snapshot
                .counts_by_slot
                .values()
                .all(|entry_count| *entry_count == 1)
        );
    }

    #[tokio::test]
    async fn file_sqlite_controller_proves_real_file_close_and_reopen() {
        let harness = RuntimeRepairProfileConfig::file_sqlite().into_harness();
        let controller = harness
            .evidence_controls()
            .sqlite_reopen_controller()
            .expect("file SQLite profile carries its opaque controller");
        controller
            .verify_close_reopen()
            .await
            .expect("marker survives pool close and reopen");
        assert_eq!(controller.completed_reopens(), 1);
    }

    #[cfg(not(feature = "postgres"))]
    #[tokio::test]
    async fn selected_live_postgres_fails_without_compiled_support() {
        let harness = RuntimeRepairProfileConfig::live_postgres(SecretString::from(
            "postgres://redacted.invalid/required".to_owned(),
        ))
        .into_harness();
        let error = harness
            .evidence_controls()
            .live_postgres_probe()
            .expect("PostgreSQL selection carries a required probe")
            .verify_required()
            .await
            .expect_err("absence is a hard error, never a skip");
        assert_eq!(error, EvidenceIntegrityError::PostgresFeatureNotEnabled);
    }

    #[tokio::test]
    async fn profile_prebinds_reports_readiness_and_joins_shutdown() {
        let harness = RuntimeRepairProfileConfig::in_memory().into_harness();
        let handle = harness.launch().await.expect("closed profile launches");
        assert!(handle.addr().ip().is_loopback());
        assert_ne!(
            handle.addr().port(),
            0,
            "port zero must be resolved at bind"
        );
        handle
            .readiness()
            .await
            .expect("all selected components ready");
        handle.shutdown();
        handle
            .join()
            .await
            .expect("all top-level components joined");
    }

    #[tokio::test]
    async fn file_sqlite_profile_closes_shared_pool_before_same_harness_relaunch() {
        let harness = RuntimeRepairProfileConfig::file_sqlite().into_harness();
        for _ in 1..=2 {
            let handle = harness
                .launch()
                .await
                .expect("same retained file-SQLite harness launches");
            handle
                .readiness()
                .await
                .expect("HTTP, worker, and observer are ready");
            handle.shutdown();
            handle
                .join()
                .await
                .expect("each launch closes its shared pool and joins");
        }
    }
}
