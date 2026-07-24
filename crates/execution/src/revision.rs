//! Immutable identities used to pin execution inputs.
//!
//! These types establish an experimental vocabulary and wire shape for revision pins.
//! The module is available only through the explicitly unstable `unstable-revisions` feature;
//! it is not part of the stable execution contract until runtime, persistence, and admission
//! consume the pins end to end.

use nebula_core::{WorkerFlavorRevisionId, WorkflowVersionId};
use serde::{Deserialize, Serialize};

/// Complete immutable revision vocabulary resolved for an execution.
///
/// This aggregate pins the workflow definition identity and the worker flavor identity together.
/// Adoption by durable execution state and runtime admission is a separate integration step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRevisions {
    workflow: WorkflowVersionId,
    worker_flavor: WorkerFlavorRevisionId,
}

impl ExecutionRevisions {
    /// Creates a complete revision aggregate.
    #[must_use]
    pub const fn new(workflow: WorkflowVersionId, worker_flavor: WorkerFlavorRevisionId) -> Self {
        Self {
            workflow,
            worker_flavor,
        }
    }

    /// Returns the pinned workflow revision.
    #[must_use]
    pub const fn workflow(self) -> WorkflowVersionId {
        self.workflow
    }

    /// Returns the pinned worker-flavor revision.
    #[must_use]
    pub const fn worker_flavor(self) -> WorkerFlavorRevisionId {
        self.worker_flavor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_revisions_round_trip_both_pins_and_reject_unknown_fields() {
        let workflow_version_id = WorkflowVersionId::new();
        let worker_flavor = WorkerFlavorRevisionId::from_bytes([0xa5; 32]);
        let revisions = ExecutionRevisions::new(workflow_version_id, worker_flavor);

        let encoded = serde_json::to_value(revisions).expect("execution revisions must serialize");
        assert_eq!(
            encoded,
            serde_json::json!({
                "workflow": workflow_version_id,
                "worker_flavor": worker_flavor.to_string(),
            })
        );

        let decoded: ExecutionRevisions =
            serde_json::from_value(encoded.clone()).expect("execution revisions must deserialize");
        assert_eq!(decoded, revisions);
        assert_eq!(decoded.workflow(), workflow_version_id);
        assert_eq!(decoded.worker_flavor(), worker_flavor);

        let mut forged = encoded;
        forged
            .as_object_mut()
            .expect("test fixture must be an object")
            .insert("latest".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<ExecutionRevisions>(forged).is_err());
    }
}
