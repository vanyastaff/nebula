//! Runtime-owned, exact-contract start admission and original-command replay.

use std::{fmt, sync::Arc};

use nebula_core::{
    ExecutionContractBundleId, ExecutionId, OrgId, W3cTraceContext, WorkflowId, WorkspaceId,
    accessor::Clock,
};
use nebula_execution::{
    ExecutionContractBundle, ExecutionRevisions, ExecutionState, context::ExecutionBudget,
};
use nebula_plugin::FrozenPluginRegistry;
use nebula_storage_port::{
    Scope,
    dto::{
        ContractBundleRecord, ControlCommand, ControlMsg, MaterializedStart, NewExecution,
        StartKey, TriggerStartKey,
    },
    store::{
        ExecutionStore, StartAcceptanceStore, StartContractIdentity, StartFingerprint,
        StartMaterialization, StartMaterializationError, StartRevisionRejection,
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{PlanFlavorRevisionBridgeError, PlanFlavorRevisionLoader, WorkflowStores};

/// Whether this request created an execution or recovered an existing acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStartDisposition {
    /// The owner transaction acknowledged a new execution.
    Accepted,
    /// The owner returned the original execution for an equivalent command.
    Replayed,
}

/// Receipt read from persisted state and its checked immutable bundle.
pub struct WorkflowStartReceipt {
    state: ExecutionState,
    bundle: ExecutionContractBundle,
    disposition: WorkflowStartDisposition,
}
impl WorkflowStartReceipt {
    /// Actual persisted state, including its original timestamps and input.
    #[must_use]
    pub const fn state(&self) -> &ExecutionState {
        &self.state
    }
    /// Checked original bundle; no current catalog lookup is needed for replay.
    #[must_use]
    pub const fn bundle(&self) -> &ExecutionContractBundle {
        &self.bundle
    }
    /// Owner transaction disposition.
    #[must_use]
    pub const fn disposition(&self) -> WorkflowStartDisposition {
        self.disposition
    }
}
impl fmt::Debug for WorkflowStartReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowStartReceipt")
            .field("execution_id", &self.state.execution_id)
            .field("bundle_id", &self.bundle.bundle_id())
            .field("disposition", &self.disposition)
            .finish_non_exhaustive()
    }
}

/// Invalid deployment configuration for runtime-owned start admission.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkflowStartBuildError {
    /// Concurrency must fit the runtime and be positive.
    #[error("invalid workflow start execution budget")]
    InvalidBudget,
}

/// Payload-redacted start admission failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkflowStartError {
    /// Scope does not contain canonical typed tenant identifiers.
    #[error("invalid workflow start scope")]
    InvalidScope,
    /// A supplied key must contain 1 to 255 bytes after trimming.
    #[error("invalid workflow start key")]
    InvalidKey,
    /// Input cannot be represented by the persisted JSON contract.
    #[error("invalid workflow start input")]
    InvalidInput,
    /// No active workflow exists in the authenticated scope.
    #[error("workflow not found")]
    MissingWorkflow,
    /// The published version is a draft or legacy version without activation pins.
    #[error("workflow has no exact activation")]
    WorkflowNotActivated,
    /// The stored activation and exact checked plan identities disagree.
    #[error("stored workflow activation is inconsistent")]
    InvalidActivation,
    /// Exact catalog loading or integrity validation failed before materialization.
    #[error("exact workflow revisions are unavailable")]
    RevisionUnavailable(#[source] Box<PlanFlavorRevisionBridgeError>),
    /// The owner refused a new reference to the exact revisions.
    #[error("workflow revisions are not admitted")]
    RevisionNotAdmitted(StartRevisionRejection),
    /// Binding requirements have no authenticated owner-scoped resolution yet.
    #[error("workflow bindings cannot be admitted")]
    UnsupportedBindings,
    /// The runtime does not execute some recorded graph semantics yet.
    #[error("recorded workflow semantics are unsupported")]
    UnsupportedRecordedSemantics,
    /// This key already identifies a different caller intent or fingerprint version.
    #[error("workflow start key identifies a different command")]
    FingerprintMismatch,
    /// An acknowledged start cannot currently produce a checked persisted receipt.
    #[error("accepted workflow start receipt is unavailable; execution_id={execution_id}")]
    ReceiptUnavailable {
        /// Known accepted execution; submitting another unkeyed start is unsafe.
        execution_id: ExecutionId,
    },
    /// A persisted reservation contains an invalid execution identifier.
    #[error("stored workflow start receipt is inconsistent")]
    InvalidReceipt,
    /// No authoritative owner reply proved the original transaction's outcome.
    #[error("workflow start materialization outcome is indeterminate; {0}")]
    MaterializationIndeterminate(Box<IndeterminateWorkflowStart>),
    /// A fresh envelope was rejected before commit submission.
    #[error("workflow start materialization envelope was rejected")]
    MaterializationRejected,
    /// Backend operation failed before start submission.
    #[error("workflow start backend unavailable")]
    BackendUnavailable,
}

/// Original uncertain attempt, retained without exposing its command or sensitive input.
pub struct IndeterminateWorkflowStart {
    attempt: OriginalStart,
}
impl IndeterminateWorkflowStart {
    /// Identity allocated before the original transaction was submitted.
    #[must_use]
    pub const fn execution_id(&self) -> ExecutionId {
        self.attempt.execution_id
    }
    /// Original workflow selection.
    #[must_use]
    pub const fn workflow_id(&self) -> WorkflowId {
        self.attempt.workflow_id
    }
    /// Original immutable bundle identity.
    #[must_use]
    pub const fn bundle_id(&self) -> ExecutionContractBundleId {
        self.attempt.bundle.identity().bundle_id()
    }
}
impl fmt::Debug for IndeterminateWorkflowStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndeterminateWorkflowStart")
            .field("execution_id", &self.execution_id())
            .field("workflow_id", &self.workflow_id())
            .field("bundle_id", &self.bundle_id())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for IndeterminateWorkflowStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "execution_id={}", self.execution_id())
    }
}

struct OriginalStart {
    scope: Scope,
    workflow_id: WorkflowId,
    workflow_key: String,
    execution_id: ExecutionId,
    key: OriginalKey,
    initial_state: Value,
    command: ControlMsg,
    bundle: ContractBundleRecord,
}
enum OriginalKey {
    Unkeyed,
    Caller(String, StartFingerprint),
    Trigger(String, String),
}
impl OriginalStart {
    fn envelope(&self) -> MaterializedStart<'_> {
        if let OriginalKey::Trigger(trigger_id, event_id) = &self.key {
            return MaterializedStart::for_trigger(
                &self.scope,
                TriggerStartKey::new(trigger_id, event_id),
                &self.command.execution_id,
                NewExecution::new(&self.workflow_key, &self.initial_state),
                &self.command,
                &self.bundle,
            );
        }
        MaterializedStart::new(
            &self.scope,
            match &self.key {
                OriginalKey::Caller(key, fingerprint) => Some(StartKey::new(key, *fingerprint)),
                _ => None,
            },
            &self.command.execution_id,
            NewExecution::new(&self.workflow_key, &self.initial_state),
            &self.command,
            &self.bundle,
        )
    }
}

/// Runtime owner of keyed and unkeyed execution materialization.
/// Hosts authenticate and authorize scope before submitting caller intent.
pub struct WorkflowStartService {
    workflows: WorkflowStores,
    executions: Arc<dyn ExecutionStore>,
    starts: Arc<dyn StartAcceptanceStore>,
    loader: PlanFlavorRevisionLoader,
    registry: Arc<FrozenPluginRegistry>,
    clock: Arc<dyn Clock>,
    budget: ExecutionBudget,
}

impl WorkflowStartService {
    /// Compose the same backend's ports and the deployment's frozen registry.
    ///
    /// # Errors
    /// Rejects an execution budget that cannot be scheduled by this runtime.
    pub fn new(
        workflows: WorkflowStores,
        executions: Arc<dyn ExecutionStore>,
        starts: Arc<dyn StartAcceptanceStore>,
        loader: PlanFlavorRevisionLoader,
        registry: Arc<FrozenPluginRegistry>,
        clock: Arc<dyn Clock>,
        budget: ExecutionBudget,
    ) -> Result<Self, WorkflowStartBuildError> {
        budget
            .validate_for_execution()
            .map_err(|_| WorkflowStartBuildError::InvalidBudget)?;
        if budget.max_concurrent_nodes > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(WorkflowStartBuildError::InvalidBudget);
        }
        Ok(Self {
            workflows,
            executions,
            starts,
            loader,
            registry,
            clock,
            budget,
        })
    }

    /// Submit caller intent through the execution owner's atomic transaction.
    ///
    /// # Errors
    /// Returns typed admission failures or an explicitly uncertain outcome.
    #[tracing::instrument(skip_all, fields(%workflow_id, outcome = tracing::field::Empty, error_code = tracing::field::Empty))]
    pub async fn start(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        input: Option<Value>,
        start_key: Option<&str>,
        trace: Option<W3cTraceContext>,
    ) -> Result<WorkflowStartReceipt, WorkflowStartError> {
        let result = self
            .start_inner(scope, workflow_id, input, start_key, trace)
            .await;
        let span = tracing::Span::current();
        match &result {
            Ok(receipt) => {
                span.record(
                    "outcome",
                    match receipt.disposition {
                        WorkflowStartDisposition::Accepted => "accepted",
                        WorkflowStartDisposition::Replayed => "replayed",
                    },
                );
            },
            Err(error) => {
                span.record("outcome", "error");
                span.record("error_code", error.code());
            },
        }
        result
    }

    /// Admit a trigger's natural event identity in its separate durable namespace.
    /// A repeated event returns its original execution regardless of payload changes.
    ///
    /// # Errors
    /// Returns admission failures or the known identity of an uncertain acceptance.
    #[tracing::instrument(skip_all, fields(%workflow_id, outcome = tracing::field::Empty, error_code = tracing::field::Empty))]
    pub async fn start_trigger(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        input: Value,
        key: TriggerStartKey<'_>,
        trace: Option<W3cTraceContext>,
    ) -> Result<WorkflowStartReceipt, WorkflowStartError> {
        let result = async {
            tenant_ids(scope)?;
            if key.trigger_id().is_empty() || key.event_id().is_empty() {
                return Err(WorkflowStartError::InvalidKey);
            }
            if let Some(receipt) = self.replay_trigger(scope, workflow_id, key).await? {
                return Ok(receipt);
            }
            let mut attempt = self
                .prepare_start(scope, workflow_id, input, None, trace)
                .await?;
            attempt.key =
                OriginalKey::Trigger(key.trigger_id().to_owned(), key.event_id().to_owned());
            self.submit_start(attempt).await
        }
        .await;
        let span = tracing::Span::current();
        match &result {
            Ok(receipt) => {
                span.record(
                    "outcome",
                    match receipt.disposition() {
                        WorkflowStartDisposition::Accepted => "accepted",
                        WorkflowStartDisposition::Replayed => "replayed",
                    },
                );
            },
            Err(error) => {
                span.record("outcome", "error");
                span.record("error_code", error.code());
            },
        }
        result
    }

    async fn replay_trigger(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        key: TriggerStartKey<'_>,
    ) -> Result<Option<WorkflowStartReceipt>, WorkflowStartError> {
        let execution = self
            .starts
            .lookup_trigger_start(scope, &key)
            .await
            .map_err(|_| WorkflowStartError::BackendUnavailable)?;
        match execution {
            Some(execution) => self
                .read_receipt(
                    scope,
                    workflow_id,
                    &execution,
                    WorkflowStartDisposition::Replayed,
                )
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    async fn start_inner(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        input: Option<Value>,
        start_key: Option<&str>,
        trace: Option<W3cTraceContext>,
    ) -> Result<WorkflowStartReceipt, WorkflowStartError> {
        tenant_ids(scope)?;
        let mut input = input.unwrap_or(Value::Null);
        let key = match start_key.map(str::trim) {
            Some(key) if key.is_empty() || key.len() > 255 => {
                return Err(WorkflowStartError::InvalidKey);
            },
            Some(key) => Some(StartKey::new(
                key,
                caller_intent_fingerprint(workflow_id, &mut input)?,
            )),
            None => None,
        };
        if let Some(key) = key
            && let Some(receipt) = self.replay_key(scope, workflow_id, key).await?
        {
            return Ok(receipt);
        }
        let attempt = self
            .prepare_start(scope, workflow_id, input, key, trace)
            .await?;
        self.submit_start(attempt).await
    }

    async fn submit_start(
        &self,
        attempt: OriginalStart,
    ) -> Result<WorkflowStartReceipt, WorkflowStartError> {
        let result = self.starts.materialize_start(&attempt.envelope()).await;
        match result {
            Ok(outcome) => {
                self.materialized_receipt(&attempt.scope, attempt.workflow_id, outcome)
                    .await
            },
            Err(StartMaterializationError::OutcomeUnknown) => self.reconcile_unknown(attempt).await,
            Err(StartMaterializationError::Storage(_)) => {
                Err(WorkflowStartError::BackendUnavailable)
            },
            Err(_) => Err(WorkflowStartError::MaterializationRejected),
        }
    }

    async fn replay_key(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        key: StartKey<'_>,
    ) -> Result<Option<WorkflowStartReceipt>, WorkflowStartError> {
        let reservation = self
            .starts
            .lookup_start(scope, key.key())
            .await
            .map_err(|_| WorkflowStartError::BackendUnavailable)?;
        let Some(reservation) = reservation else {
            return Ok(None);
        };
        if reservation.fingerprint() != key.fingerprint() {
            return Err(WorkflowStartError::FingerprintMismatch);
        }
        self.read_receipt(
            scope,
            workflow_id,
            reservation.execution_id(),
            WorkflowStartDisposition::Replayed,
        )
        .await
        .map(Some)
    }

    async fn prepare_start(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        input: Value,
        key: Option<StartKey<'_>>,
        trace: Option<W3cTraceContext>,
    ) -> Result<OriginalStart, WorkflowStartError> {
        let workflow_key = workflow_id.to_string();
        let row = self
            .workflows
            .workflow
            .get(scope, &workflow_key)
            .await
            .map_err(|_| WorkflowStartError::BackendUnavailable)?
            .filter(|row| !row.deleted)
            .ok_or(WorkflowStartError::MissingWorkflow)?;
        if row.scope != *scope || row.id != workflow_key {
            return Err(WorkflowStartError::InvalidActivation);
        }
        let version = self
            .workflows
            .versions
            .get_published(scope, &workflow_key)
            .await
            .map_err(|_| WorkflowStartError::BackendUnavailable)?
            .ok_or(WorkflowStartError::WorkflowNotActivated)?;
        let activation = version
            .activation
            .ok_or(WorkflowStartError::WorkflowNotActivated)?;
        if version.workflow_id != workflow_key || version.number == 0 || !version.published {
            return Err(WorkflowStartError::InvalidActivation);
        }
        let loaded = self
            .loader
            .load_exact(activation.revisions(), self.registry.clone())
            .await
            .map_err(|error| WorkflowStartError::RevisionUnavailable(Box::new(error)))?;
        let plan = loaded.plan();
        if plan.workflow_id() != workflow_id
            || plan.workflow_version_id() != activation.workflow_version_id()
        {
            return Err(WorkflowStartError::InvalidActivation);
        }
        let graph = plan
            .execution_graph()
            .map_err(|_| WorkflowStartError::UnsupportedRecordedSemantics)?;
        crate::recorded_graph::validate_recorded_graph(&graph).map_err(
            |rejection| match rejection {
                crate::recorded_graph::RecordedGraphRejection::UnresolvedBindings => {
                    WorkflowStartError::UnsupportedBindings
                },
                crate::recorded_graph::RecordedGraphRejection::UnsupportedSemantics => {
                    WorkflowStartError::UnsupportedRecordedSemantics
                },
            },
        )?;
        let (org_id, workspace_id) = tenant_ids(scope)?;
        let bundle = ExecutionContractBundle::new_graph_v1(
            ExecutionContractBundleId::new(),
            org_id,
            workspace_id,
            plan.id(),
            plan.plugin_set_id(),
            ExecutionRevisions::new(
                activation.workflow_version_id(),
                plan.worker_flavor_revision_id(),
            ),
            [],
        );
        let record = ContractBundleRecord::v1_json(
            StartContractIdentity::new(bundle.bundle_id(), activation.revisions()),
            serde_json::to_vec(&bundle).map_err(|_| WorkflowStartError::InvalidInput)?,
        )
        .map_err(|_| WorkflowStartError::InvalidInput)?;
        let execution_id = ExecutionId::new();
        let mut state = ExecutionState::new(execution_id, workflow_id, &[]);
        let now = self.clock.now();
        state.created_at = now;
        state.updated_at = now;
        state.set_workflow_version_number(version.number);
        state.set_revision_ids(plan.id(), plan.worker_flavor_revision_id());
        state.set_workflow_input(input);
        state.set_budget(self.budget.clone());
        Ok(OriginalStart {
            scope: scope.clone(),
            workflow_id,
            workflow_key,
            execution_id,
            key: key.map_or(OriginalKey::Unkeyed, |key| {
                OriginalKey::Caller(key.key().to_owned(), key.fingerprint())
            }),
            initial_state: serde_json::to_value(state)
                .map_err(|_| WorkflowStartError::InvalidInput)?,
            command: ControlMsg {
                id: ulid::Ulid::new().to_bytes(),
                execution_id: execution_id.to_string(),
                command: ControlCommand::Start,
                scope: scope.clone(),
                w3c_traceparent: trace.map(|trace| trace.traceparent().to_owned()),
                reclaim_count: 0,
                resume_target: None,
            },
            bundle: record,
        })
    }

    async fn reconcile_unknown(
        &self,
        attempt: OriginalStart,
    ) -> Result<WorkflowStartReceipt, WorkflowStartError> {
        // Exactly one retry, with the same complete original envelope. The owner
        // resolves both possible outcomes safely: it accepts an attempt that did
        // not commit, or replays the immutable commitment of one that did.
        if let Ok(
            outcome @ (StartMaterialization::Accepted { .. }
            | StartMaterialization::Replayed { .. }),
        ) = self.starts.materialize_start(&attempt.envelope()).await
        {
            return self
                .materialized_receipt(&attempt.scope, attempt.workflow_id, outcome)
                .await;
        }
        Err(WorkflowStartError::MaterializationIndeterminate(Box::new(
            IndeterminateWorkflowStart { attempt },
        )))
    }

    async fn materialized_receipt(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        outcome: StartMaterialization,
    ) -> Result<WorkflowStartReceipt, WorkflowStartError> {
        match outcome {
            StartMaterialization::Accepted { execution_id } => {
                self.read_receipt(
                    scope,
                    workflow_id,
                    &execution_id,
                    WorkflowStartDisposition::Accepted,
                )
                .await
            },
            StartMaterialization::Replayed { execution_id } => {
                self.read_receipt(
                    scope,
                    workflow_id,
                    &execution_id,
                    WorkflowStartDisposition::Replayed,
                )
                .await
            },
            StartMaterialization::FingerprintMismatch => {
                Err(WorkflowStartError::FingerprintMismatch)
            },
            StartMaterialization::RevisionRejected(reason) => {
                Err(WorkflowStartError::RevisionNotAdmitted(reason))
            },
        }
    }

    async fn read_receipt(
        &self,
        scope: &Scope,
        workflow_id: WorkflowId,
        execution_key: &str,
        disposition: WorkflowStartDisposition,
    ) -> Result<WorkflowStartReceipt, WorkflowStartError> {
        let execution_id: ExecutionId = execution_key
            .parse()
            .map_err(|_| WorkflowStartError::InvalidReceipt)?;
        let unavailable = || WorkflowStartError::ReceiptUnavailable { execution_id };
        let row = self
            .executions
            .get(scope, execution_key)
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        let stored = self
            .starts
            .read_contract_bundle(scope, execution_key)
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        let mut state: ExecutionState =
            serde_json::from_slice(&serde_json::to_vec(&row.state).map_err(|_| unavailable())?)
                .map_err(|_| unavailable())?;
        // Option<Value> decoding collapses an explicit JSON null into None.
        // Preserve the persisted input, including null, in the receipt.
        state.workflow_input = row.state.get("workflow_input").cloned();
        let bundle = crate::recorded_contract::checked_bundle(scope, execution_key, &stored)
            .map_err(|_| unavailable())?;
        let identity = stored.record().identity();
        if row.scope != *scope
            || row.id != execution_key
            || row.workflow_id != workflow_id
            || state.execution_id != execution_id
            || state.workflow_id != workflow_id
            || state.executable_plan_revision_id != Some(identity.revisions().plan())
            || state.worker_flavor_revision_id != Some(identity.revisions().worker_flavor())
            || state
                .workflow_version_number
                .is_none_or(|number| number == 0)
            || state.budget.is_none()
        {
            return Err(unavailable());
        }
        Ok(WorkflowStartReceipt {
            state,
            bundle,
            disposition,
        })
    }
}

fn tenant_ids(scope: &Scope) -> Result<(OrgId, WorkspaceId), WorkflowStartError> {
    Ok((
        scope
            .org_id
            .parse()
            .map_err(|_| WorkflowStartError::InvalidScope)?,
        scope
            .workspace_id
            .parse()
            .map_err(|_| WorkflowStartError::InvalidScope)?,
    ))
}

fn caller_intent_fingerprint(
    workflow_id: WorkflowId,
    input: &mut Value,
) -> Result<StartFingerprint, WorkflowStartError> {
    sort_object_keys(input);
    let bytes =
        serde_json::to_vec(&(workflow_id, input)).map_err(|_| WorkflowStartError::InvalidInput)?;
    let mut digest = Sha256::new();
    digest.update(b"nebula.workflow-start.caller-intent.v2\0");
    digest.update(bytes);
    Ok(StartFingerprint::new(2, digest.finalize().into()))
}

fn sort_object_keys(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.sort_keys();
            for value in fields.values_mut() {
                sort_object_keys(value);
            }
        },
        Value::Array(values) => {
            for value in values {
                sort_object_keys(value);
            }
        },
        _ => {},
    }
}

impl WorkflowStartError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidScope => "WORKFLOW_START:INVALID_SCOPE",
            Self::InvalidKey => "WORKFLOW_START:INVALID_KEY",
            Self::InvalidInput => "WORKFLOW_START:INVALID_INPUT",
            Self::MissingWorkflow => "WORKFLOW_START:MISSING_WORKFLOW",
            Self::WorkflowNotActivated => "WORKFLOW_START:NOT_ACTIVATED",
            Self::InvalidActivation => "WORKFLOW_START:INVALID_ACTIVATION",
            Self::RevisionUnavailable(_) => "WORKFLOW_START:REVISION_UNAVAILABLE",
            Self::RevisionNotAdmitted(_) => "WORKFLOW_START:REVISION_NOT_ADMITTED",
            Self::UnsupportedBindings => "WORKFLOW_START:UNSUPPORTED_BINDINGS",
            Self::UnsupportedRecordedSemantics => "WORKFLOW_START:UNSUPPORTED_SEMANTICS",
            Self::FingerprintMismatch => "WORKFLOW_START:FINGERPRINT_MISMATCH",
            Self::ReceiptUnavailable { .. } => "WORKFLOW_START:RECEIPT_UNAVAILABLE",
            Self::InvalidReceipt => "WORKFLOW_START:INVALID_RECEIPT",
            Self::MaterializationIndeterminate(_) => "WORKFLOW_START:INDETERMINATE",
            Self::MaterializationRejected => "WORKFLOW_START:MATERIALIZATION_REJECTED",
            Self::BackendUnavailable => "WORKFLOW_START:BACKEND_UNAVAILABLE",
        }
    }
}

impl fmt::Debug for WorkflowStartService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowStartService")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
