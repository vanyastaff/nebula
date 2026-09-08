//! Compile, install, and atomically publish an exact workflow activation.

use std::{fmt, sync::Arc};

use nebula_core::{WorkflowId, WorkflowVersionId, accessor::Clock};
use nebula_plugin::{FrozenPluginRegistry, PlanCompilationError};
use nebula_storage_port::{
    PlanFlavorRevisionIds, Scope, StorageError,
    dto::{WorkflowActivation, WorkflowVersionRecord},
    store::{WorkflowPublicationError, WorkflowStore, WorkflowVersionStore},
};
use nebula_workflow::WorkflowDefinition;

use crate::PlanFlavorRevisionInstaller;

/// Immutable receipt of an acknowledged or exactly reconciled publication.
#[derive(Clone)]
pub struct WorkflowActivationReceipt {
    version: WorkflowVersionRecord,
}

impl WorkflowActivationReceipt {
    /// The original persisted version, including its normalized definition timestamp.
    #[must_use]
    pub const fn version(&self) -> &WorkflowVersionRecord {
        &self.version
    }

    /// Consume the receipt and return the original persisted version.
    #[must_use]
    pub fn into_version(self) -> WorkflowVersionRecord {
        self.version
    }
}

impl fmt::Debug for WorkflowActivationReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowActivationReceipt")
            .field("number", &self.version.number)
            .field("activation", &self.version.activation)
            .finish_non_exhaustive()
    }
}

/// Identity of a publication whose commit could not be established.
#[derive(Clone)]
pub struct IndeterminateWorkflowPublication {
    scope: Scope,
    workflow_id: WorkflowId,
    number: u32,
    activation: WorkflowActivation,
}

impl IndeterminateWorkflowPublication {
    /// Authenticated host scope used for the original attempt.
    #[must_use]
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }
    /// Workflow targeted by the original attempt.
    #[must_use]
    pub const fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
    /// Exact attempted version number; never a latest-version selector.
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.number
    }
    /// Exact original compiler and catalog identities.
    #[must_use]
    pub const fn activation(&self) -> WorkflowActivation {
        self.activation
    }
}

impl fmt::Debug for IndeterminateWorkflowPublication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndeterminateWorkflowPublication")
            .field("workflow_id", &self.workflow_id)
            .field("number", &self.number)
            .field("activation", &self.activation)
            .finish_non_exhaustive()
    }
}

/// Redacted activation failures; storage driver text is never retained.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkflowActivationError {
    /// No active workflow exists in the supplied scope.
    #[error("workflow not found")]
    MissingWorkflow,
    /// The expected aggregate version lost its compare-and-swap race.
    #[error("workflow version conflict")]
    CasConflict,
    /// The definition identity, serialization, or version progression is invalid.
    #[error("invalid workflow definition")]
    InvalidDefinition,
    /// Compilation rejected the definition; typed diagnostics remain available.
    #[error("workflow compilation failed")]
    Compilation(#[source] PlanCompilationError),
    /// The compiled graph still contains bindings this runtime cannot resolve.
    #[error("workflow contains unresolved runtime bindings")]
    UnresolvedBindings,
    /// The compiled graph requests semantics the recorded runtime cannot preserve.
    #[error("workflow contains unsupported recorded runtime semantics")]
    UnsupportedRecordedSemantics,
    /// The exact compiled revisions could not be admitted for publication.
    #[error("workflow revisions are not admitted")]
    RevisionNotAdmitted,
    /// Exact revision installation may have committed, but its acknowledgement was lost.
    #[error("workflow revision installation outcome is indeterminate")]
    RevisionInstallationIndeterminate,
    /// A backend operation failed before publication commit submission.
    #[error("workflow activation backend unavailable")]
    BackendUnavailable,
    /// Commit may have happened; retrying with new identities is unsafe.
    #[error("workflow publication outcome is indeterminate")]
    PublicationIndeterminate(Box<IndeterminateWorkflowPublication>),
}

/// Runtime-owned workflow activation orchestration over scoped persistence ports.
///
/// Hosts must authenticate and authorize the supplied scope before calling this service.
pub struct WorkflowActivationService {
    workflows: Arc<dyn WorkflowStore>,
    versions: Arc<dyn WorkflowVersionStore>,
    registry: Arc<FrozenPluginRegistry>,
    installer: PlanFlavorRevisionInstaller,
    clock: Arc<dyn Clock>,
}

impl WorkflowActivationService {
    /// Compose activation with the deployment's shared catalog and scoped stores.
    #[must_use]
    pub fn new(
        workflows: Arc<dyn WorkflowStore>,
        versions: Arc<dyn WorkflowVersionStore>,
        registry: Arc<FrozenPluginRegistry>,
        installer: PlanFlavorRevisionInstaller,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            workflows,
            versions,
            registry,
            installer,
            clock,
        }
    }

    /// Routing context derived from the exact snapshot used for compilation.
    #[must_use]
    pub fn worker_flavor_context(&self) -> nebula_plugin::WorkerFlavorContext {
        nebula_plugin::WorkerFlavorContext::from_registry(&self.registry)
    }

    /// Validate a definition against the same frozen compiler snapshot used
    /// by activation, without installing or publishing the resulting plan.
    ///
    /// # Errors
    /// Returns the compiler's payload-free activation diagnostics.
    pub fn validate_definition(
        &self,
        definition: &WorkflowDefinition,
    ) -> Result<(), PlanCompilationError> {
        self.registry
            .compile_graph_v1(WorkflowVersionId::new(), definition)
            .map(drop)
    }

    /// Publish a compiled definition using the expected aggregate version.
    ///
    /// # Errors
    /// Returns typed validation, admission, CAS, backend, or indeterminate-commit errors.
    #[tracing::instrument(skip_all, fields(%workflow_id, expected_version, outcome = tracing::field::Empty, error_code = tracing::field::Empty))]
    pub async fn activate(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        expected_version: u64,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowActivationReceipt, WorkflowActivationError> {
        let result = self
            .activate_inner(scope, workflow_id, expected_version, definition)
            .await;
        tracing::Span::current().record(
            "outcome",
            match &result {
                Ok(_) => "published",
                Err(
                    WorkflowActivationError::PublicationIndeterminate(_)
                    | WorkflowActivationError::RevisionInstallationIndeterminate,
                ) => "indeterminate",
                Err(_) => "rejected",
            },
        );
        if let Err(error) = &result {
            tracing::Span::current().record(
                "error_code",
                match error {
                    WorkflowActivationError::MissingWorkflow => "WORKFLOW_ACTIVATION:MISSING",
                    WorkflowActivationError::CasConflict => "WORKFLOW_ACTIVATION:CAS_CONFLICT",
                    WorkflowActivationError::InvalidDefinition => {
                        "WORKFLOW_ACTIVATION:INVALID_DEFINITION"
                    },
                    WorkflowActivationError::Compilation(_) => "WORKFLOW_ACTIVATION:COMPILATION",
                    WorkflowActivationError::UnresolvedBindings => {
                        "WORKFLOW_ACTIVATION:UNRESOLVED_BINDINGS"
                    },
                    WorkflowActivationError::UnsupportedRecordedSemantics => {
                        "WORKFLOW_ACTIVATION:UNSUPPORTED_RECORDED_SEMANTICS"
                    },
                    WorkflowActivationError::RevisionNotAdmitted => {
                        "WORKFLOW_ACTIVATION:REVISION_NOT_ADMITTED"
                    },
                    WorkflowActivationError::RevisionInstallationIndeterminate => {
                        "WORKFLOW_ACTIVATION:REVISION_INSTALLATION_INDETERMINATE"
                    },
                    WorkflowActivationError::BackendUnavailable => {
                        "WORKFLOW_ACTIVATION:BACKEND_UNAVAILABLE"
                    },
                    WorkflowActivationError::PublicationIndeterminate(_) => {
                        "WORKFLOW_ACTIVATION:INDETERMINATE"
                    },
                },
            );
        }
        result
    }

    async fn activate_inner(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        expected_version: u64,
        mut definition: WorkflowDefinition,
    ) -> Result<WorkflowActivationReceipt, WorkflowActivationError> {
        let workflow_key = workflow_id.to_string();
        let mut row = self
            .workflows
            .get(scope, &workflow_key)
            .await
            .map_err(map_storage_error)?
            .filter(|row| !row.deleted)
            .ok_or(WorkflowActivationError::MissingWorkflow)?;
        if row.version != expected_version {
            return Err(WorkflowActivationError::CasConflict);
        }
        if definition.id != workflow_id || row.scope != *scope || row.id != workflow_key {
            return Err(WorkflowActivationError::InvalidDefinition);
        }
        let next = expected_version
            .checked_add(1)
            .ok_or(WorkflowActivationError::InvalidDefinition)?;
        let number = u32::try_from(next).map_err(|_| WorkflowActivationError::InvalidDefinition)?;
        definition.updated_at = self.clock.now();
        let workflow_revision = WorkflowVersionId::new();
        let plan = self
            .registry
            .compile_graph_v1(workflow_revision, &definition)
            .map_err(WorkflowActivationError::Compilation)?;
        let graph = plan
            .execution_graph()
            .map_err(|_| WorkflowActivationError::UnsupportedRecordedSemantics)?;
        crate::recorded_graph::validate_recorded_graph(&graph).map_err(
            |rejection| match rejection {
                crate::recorded_graph::RecordedGraphRejection::UnresolvedBindings => {
                    WorkflowActivationError::UnresolvedBindings
                },
                crate::recorded_graph::RecordedGraphRejection::UnsupportedSemantics => {
                    WorkflowActivationError::UnsupportedRecordedSemantics
                },
            },
        )?;
        let activation = WorkflowActivation::new(
            workflow_revision,
            PlanFlavorRevisionIds::new(plan.id(), plan.worker_flavor_revision_id()),
        );
        let version = WorkflowVersionRecord {
            activation: Some(activation),
            workflow_id: workflow_key.clone(),
            number,
            published: true,
            pinned: false,
            definition: serde_json::to_value(definition)
                .map_err(|_| WorkflowActivationError::InvalidDefinition)?,
        };
        self.installer
            .install(&self.registry, &plan)
            .await
            .map_err(map_install_error)?;
        row.version = next;
        // Retain the exact original payload to prove a lost acknowledgement without
        // selecting the current pointer or allocating another compiler identity.
        match self
            .workflows
            .publish_activated_version(scope, row, version.clone(), expected_version)
            .await
        {
            Ok(()) => Ok(WorkflowActivationReceipt { version }),
            Err(WorkflowPublicationError::OutcomeUnknown) => {
                match self.versions.get(scope, &workflow_key, number).await {
                    Ok(Some(persisted)) if persisted == version => {
                        Ok(WorkflowActivationReceipt { version: persisted })
                    },
                    _ => Err(WorkflowActivationError::PublicationIndeterminate(Box::new(
                        IndeterminateWorkflowPublication {
                            scope: scope.clone(),
                            workflow_id,
                            number,
                            activation,
                        },
                    ))),
                }
            },
            Err(WorkflowPublicationError::Storage(error)) => Err(map_storage_error(error)),
            Err(WorkflowPublicationError::RevisionNotAdmitted) => {
                Err(WorkflowActivationError::RevisionNotAdmitted)
            },
            Err(WorkflowPublicationError::InvalidPublication) => {
                Err(WorkflowActivationError::InvalidDefinition)
            },
            Err(_) => Err(WorkflowActivationError::BackendUnavailable),
        }
    }
}

fn map_storage_error(error: StorageError) -> WorkflowActivationError {
    match error {
        StorageError::NotFound { .. } | StorageError::ScopeViolation { .. } => {
            WorkflowActivationError::MissingWorkflow
        },
        StorageError::Conflict { .. } | StorageError::Duplicate { .. } => {
            WorkflowActivationError::CasConflict
        },
        _ => WorkflowActivationError::BackendUnavailable,
    }
}

fn map_install_error(error: crate::PlanFlavorRevisionBridgeError) -> WorkflowActivationError {
    use nebula_storage_port::RevisionCatalogError;
    match error {
        crate::PlanFlavorRevisionBridgeError::Catalog {
            source: RevisionCatalogError::Unavailable,
        } => WorkflowActivationError::BackendUnavailable,
        crate::PlanFlavorRevisionBridgeError::Catalog {
            source: RevisionCatalogError::OutcomeUnknown,
        } => WorkflowActivationError::RevisionInstallationIndeterminate,
        crate::PlanFlavorRevisionBridgeError::Catalog {
            source:
                RevisionCatalogError::PlanUnavailable { .. }
                | RevisionCatalogError::WorkerFlavorUnavailable { .. }
                | RevisionCatalogError::PlanFlavorMismatch { .. }
                | RevisionCatalogError::ContentConflict { .. }
                | RevisionCatalogError::Draining { .. }
                | RevisionCatalogError::Deleted { .. },
        } => WorkflowActivationError::RevisionNotAdmitted,
        _ => WorkflowActivationError::InvalidDefinition,
    }
}

impl fmt::Debug for WorkflowActivationService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowActivationService")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
