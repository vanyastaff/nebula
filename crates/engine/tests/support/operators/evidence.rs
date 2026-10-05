//! Joint journal/counter/span assertions for independently executed owner decisions.

use nebula_execution::{ExecutionControlOutcome, ExecutionControlSource, JournalEntry};
use nebula_storage_port::dto::JournalEntry as StoredJournalEntry;

use super::{
    capture::TraceObservation,
    metrics::{CounterObservation, outcome_delta},
};

pub(super) const COUNTER: &str = "nebula_execution_control_outcomes_total";

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct CaseEvidence {
    pub scenario: String,
    pub execution_id: String,
    pub org_id: String,
    pub workspace_id: String,
    pub executable_plan_revision_id: nebula_core::ExecutablePlanRevisionId,
    pub worker_flavor_revision_id: nebula_core::WorkerFlavorRevisionId,
    pub expected_source: ExecutionControlSource,
    pub journal: Vec<StoredJournalEntry>,
    /// Rows already journaled when the scenario took `counters_before`.
    pub journal_before: usize,
    pub counters_before: Vec<CounterObservation>,
    pub counters_after: Vec<CounterObservation>,
    pub trace: Vec<TraceObservation>,
}

impl CaseEvidence {
    /// Every durable observation written during the scenario has exactly one
    /// matching counter increment on the scenario's registry, outcome by
    /// outcome; no journal row goes uncounted and no count lacks a row.
    pub(super) fn verify_journal_metric_parity(&self, backend: &str) {
        for outcome in [
            ExecutionControlOutcome::Accepted,
            ExecutionControlOutcome::Fenced,
            ExecutionControlOutcome::Deferred,
            ExecutionControlOutcome::Throttled,
            ExecutionControlOutcome::Recovered,
            ExecutionControlOutcome::FlavorMismatch,
        ] {
            let rows = self
                .journal
                .iter()
                .skip(self.journal_before)
                .filter(|row| {
                    matches!(
                        <JournalEntry as serde::Deserialize>::deserialize(&row.payload),
                        Ok(JournalEntry::ControlObserved { ref observation, .. })
                            if observation.outcome() == outcome
                    )
                })
                .count();
            // Only decisions made while the scenario observed this registry;
            // setup the test drives directly against storage is excluded.
            let counted = outcome_delta(
                &self.counters_before,
                &self.counters_after,
                COUNTER,
                backend,
                outcome.as_str(),
            );
            assert_eq!(
                u64::try_from(rows).expect("row count"),
                counted,
                "{}: journal rows and outcome counter disagree for {}",
                self.scenario,
                outcome.as_str()
            );
        }
    }

    pub(super) fn verify(&self, backend: &str, outcome: ExecutionControlOutcome) {
        let (target, span, reason) = match outcome {
            ExecutionControlOutcome::Accepted => (
                "nebula_engine::engine::resume::lease",
                "prepare_resume_lease",
                "control_accepted",
            ),
            ExecutionControlOutcome::Fenced => (
                "nebula_engine::engine::control_turn",
                "commit_claimed_control",
                "lease_fenced",
            ),
            ExecutionControlOutcome::Deferred => (
                "nebula_engine::engine::control_turn",
                "commit_claimed_control",
                "execution_version_conflict",
            ),
            ExecutionControlOutcome::Throttled => (
                "nebula_engine::engine",
                "record_admission_refusal",
                "admission_throttled",
            ),
            ExecutionControlOutcome::Recovered => (
                "nebula_engine::engine::resume::lease",
                "prepare_resume_lease",
                "accepted_turn_recovered",
            ),
            ExecutionControlOutcome::FlavorMismatch => (
                "nebula_engine::engine::resume::recovery",
                "record_claimed_flavor_refusal",
                "exact_flavor_mismatch",
            ),
        };
        let observations = self
            .journal
            .iter()
            .filter_map(|row| {
                assert!(
                    row.seq.is_some(),
                    "journal witness must have a backend-assigned sequence"
                );
                match <JournalEntry as serde::Deserialize>::deserialize(&row.payload)
                    .expect("durable journal witness decodes through the closed protocol")
                {
                    JournalEntry::ControlObserved { observation, .. }
                        if observation.outcome() == outcome =>
                    {
                        Some(observation)
                    },
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        assert!(
            !observations.is_empty(),
            "missing durable owner observation for {}",
            outcome.as_str()
        );
        assert!(
            observations
                .iter()
                .any(|record| record.source() == &self.expected_source
                    && record.reason().as_str() == reason),
            "journal observation must retain the actual delivery/accepted-turn identity"
        );
        for record in observations
            .iter()
            .filter(|record| record.source() == &self.expected_source)
        {
            match record.reason() {
                nebula_execution::ExecutionControlReason::LeaseFenced {
                    attempted_execution_lease_generation,
                    current_execution_lease_generation,
                } => {
                    assert!(
                        current_execution_lease_generation > attempted_execution_lease_generation,
                        "real takeover must advance the durable generation"
                    );
                    assert_eq!(
                        record.execution_lease_generation(),
                        *current_execution_lease_generation
                    );
                },
                nebula_execution::ExecutionControlReason::ExecutionVersionConflict {
                    expected_version,
                    actual_version,
                } => assert_eq!(
                    *actual_version,
                    expected_version + 1,
                    "the real concurrent checkpoint must distinguish the stale version"
                ),
                nebula_execution::ExecutionControlReason::ExactFlavorMismatch {
                    expected,
                    actual,
                } => {
                    assert_eq!(expected, &self.worker_flavor_revision_id);
                    assert_ne!(actual, expected);
                },
                _ => {},
            }
        }
        assert!(
            outcome_delta(
                &self.counters_before,
                &self.counters_after,
                COUNTER,
                backend,
                outcome.as_str()
            ) > 0,
            "missing independent operator metric for {}",
            outcome.as_str()
        );
        assert!(
            self.counters_after
                .iter()
                .any(|counter| counter.name == COUNTER
                    && counter
                        .labels
                        .get("backend")
                        .is_some_and(|label| label == backend)
                    && counter
                        .labels
                        .get("outcome")
                        .is_some_and(|label| label == outcome.as_str())
                    && counter.labels.len() == 2),
            "metric uses only closed backend/outcome labels"
        );
        assert!(
            self.trace.iter().any(|record| record.target == target
                && record.name == span
                && record
                    .fields
                    .get("execution_id")
                    .is_some_and(|value| value == &self.execution_id)
                && record
                    .fields
                    .get("org_id")
                    .is_some_and(|value| value == &self.org_id)
                && record
                    .fields
                    .get("workspace_id")
                    .is_some_and(|value| value == &self.workspace_id)
                && record
                    .fields
                    .get("outcome")
                    .is_some_and(|value| value == outcome.as_str())
                && record
                    .fields
                    .get("backend")
                    .is_some_and(|value| value == backend)),
            "missing independently captured attributed operator span for {}",
            outcome.as_str()
        );
    }
}
