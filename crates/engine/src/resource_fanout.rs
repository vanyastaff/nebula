//! Durable resource-event delivery into workflow starts.

use std::{fmt, sync::Arc, time::Duration};

use nebula_core::{NodeKey, WorkflowId};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{
        ClaimResourceRuntimeWorkRequest, ClaimedResourceDelivery, ClaimedResourceHandoff,
        CompleteResourceDeliveryRequest, HeartbeatResourceHandoffRequest,
        ReleaseResourceDeliveryRequest, ResourceConsumerIdentity, ResourceConsumerKind,
        ResourceDeliveryCompletion, ResourceHandoffClaimRequest, ResourceSubscriptionState,
        TerminalDeliveryIneligibility, TriggerStartKey,
    },
    store::{
        ResourceEventFanoutStore, ResourceExecutionHandoffStore, ResourceRuntimeRecovery,
        ResourceSubscriptionStore,
    },
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{WorkflowStartError, WorkflowStartService};

const WORKFLOW_TRIGGER_CONSUMER_KIND: &str = "workflow-trigger";
const WORKFLOW_TRIGGER_CODEC_MAGIC: &[u8; 4] = b"NWTC";
const WORKFLOW_TRIGGER_CODEC_VERSION: u8 = 1;
const WORKFLOW_TRIGGER_TARGET_TAG: u8 = 1;
const RESOURCE_EVENT_SCHEMA_VERSION: u32 = 1;

/// Exact workflow and trigger selected by a resource subscription.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkflowTriggerTarget {
    workflow_id: WorkflowId,
    trigger_id: NodeKey,
}

impl WorkflowTriggerTarget {
    /// Construct an exact workflow-trigger target.
    #[must_use]
    pub const fn new(workflow_id: WorkflowId, trigger_id: NodeKey) -> Self {
        Self {
            workflow_id,
            trigger_id,
        }
    }

    /// Return the exact workflow identifier.
    #[must_use]
    pub const fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }

    /// Return the exact trigger identity within the workflow.
    #[must_use]
    pub const fn trigger_id(&self) -> &NodeKey {
        &self.trigger_id
    }
}

impl fmt::Debug for WorkflowTriggerTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowTriggerTarget")
            .finish_non_exhaustive()
    }
}

/// Payload-free workflow-trigger consumer codec failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WorkflowTriggerConsumerCodecError {
    /// The subscription belongs to another consumer class.
    #[error("resource consumer kind is not a workflow trigger")]
    WrongKind,
    /// The identity is not framed as a workflow-trigger target.
    #[error("workflow trigger consumer identity is malformed")]
    Malformed,
    /// The framed codec version is not supported.
    #[error("workflow trigger consumer identity version is unsupported")]
    UnsupportedVersion,
    /// The framed record type is unknown.
    #[error("workflow trigger consumer identity record type is unknown")]
    UnknownRecordType,
    /// Bytes remain after the complete framed target.
    #[error("workflow trigger consumer identity has trailing data")]
    TrailingData,
    /// The bounded storage-port consumer value rejected the encoded target.
    #[error("workflow trigger consumer identity exceeds storage limits")]
    StorageValueRejected,
}

impl WorkflowTriggerConsumerCodecError {
    const fn code(self) -> &'static str {
        match self {
            Self::WrongKind => "RESOURCE_FANOUT:WRONG_CONSUMER_KIND",
            Self::Malformed => "RESOURCE_FANOUT:MALFORMED_CONSUMER_IDENTITY",
            Self::UnsupportedVersion => "RESOURCE_FANOUT:UNSUPPORTED_CONSUMER_VERSION",
            Self::UnknownRecordType => "RESOURCE_FANOUT:UNKNOWN_CONSUMER_RECORD",
            Self::TrailingData => "RESOURCE_FANOUT:TRAILING_CONSUMER_DATA",
            Self::StorageValueRejected => "RESOURCE_FANOUT:CONSUMER_VALUE_REJECTED",
        }
    }
}

/// Versioned codec for workflow-trigger resource subscription consumers.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkflowTriggerConsumerCodec;

impl WorkflowTriggerConsumerCodec {
    /// Encode an exact target into the existing storage-port kind and identity types.
    ///
    /// # Errors
    /// Returns a payload-free error if the bounded port values reject the encoded target.
    pub fn encode(
        target: &WorkflowTriggerTarget,
    ) -> Result<(ResourceConsumerKind, ResourceConsumerIdentity), WorkflowTriggerConsumerCodecError>
    {
        let workflow_id = target.workflow_id.to_string();
        let trigger_id = target.trigger_id.as_str().as_bytes();
        let workflow_len = u8::try_from(workflow_id.len())
            .map_err(|_| WorkflowTriggerConsumerCodecError::StorageValueRejected)?;
        let trigger_len = u16::try_from(trigger_id.len())
            .map_err(|_| WorkflowTriggerConsumerCodecError::StorageValueRejected)?;
        let mut identity = Vec::with_capacity(8 + workflow_id.len() + trigger_id.len());
        identity.extend_from_slice(WORKFLOW_TRIGGER_CODEC_MAGIC);
        identity.push(WORKFLOW_TRIGGER_CODEC_VERSION);
        identity.push(WORKFLOW_TRIGGER_TARGET_TAG);
        identity.push(workflow_len);
        identity.extend_from_slice(&trigger_len.to_be_bytes());
        identity.extend_from_slice(workflow_id.as_bytes());
        identity.extend_from_slice(trigger_id);

        let kind = ResourceConsumerKind::new(WORKFLOW_TRIGGER_CONSUMER_KIND)
            .map_err(|_| WorkflowTriggerConsumerCodecError::StorageValueRejected)?;
        let identity = ResourceConsumerIdentity::try_from_vec(identity)
            .map_err(|_| WorkflowTriggerConsumerCodecError::StorageValueRejected)?;
        Ok((kind, identity))
    }

    /// Decode an exact target, rejecting every unsupported or non-canonical frame.
    ///
    /// # Errors
    /// Returns a payload-free typed classification for wrong kind, malformed framing,
    /// unsupported versions or records, and trailing bytes.
    pub fn decode(
        kind: &ResourceConsumerKind,
        identity: &[u8],
    ) -> Result<WorkflowTriggerTarget, WorkflowTriggerConsumerCodecError> {
        if kind.as_str() != WORKFLOW_TRIGGER_CONSUMER_KIND {
            return Err(WorkflowTriggerConsumerCodecError::WrongKind);
        }
        if identity.len() < 9 || &identity[..4] != WORKFLOW_TRIGGER_CODEC_MAGIC {
            return Err(WorkflowTriggerConsumerCodecError::Malformed);
        }
        if identity[4] != WORKFLOW_TRIGGER_CODEC_VERSION {
            return Err(WorkflowTriggerConsumerCodecError::UnsupportedVersion);
        }
        if identity[5] != WORKFLOW_TRIGGER_TARGET_TAG {
            return Err(WorkflowTriggerConsumerCodecError::UnknownRecordType);
        }
        let workflow_len = usize::from(identity[6]);
        let trigger_len = usize::from(u16::from_be_bytes([identity[7], identity[8]]));
        let expected_len = 9usize
            .checked_add(workflow_len)
            .and_then(|len| len.checked_add(trigger_len))
            .ok_or(WorkflowTriggerConsumerCodecError::Malformed)?;
        if identity.len() < expected_len {
            return Err(WorkflowTriggerConsumerCodecError::Malformed);
        }
        if identity.len() > expected_len {
            return Err(WorkflowTriggerConsumerCodecError::TrailingData);
        }
        let workflow_end = 9 + workflow_len;
        let workflow_id = std::str::from_utf8(&identity[9..workflow_end])
            .map_err(|_| WorkflowTriggerConsumerCodecError::Malformed)?
            .parse()
            .map_err(|_| WorkflowTriggerConsumerCodecError::Malformed)?;
        let trigger_id = std::str::from_utf8(&identity[workflow_end..])
            .map_err(|_| WorkflowTriggerConsumerCodecError::Malformed)
            .and_then(|value| {
                NodeKey::new(value).map_err(|_| WorkflowTriggerConsumerCodecError::Malformed)
            })?;
        Ok(WorkflowTriggerTarget::new(workflow_id, trigger_id))
    }
}

/// Invalid coordinator supervision configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResourceFanoutCoordinatorBuildError {
    /// Polling must yield between empty drains.
    #[error("resource fanout poll interval must be positive")]
    ZeroPollInterval,
    /// Persistent infrastructure failures need a positive retry bound.
    #[error("resource fanout infrastructure failure bound must be positive")]
    ZeroFailureBound,
}

/// Closed, payload-free reason the durable resource coordinator could not proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceFanoutFailureKind {
    /// The storage backend could not be reached; a later attempt may succeed.
    Unavailable,
    /// A storage operation exceeded its deadline.
    Timeout,
    /// A submitted mutation may have committed without an acknowledgement.
    CommitUnknown,
    /// The exact item claim no longer grants authority to this coordinator.
    OwnershipLost,
    /// Persisted data cannot be decoded or violates its recorded schema.
    StoredDataInvalid,
    /// The configured backend cannot execute the requested work.
    Misconfigured,
    /// The coordinator issued a command the storage contract rejects.
    InvalidCommand,
    /// Durable ownership, storage or workflow-start invariants were violated.
    InvariantViolation,
}

impl ResourceFanoutFailureKind {
    /// Return a stable value-free label for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::CommitUnknown => "commit_unknown",
            Self::OwnershipLost => "ownership_lost",
            Self::StoredDataInvalid => "stored_data_invalid",
            Self::Misconfigured => "misconfigured",
            Self::InvalidCommand => "invalid_command",
            Self::InvariantViolation => "invariant_violation",
        }
    }

    const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::Unavailable | Self::Timeout | Self::CommitUnknown
        )
    }
}

/// Payload-free coordinator failure: permanent immediately, transient after bounded retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "resource fanout failed after {attempts} attempts; kind={failure_kind:?}; code={error_code}"
)]
pub struct ResourceFanoutCoordinatorError {
    attempts: u32,
    error_code: &'static str,
    failure_kind: ResourceFanoutFailureKind,
}

impl ResourceFanoutCoordinatorError {
    /// Return the number of consecutive failed drain attempts.
    #[must_use]
    pub const fn attempts(self) -> u32 {
        self.attempts
    }

    /// Return the stable payload-free failure code.
    #[must_use]
    pub const fn error_code(self) -> &'static str {
        self.error_code
    }

    /// Return the closed failure classification without retaining storage messages.
    #[must_use]
    pub const fn failure_kind(self) -> ResourceFanoutFailureKind {
        self.failure_kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessOutcome {
    Completed,
    OwnershipLost,
}

/// Work completed by one authoritative storage drain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceFanoutDrainOutcome {
    /// Deliveries terminalized during this drain.
    pub completed_deliveries: u32,
    /// Handoffs acknowledged during this drain.
    pub acknowledged_handoffs: u32,
}

/// Engine-owned durable resource fanout and workflow-start coordinator.
pub struct ResourceFanoutCoordinator {
    recovery: Arc<dyn ResourceRuntimeRecovery>,
    subscriptions: Arc<dyn ResourceSubscriptionStore>,
    fanout: Arc<dyn ResourceEventFanoutStore>,
    handoffs: Arc<dyn ResourceExecutionHandoffStore>,
    starts: Arc<WorkflowStartService>,
    claim: ClaimResourceRuntimeWorkRequest,
    poll_interval: Duration,
    max_consecutive_failures: u32,
}

impl ResourceFanoutCoordinator {
    /// Compose the durable roles used by one supervised resource runtime.
    ///
    /// # Errors
    /// Returns a typed error for a zero poll interval or failure bound.
    #[expect(
        clippy::too_many_arguments,
        reason = "each durable owner port is an explicit composition dependency"
    )]
    pub fn new(
        recovery: Arc<dyn ResourceRuntimeRecovery>,
        subscriptions: Arc<dyn ResourceSubscriptionStore>,
        fanout: Arc<dyn ResourceEventFanoutStore>,
        handoffs: Arc<dyn ResourceExecutionHandoffStore>,
        starts: Arc<WorkflowStartService>,
        claim: ClaimResourceRuntimeWorkRequest,
        poll_interval: Duration,
        max_consecutive_failures: u32,
    ) -> Result<Self, ResourceFanoutCoordinatorBuildError> {
        if poll_interval.is_zero() {
            return Err(ResourceFanoutCoordinatorBuildError::ZeroPollInterval);
        }
        if max_consecutive_failures == 0 {
            return Err(ResourceFanoutCoordinatorBuildError::ZeroFailureBound);
        }
        Ok(Self {
            recovery,
            subscriptions,
            fanout,
            handoffs,
            starts,
            claim,
            poll_interval,
            max_consecutive_failures,
        })
    }

    /// Drain one globally claimed delivery batch followed by one handoff batch.
    ///
    /// # Errors
    /// Permanent failures stop the drain without modifying the damaged item's claim.
    /// Retryable failures release only the exact item claim. A superseded claim contributes
    /// no completed work and grants no authority to release or acknowledge the item.
    #[tracing::instrument(skip_all, fields(error_code = tracing::field::Empty, failure_kind = tracing::field::Empty))]
    pub async fn drain_once(
        &self,
    ) -> Result<ResourceFanoutDrainOutcome, ResourceFanoutCoordinatorError> {
        let mut outcome = ResourceFanoutDrainOutcome::default();
        let mut item_failure = None;
        let deliveries = self
            .recovery
            .claim_deliveries_globally(self.claim.clone())
            .await
            .map_err(|error| storage_failure(error, "RESOURCE_FANOUT:CLAIM_DELIVERIES"))?;
        for scoped in deliveries {
            let (scope, delivery) = scoped.into_parts();
            match self.process_delivery(&scope, &delivery).await {
                Ok(ProcessOutcome::Completed) => {
                    outcome.completed_deliveries = outcome.completed_deliveries.saturating_add(1);
                },
                Ok(ProcessOutcome::OwnershipLost) => {},
                Err(error) if !error.failure_kind.is_retryable() => return Err(error),
                Err(error) => {
                    item_failure =
                        Some(item_failure.map_or(error, |primary| prefer_failure(primary, error)));
                },
            }
        }

        let handoffs = self
            .recovery
            .claim_handoffs_globally(self.claim.clone())
            .await
            .map_err(|error| {
                let error = storage_failure(error, "RESOURCE_FANOUT:CLAIM_HANDOFFS");
                item_failure.map_or(error, |primary| prefer_failure(primary, error))
            })?;
        for scoped in handoffs {
            let (scope, handoff) = scoped.into_parts();
            match self.process_handoff(&scope, &handoff).await {
                Ok(ProcessOutcome::Completed) => {
                    outcome.acknowledged_handoffs = outcome.acknowledged_handoffs.saturating_add(1);
                },
                Ok(ProcessOutcome::OwnershipLost) => {},
                Err(error) if !error.failure_kind.is_retryable() => return Err(error),
                Err(error) => {
                    item_failure =
                        Some(item_failure.map_or(error, |primary| prefer_failure(primary, error)));
                },
            }
        }
        item_failure.map_or(Ok(outcome), Err)
    }

    /// Poll durable work until cancellation, a permanent failure, or bounded transient failures.
    ///
    /// # Errors
    /// Permanent failures stop immediately. Retryable failures stop after the configured
    /// number of consecutive failed drains; empty successful drains reset that count.
    pub async fn run(
        &self,
        shutdown: CancellationToken,
    ) -> Result<(), ResourceFanoutCoordinatorError> {
        let mut consecutive_failures = 0u32;
        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            match self.drain_once().await {
                Ok(_) => consecutive_failures = 0,
                Err(error) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    tracing::warn!(
                        error_code = error.error_code(),
                        failure_kind = error.failure_kind().as_str(),
                        attempts = consecutive_failures,
                        "resource fanout drain failed"
                    );
                    if !error.failure_kind.is_retryable()
                        || consecutive_failures >= self.max_consecutive_failures
                    {
                        return Err(ResourceFanoutCoordinatorError {
                            attempts: consecutive_failures,
                            error_code: error.error_code,
                            failure_kind: error.failure_kind,
                        });
                    }
                },
            }
            tokio::select! {
                () = shutdown.cancelled() => return Ok(()),
                () = tokio::time::sleep(self.poll_interval) => {},
            }
        }
    }

    async fn process_delivery(
        &self,
        scope: &Scope,
        delivery: &ClaimedResourceDelivery,
    ) -> Result<ProcessOutcome, ResourceFanoutCoordinatorError> {
        let subscription = match self
            .subscriptions
            .get(scope, delivery.subscription_id())
            .await
        {
            Ok(subscription) => subscription,
            Err(error) => {
                return self
                    .fail_delivery(
                        scope,
                        delivery,
                        storage_failure(error, "RESOURCE_FANOUT:LOAD_SUBSCRIPTION"),
                    )
                    .await;
            },
        };
        let completion = match subscription {
            None => ResourceDeliveryCompletion::Ineligible(
                TerminalDeliveryIneligibility::ConsumerUnavailable,
            ),
            Some(subscription) => match subscription.state() {
                ResourceSubscriptionState::Disabled => ResourceDeliveryCompletion::Ineligible(
                    TerminalDeliveryIneligibility::SubscriptionDisabled,
                ),
                ResourceSubscriptionState::Tombstoned => ResourceDeliveryCompletion::Ineligible(
                    TerminalDeliveryIneligibility::SubscriptionTombstoned,
                ),
                ResourceSubscriptionState::Active => {
                    if let Err(error) = WorkflowTriggerConsumerCodec::decode(
                        subscription.consumer_kind(),
                        subscription.consumer_identity_bytes(),
                    ) {
                        tracing::warn!(
                            delivery_id = ?delivery.id(),
                            error_code = error.code(),
                            "resource delivery target is permanently ineligible"
                        );
                        ResourceDeliveryCompletion::Ineligible(
                            TerminalDeliveryIneligibility::ConsumerUnavailable,
                        )
                    } else if delivery.envelope().schema_version() != RESOURCE_EVENT_SCHEMA_VERSION
                        || serde_json::from_slice::<Value>(delivery.envelope().canonical_payload())
                            .is_err()
                    {
                        ResourceDeliveryCompletion::Ineligible(
                            TerminalDeliveryIneligibility::UnsupportedEnvelopeSchema,
                        )
                    } else {
                        ResourceDeliveryCompletion::Delivered
                    }
                },
            },
        };
        if let Err(error) = self
            .fanout
            .complete_delivery(CompleteResourceDeliveryRequest::new(
                scope.clone(),
                delivery.id(),
                delivery.token().clone(),
                completion,
            ))
            .await
        {
            return self
                .fail_delivery(
                    scope,
                    delivery,
                    exact_claim_failure(error, "RESOURCE_FANOUT:COMPLETE_DELIVERY"),
                )
                .await;
        }
        Ok(ProcessOutcome::Completed)
    }

    async fn fail_delivery(
        &self,
        scope: &Scope,
        delivery: &ClaimedResourceDelivery,
        primary: ResourceFanoutCoordinatorError,
    ) -> Result<ProcessOutcome, ResourceFanoutCoordinatorError> {
        if primary.failure_kind == ResourceFanoutFailureKind::OwnershipLost {
            return Ok(ProcessOutcome::OwnershipLost);
        }
        if !primary.failure_kind.is_retryable() {
            return Err(primary);
        }
        let cleanup = self
            .fanout
            .release_delivery(ReleaseResourceDeliveryRequest::new(
                scope.clone(),
                delivery.id(),
                delivery.token().clone(),
            ))
            .await;
        Err(after_cleanup(
            primary,
            cleanup,
            "RESOURCE_FANOUT:RELEASE_DELIVERY",
        ))
    }

    async fn process_handoff(
        &self,
        scope: &Scope,
        handoff: &ClaimedResourceHandoff,
    ) -> Result<ProcessOutcome, ResourceFanoutCoordinatorError> {
        let claim = ResourceHandoffClaimRequest::new(
            scope.clone(),
            handoff.delivery_id(),
            handoff.token().clone(),
        );
        let subscription = match self
            .subscriptions
            .get(scope, handoff.subscription_id())
            .await
        {
            Ok(Some(subscription)) => subscription,
            Ok(None) => {
                return self
                    .acknowledge_handoff(claim, Some("RESOURCE_FANOUT:MISSING_HANDOFF_TARGET"))
                    .await;
            },
            Err(error) => {
                return self
                    .fail_handoff(
                        claim,
                        storage_failure(error, "RESOURCE_FANOUT:LOAD_HANDOFF_TARGET"),
                    )
                    .await;
            },
        };
        let target = match WorkflowTriggerConsumerCodec::decode(
            subscription.consumer_kind(),
            subscription.consumer_identity_bytes(),
        ) {
            Ok(target) => target,
            Err(error) => {
                return self.acknowledge_handoff(claim, Some(error.code())).await;
            },
        };
        let input = match serde_json::from_slice::<Value>(handoff.envelope().canonical_payload()) {
            Ok(input) if handoff.envelope().schema_version() == RESOURCE_EVENT_SCHEMA_VERSION => {
                input
            },
            _ => {
                return self
                    .acknowledge_handoff(claim, Some("RESOURCE_FANOUT:INVALID_HANDOFF_ENVELOPE"))
                    .await;
            },
        };
        let trigger_key = format!(
            "resource:{}:{}",
            target.workflow_id(),
            target.trigger_id().as_str()
        );
        let event_key = uuid::Uuid::from_bytes(handoff.delivery_id().into_bytes())
            .simple()
            .to_string();
        if let Err(error) = self
            .handoffs
            .heartbeat_handoff(HeartbeatResourceHandoffRequest::new(
                claim.clone(),
                self.claim.ttl(),
            ))
            .await
        {
            return self
                .fail_handoff(
                    claim,
                    exact_claim_failure(error, "RESOURCE_FANOUT:HEARTBEAT_HANDOFF"),
                )
                .await;
        }
        match self
            .starts
            .start_trigger(
                scope,
                target.workflow_id(),
                input,
                TriggerStartKey::new(&trigger_key, &event_key),
                None,
            )
            .await
        {
            Ok(_) => self.acknowledge_handoff(claim, None).await,
            Err(error) => {
                let error_code = error.code();
                match classify_workflow_start_error(&error) {
                    WorkflowStartFailureClass::Terminal => {
                        self.acknowledge_handoff(claim, Some(error_code)).await
                    },
                    WorkflowStartFailureClass::Retry => {
                        self.fail_handoff(claim, workflow_start_retry_failure(&error))
                            .await
                    },
                    WorkflowStartFailureClass::Invariant => Err(coordinator_failure(
                        error_code,
                        ResourceFanoutFailureKind::InvariantViolation,
                    )),
                }
            },
        }
    }

    async fn fail_handoff(
        &self,
        claim: ResourceHandoffClaimRequest,
        primary: ResourceFanoutCoordinatorError,
    ) -> Result<ProcessOutcome, ResourceFanoutCoordinatorError> {
        if primary.failure_kind == ResourceFanoutFailureKind::OwnershipLost {
            return Ok(ProcessOutcome::OwnershipLost);
        }
        if !primary.failure_kind.is_retryable() {
            return Err(primary);
        }
        let cleanup = self.handoffs.release_handoff(claim).await;
        Err(after_cleanup(
            primary,
            cleanup,
            "RESOURCE_FANOUT:RELEASE_HANDOFF",
        ))
    }

    async fn acknowledge_handoff(
        &self,
        claim: ResourceHandoffClaimRequest,
        terminal_error_code: Option<&'static str>,
    ) -> Result<ProcessOutcome, ResourceFanoutCoordinatorError> {
        if let Some(error_code) = terminal_error_code {
            tracing::warn!(
                delivery_id = ?claim.delivery_id(),
                error_code,
                "resource handoff reached a permanent terminal outcome"
            );
        }
        if let Err(error) = self.handoffs.acknowledge_handoff(claim.clone()).await {
            return self
                .fail_handoff(
                    claim,
                    exact_claim_failure(error, "RESOURCE_FANOUT:ACK_HANDOFF"),
                )
                .await;
        }
        Ok(ProcessOutcome::Completed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowStartFailureClass {
    Retry,
    Terminal,
    Invariant,
}

fn classify_workflow_start_error(error: &WorkflowStartError) -> WorkflowStartFailureClass {
    match error {
        WorkflowStartError::RevisionUnavailable(source)
            if source.is_transient_catalog_failure() =>
        {
            WorkflowStartFailureClass::Retry
        },
        WorkflowStartError::ReceiptUnavailable { .. }
        | WorkflowStartError::MaterializationIndeterminate(_)
        | WorkflowStartError::BackendUnavailable => WorkflowStartFailureClass::Retry,
        WorkflowStartError::InvalidScope
        | WorkflowStartError::InvalidInput
        | WorkflowStartError::InvalidActivation
        | WorkflowStartError::RevisionUnavailable(_)
        | WorkflowStartError::FingerprintMismatch
        | WorkflowStartError::InvalidReceipt => WorkflowStartFailureClass::Invariant,
        WorkflowStartError::InvalidKey
        | WorkflowStartError::MissingWorkflow
        | WorkflowStartError::WorkflowNotActivated
        | WorkflowStartError::RevisionNotAdmitted(_)
        | WorkflowStartError::UnsupportedBindings
        | WorkflowStartError::BindingResolution(_)
        | WorkflowStartError::UnsupportedRecordedSemantics
        | WorkflowStartError::MaterializationRejected => WorkflowStartFailureClass::Terminal,
    }
}

impl fmt::Debug for ResourceFanoutCoordinator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceFanoutCoordinator")
            .field("poll_interval", &self.poll_interval)
            .field("max_consecutive_failures", &self.max_consecutive_failures)
            .finish_non_exhaustive()
    }
}

fn coordinator_failure(
    error_code: &'static str,
    failure_kind: ResourceFanoutFailureKind,
) -> ResourceFanoutCoordinatorError {
    tracing::Span::current().record("error_code", error_code);
    tracing::Span::current().record("failure_kind", failure_kind.as_str());
    tracing::warn!(
        error_code,
        failure_kind = failure_kind.as_str(),
        "resource fanout operation failed"
    );
    ResourceFanoutCoordinatorError {
        attempts: 1,
        error_code,
        failure_kind,
    }
}

fn storage_failure(
    error: StorageError,
    error_code: &'static str,
) -> ResourceFanoutCoordinatorError {
    let kind = match error {
        StorageError::Connection(_) => ResourceFanoutFailureKind::Unavailable,
        StorageError::Timeout { .. } => ResourceFanoutFailureKind::Timeout,
        StorageError::AcknowledgementUnknown { .. } => ResourceFanoutFailureKind::CommitUnknown,
        StorageError::Corrupt(_)
        | StorageError::UnknownSchemaVersion { .. }
        | StorageError::Serialization(_) => ResourceFanoutFailureKind::StoredDataInvalid,
        StorageError::Configuration(_) => ResourceFanoutFailureKind::Misconfigured,
        StorageError::InvalidInput(_) => ResourceFanoutFailureKind::InvalidCommand,
        // A lookup or global claim has no exact item token to supersede. These
        // failures contradict its contract rather than proving ownership loss.
        _ => ResourceFanoutFailureKind::InvariantViolation,
    };
    coordinator_failure(error_code, kind)
}

fn exact_claim_failure(
    error: StorageError,
    error_code: &'static str,
) -> ResourceFanoutCoordinatorError {
    if matches!(
        error,
        StorageError::FencedOut { .. } | StorageError::NotFound { .. }
    ) {
        coordinator_failure(error_code, ResourceFanoutFailureKind::OwnershipLost)
    } else {
        storage_failure(error, error_code)
    }
}

/// Cleanup cannot hide an uncertain primary commit or turn a permanent failure
/// into a retry. Lost cleanup authority leaves the original failure observable.
fn after_cleanup(
    primary: ResourceFanoutCoordinatorError,
    cleanup: Result<(), StorageError>,
    error_code: &'static str,
) -> ResourceFanoutCoordinatorError {
    let Err(error) = cleanup else {
        return primary;
    };
    let cleanup = exact_claim_failure(error, error_code);
    if cleanup.failure_kind == ResourceFanoutFailureKind::OwnershipLost {
        return primary;
    }
    prefer_failure(primary, cleanup)
}

/// Keep a permanent failure or uncertain commit ahead of a later connectivity
/// failure. Among equally actionable failures, retain the first failed stage.
fn prefer_failure(
    primary: ResourceFanoutCoordinatorError,
    later: ResourceFanoutCoordinatorError,
) -> ResourceFanoutCoordinatorError {
    if !primary.failure_kind.is_retryable() {
        primary
    } else if !later.failure_kind.is_retryable()
        || (primary.failure_kind != ResourceFanoutFailureKind::CommitUnknown
            && later.failure_kind == ResourceFanoutFailureKind::CommitUnknown)
    {
        later
    } else {
        primary
    }
}

fn workflow_start_retry_failure(error: &WorkflowStartError) -> ResourceFanoutCoordinatorError {
    let kind = match error {
        WorkflowStartError::ReceiptUnavailable { .. }
        | WorkflowStartError::MaterializationIndeterminate(_) => {
            ResourceFanoutFailureKind::CommitUnknown
        },
        WorkflowStartError::RevisionUnavailable(source)
            if matches!(
                source.as_ref(),
                crate::PlanFlavorRevisionBridgeError::Catalog {
                    source: nebula_storage_port::dto::RevisionCatalogError::OutcomeUnknown
                }
            ) =>
        {
            ResourceFanoutFailureKind::CommitUnknown
        },
        _ => ResourceFanoutFailureKind::Unavailable,
    };
    coordinator_failure(error.code(), kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_storage_port::dto::RevisionCatalogError;

    #[test]
    fn storage_failure_classes_are_closed_and_value_free() {
        let cases = [
            (
                StorageError::Connection("private-backend".into()),
                ResourceFanoutFailureKind::Unavailable,
            ),
            (
                StorageError::Timeout {
                    operation: "private-operation".into(),
                    duration: Duration::ZERO,
                },
                ResourceFanoutFailureKind::Timeout,
            ),
            (
                StorageError::AcknowledgementUnknown {
                    operation: "private-operation",
                },
                ResourceFanoutFailureKind::CommitUnknown,
            ),
            (
                StorageError::Corrupt("private-row".into()),
                ResourceFanoutFailureKind::StoredDataInvalid,
            ),
            (
                StorageError::UnknownSchemaVersion { found: 9, max: 1 },
                ResourceFanoutFailureKind::StoredDataInvalid,
            ),
            (
                StorageError::Serialization("private-row".into()),
                ResourceFanoutFailureKind::StoredDataInvalid,
            ),
            (
                StorageError::Configuration("private-config".into()),
                ResourceFanoutFailureKind::Misconfigured,
            ),
            (
                StorageError::InvalidInput("private-command".into()),
                ResourceFanoutFailureKind::InvalidCommand,
            ),
            (
                StorageError::Internal("private-invariant".into()),
                ResourceFanoutFailureKind::InvariantViolation,
            ),
            (
                StorageError::ScopeViolation {
                    entity: "private-entity",
                },
                ResourceFanoutFailureKind::InvariantViolation,
            ),
            (
                StorageError::Conflict {
                    entity: "private-entity",
                    id: "private-id".into(),
                    expected: 1,
                    actual: 2,
                },
                ResourceFanoutFailureKind::InvariantViolation,
            ),
            (
                StorageError::Duplicate {
                    entity: "private-entity",
                    detail: "private-detail".into(),
                },
                ResourceFanoutFailureKind::InvariantViolation,
            ),
            (
                StorageError::not_found("private-entity", "private-id"),
                ResourceFanoutFailureKind::InvariantViolation,
            ),
            (
                StorageError::FencedOut {
                    entity: "private-entity",
                    id: "private-id".into(),
                },
                ResourceFanoutFailureKind::InvariantViolation,
            ),
        ];
        for (source, kind) in cases {
            let failure = storage_failure(source, "RESOURCE_FANOUT:TEST");
            assert_eq!(failure.failure_kind(), kind);
            assert!(!format!("{failure:?} {failure}").contains("private"));
        }
    }

    #[test]
    fn ownership_loss_is_meaningful_only_at_the_exact_claim_seam() {
        for source in [
            StorageError::FencedOut {
                entity: "delivery",
                id: "opaque".into(),
            },
            StorageError::not_found("delivery", "opaque"),
        ] {
            assert_eq!(
                exact_claim_failure(source, "RESOURCE_FANOUT:TEST").failure_kind(),
                ResourceFanoutFailureKind::OwnershipLost
            );
        }
        assert!(!ResourceFanoutFailureKind::OwnershipLost.is_retryable());
    }

    #[test]
    fn cleanup_keeps_primary_commit_uncertainty_unless_it_finds_a_permanent_failure() {
        let primary = coordinator_failure(
            "RESOURCE_FANOUT:COMPLETE_DELIVERY",
            ResourceFanoutFailureKind::CommitUnknown,
        );
        for cleanup in [
            StorageError::Connection("private-backend".into()),
            StorageError::Timeout {
                operation: "private-operation".into(),
                duration: Duration::ZERO,
            },
            StorageError::FencedOut {
                entity: "delivery",
                id: "private-id".into(),
            },
        ] {
            assert_eq!(
                after_cleanup(primary, Err(cleanup), "RESOURCE_FANOUT:RELEASE_DELIVERY"),
                primary
            );
        }
        assert_eq!(
            after_cleanup(
                primary,
                Err(StorageError::Corrupt("private-row".into())),
                "RESOURCE_FANOUT:RELEASE_DELIVERY"
            )
            .failure_kind(),
            ResourceFanoutFailureKind::StoredDataInvalid
        );
    }

    #[test]
    fn a_later_global_claim_failure_cannot_hide_a_stronger_item_failure() {
        let unknown = coordinator_failure(
            "RESOURCE_FANOUT:COMPLETE_DELIVERY",
            ResourceFanoutFailureKind::CommitUnknown,
        );
        let unavailable = coordinator_failure(
            "RESOURCE_FANOUT:CLAIM_HANDOFFS",
            ResourceFanoutFailureKind::Unavailable,
        );
        let corrupt = coordinator_failure(
            "RESOURCE_FANOUT:CLAIM_HANDOFFS",
            ResourceFanoutFailureKind::StoredDataInvalid,
        );
        assert_eq!(prefer_failure(unknown, unavailable), unknown);
        assert_eq!(prefer_failure(unavailable, unknown), unknown);
        assert_eq!(prefer_failure(unknown, corrupt), corrupt);
        assert_eq!(prefer_failure(corrupt, unavailable), corrupt);
    }

    fn target() -> WorkflowTriggerTarget {
        WorkflowTriggerTarget::new(
            WorkflowId::new(),
            NodeKey::new("resource_event").expect("fixture trigger key is valid"),
        )
    }

    #[test]
    fn codec_round_trips_exact_target_without_debug_payload() {
        let target = target();
        let (kind, identity) = WorkflowTriggerConsumerCodec::encode(&target).expect("encodes");
        let decoded =
            WorkflowTriggerConsumerCodec::decode(&kind, identity.as_bytes()).expect("decodes");

        assert_eq!(decoded, target);
        let debug = format!("{decoded:?} {identity:?}");
        assert!(!debug.contains("resource_event"));
        assert!(!debug.contains(&target.workflow_id().to_string()));
    }

    #[test]
    fn codec_rejects_wrong_kind_version_record_malformed_and_trailing_data() {
        let target = target();
        let (kind, identity) = WorkflowTriggerConsumerCodec::encode(&target).expect("encodes");
        let wrong_kind = ResourceConsumerKind::new("other").expect("valid kind");
        assert_eq!(
            WorkflowTriggerConsumerCodec::decode(&wrong_kind, identity.as_bytes()),
            Err(WorkflowTriggerConsumerCodecError::WrongKind)
        );

        let mut unsupported = identity.as_bytes().to_vec();
        unsupported[4] = 2;
        assert_eq!(
            WorkflowTriggerConsumerCodec::decode(&kind, &unsupported),
            Err(WorkflowTriggerConsumerCodecError::UnsupportedVersion)
        );
        let mut unknown = identity.as_bytes().to_vec();
        unknown[5] = 9;
        assert_eq!(
            WorkflowTriggerConsumerCodec::decode(&kind, &unknown),
            Err(WorkflowTriggerConsumerCodecError::UnknownRecordType)
        );
        assert_eq!(
            WorkflowTriggerConsumerCodec::decode(&kind, &identity.as_bytes()[..8]),
            Err(WorkflowTriggerConsumerCodecError::Malformed)
        );
        let mut trailing = identity.as_bytes().to_vec();
        trailing.push(0);
        assert_eq!(
            WorkflowTriggerConsumerCodec::decode(&kind, &trailing),
            Err(WorkflowTriggerConsumerCodecError::TrailingData)
        );
    }

    #[test]
    fn codec_error_debug_never_contains_rejected_identity() {
        let secret = b"private-trigger-payload";
        let kind = ResourceConsumerKind::new(WORKFLOW_TRIGGER_CONSUMER_KIND).expect("valid kind");
        let error = WorkflowTriggerConsumerCodec::decode(&kind, secret).expect_err("rejects");
        assert!(!format!("{error:?}").contains("private-trigger-payload"));
    }

    #[test]
    fn workflow_start_transient_failures_are_retried() {
        for source in [
            RevisionCatalogError::Unavailable,
            RevisionCatalogError::OutcomeUnknown,
        ] {
            let error = WorkflowStartError::RevisionUnavailable(Box::new(
                crate::PlanFlavorRevisionBridgeError::Catalog { source },
            ));
            assert_eq!(
                classify_workflow_start_error(&error),
                WorkflowStartFailureClass::Retry
            );
        }

        for error in [
            WorkflowStartError::ReceiptUnavailable {
                execution_id: nebula_core::ExecutionId::new(),
            },
            WorkflowStartError::BackendUnavailable,
        ] {
            assert_eq!(
                classify_workflow_start_error(&error),
                WorkflowStartFailureClass::Retry
            );
        }
    }

    #[test]
    fn workflow_start_integrity_contradictions_fail_closed_for_retry() {
        for error in [
            WorkflowStartError::FingerprintMismatch,
            WorkflowStartError::InvalidActivation,
            WorkflowStartError::InvalidReceipt,
        ] {
            assert_eq!(
                classify_workflow_start_error(&error),
                WorkflowStartFailureClass::Invariant
            );
        }
    }

    #[test]
    fn only_deterministic_workflow_start_rejections_are_terminal() {
        for error in [
            WorkflowStartError::InvalidKey,
            WorkflowStartError::MissingWorkflow,
            WorkflowStartError::WorkflowNotActivated,
            WorkflowStartError::UnsupportedBindings,
            WorkflowStartError::UnsupportedRecordedSemantics,
            WorkflowStartError::MaterializationRejected,
        ] {
            assert_eq!(
                classify_workflow_start_error(&error),
                WorkflowStartFailureClass::Terminal
            );
        }
    }
}
