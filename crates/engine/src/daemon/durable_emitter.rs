//! Trigger fan-out through the runtime owner's exact-contract start service.

use std::{future::Future, pin::Pin, sync::Arc};

use nebula_action::{ActionError, ExecutionEmitter, IdempotencyKey};
use nebula_core::{ExecutionId, NodeKey, WorkflowId};
use nebula_storage_port::{Scope, dto::TriggerStartKey};

use crate::{WorkflowStartError, WorkflowStartService};

/// Submits trigger intent to the execution owner.
///
/// The service atomically persists the execution, immutable contract, revision
/// references and one Start command. Natural event identities use the existing
/// scoped trigger namespace and return the original acceptance on redelivery.
#[derive(Clone)]
pub struct DurableExecutionEmitter {
    service: Arc<WorkflowStartService>,
    workflow_id: WorkflowId,
    trigger_id: NodeKey,
    scope: Scope,
}

impl DurableExecutionEmitter {
    /// Bind an authenticated trigger activation to its runtime owner.
    #[must_use]
    pub fn new(
        service: Arc<WorkflowStartService>,
        workflow_id: WorkflowId,
        trigger_id: NodeKey,
        scope: Scope,
    ) -> Self {
        Self {
            service,
            workflow_id,
            trigger_id,
            scope,
        }
    }

    #[tracing::instrument(skip_all, fields(workflow_id = %self.workflow_id, outcome = tracing::field::Empty))]
    async fn do_emit(
        &self,
        input: serde_json::Value,
        event_id: Option<IdempotencyKey>,
    ) -> Result<ExecutionId, ActionError> {
        let result = match &event_id {
            Some(event) => {
                self.service
                    .start_trigger(
                        &self.scope,
                        self.workflow_id,
                        input,
                        TriggerStartKey::new(self.trigger_id.as_str(), event.as_str()),
                        None,
                    )
                    .await
            },
            None => {
                self.service
                    .start(&self.scope, self.workflow_id, Some(input), None, None)
                    .await
            },
        };
        match result {
            Ok(receipt) => {
                tracing::Span::current().record("outcome", "accepted");
                Ok(receipt.state().execution_id)
            },
            Err(error) => {
                tracing::Span::current().record("outcome", "error");
                // An unkeyed re-entry allocates a new identity. It is safe only
                // when the owner knows submission failed before commit.
                let retryable = matches!(error, WorkflowStartError::BackendUnavailable)
                    || (event_id.is_some()
                        && matches!(
                            error,
                            WorkflowStartError::MaterializationIndeterminate(_)
                                | WorkflowStartError::ReceiptUnavailable { .. }
                        ));
                Err(if retryable {
                    ActionError::retryable_from(error)
                } else {
                    ActionError::fatal_from(error)
                })
            },
        }
    }
}

impl std::fmt::Debug for DurableExecutionEmitter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableExecutionEmitter")
            .field("workflow_id", &self.workflow_id)
            .finish_non_exhaustive()
    }
}

impl ExecutionEmitter for DurableExecutionEmitter {
    fn emit(
        &self,
        input: serde_json::Value,
        event_id: Option<IdempotencyKey>,
    ) -> Pin<Box<dyn Future<Output = Result<ExecutionId, ActionError>> + Send + '_>> {
        Box::pin(self.do_emit(input, event_id))
    }
}
