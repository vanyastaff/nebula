//! Immutable identities used to pin execution inputs.
//!
//! These types establish the shared vocabulary and wire shape for revision pins.
//! They do not migrate existing runtime or persistence consumers by themselves.

use std::{fmt, str::FromStr};

use nebula_core::{UlidParseError, WorkerFlavorRevisionId, WorkflowVersionId};
use serde::{Deserialize, Serialize};

/// Execution-facing semantic type for the canonical identity of a published workflow version.
///
/// The inner [`WorkflowVersionId`] remains the only workflow-version identity. A human-facing
/// numeric publication number is metadata and is deliberately not part of this value.
/// Serialization is transparent, so this type has exactly the same wire representation as
/// [`WorkflowVersionId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkflowRevision(WorkflowVersionId);

impl WorkflowRevision {
    /// Wraps the canonical workflow-version identity for execution use.
    #[must_use]
    pub const fn new(workflow_version_id: WorkflowVersionId) -> Self {
        Self(workflow_version_id)
    }

    /// Returns the canonical workflow-version identity.
    #[must_use]
    pub const fn workflow_version_id(self) -> WorkflowVersionId {
        self.0
    }
}

impl From<WorkflowVersionId> for WorkflowRevision {
    fn from(workflow_version_id: WorkflowVersionId) -> Self {
        Self::new(workflow_version_id)
    }
}

impl From<WorkflowRevision> for WorkflowVersionId {
    fn from(revision: WorkflowRevision) -> Self {
        revision.workflow_version_id()
    }
}

impl fmt::Display for WorkflowRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl FromStr for WorkflowRevision {
    type Err = UlidParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<WorkflowVersionId>().map(Self::new)
    }
}

/// Complete immutable revision vocabulary resolved for an execution.
///
/// This aggregate pins the workflow definition identity and the worker flavor identity together.
/// Adoption by durable execution state and runtime admission is a separate integration step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRevisions {
    workflow: WorkflowRevision,
    worker_flavor: WorkerFlavorRevisionId,
}

impl ExecutionRevisions {
    /// Creates a complete revision aggregate.
    #[must_use]
    pub const fn new(workflow: WorkflowRevision, worker_flavor: WorkerFlavorRevisionId) -> Self {
        Self {
            workflow,
            worker_flavor,
        }
    }

    /// Returns the pinned workflow revision.
    #[must_use]
    pub const fn workflow(self) -> WorkflowRevision {
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
    use nebula_core::{UlidParseError, WorkflowId};

    use super::*;

    #[test]
    fn workflow_revision_is_transparent_and_preserves_canonical_identity() {
        let workflow_version_id = WorkflowVersionId::new();
        let revision = WorkflowRevision::new(workflow_version_id);

        let canonical_json =
            serde_json::to_value(workflow_version_id).expect("workflow version must serialize");
        let revision_json =
            serde_json::to_value(revision).expect("workflow revision must serialize");
        assert_eq!(revision_json, canonical_json);

        let decoded: WorkflowRevision =
            serde_json::from_value(revision_json).expect("workflow revision must deserialize");
        assert_eq!(decoded, revision);
        assert_eq!(decoded.workflow_version_id(), workflow_version_id);

        let converted: WorkflowVersionId = WorkflowRevision::from(workflow_version_id).into();
        assert_eq!(converted, workflow_version_id);
    }

    #[test]
    fn workflow_revision_display_and_parse_round_trip_canonical_wire_value() {
        let workflow_version_id = WorkflowVersionId::new();
        let revision = WorkflowRevision::new(workflow_version_id);

        let encoded = revision.to_string();
        assert_eq!(encoded, workflow_version_id.to_string());

        let decoded: Result<WorkflowRevision, UlidParseError> = encoded.parse();
        assert_eq!(decoded, Ok(revision));

        let wrong_domain = WorkflowId::new().to_string().parse::<WorkflowRevision>();
        assert!(matches!(
            wrong_domain,
            Err(UlidParseError::WrongPrefix {
                expected_prefix: "wfv"
            })
        ));
    }

    #[test]
    fn workflow_revision_serde_rejects_non_canonical_shapes_and_domains() {
        let workflow_version_id = WorkflowVersionId::new();
        let wrong_domain = WorkflowId::new().to_string();

        for forged in [
            serde_json::json!({"workflow_version_id": workflow_version_id}),
            serde_json::json!(wrong_domain),
            serde_json::json!("wfv_not-a-ulid"),
            serde_json::json!(1),
        ] {
            assert!(serde_json::from_value::<WorkflowRevision>(forged).is_err());
        }
    }

    #[test]
    fn execution_revisions_round_trip_both_pins_and_reject_unknown_fields() {
        let workflow_version_id = WorkflowVersionId::new();
        let workflow = WorkflowRevision::new(workflow_version_id);
        let worker_flavor = WorkerFlavorRevisionId::from_bytes([0xa5; 32]);
        let revisions = ExecutionRevisions::new(workflow, worker_flavor);

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
        assert_eq!(decoded.workflow(), workflow);
        assert_eq!(decoded.worker_flavor(), worker_flavor);

        let mut forged = encoded;
        forged
            .as_object_mut()
            .expect("test fixture must be an object")
            .insert("latest".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<ExecutionRevisions>(forged).is_err());
    }
}
