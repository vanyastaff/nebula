//! Durable resource-event delivery into workflow starts.

use std::{fmt, sync::Arc, time::Duration};

use nebula_core::{NodeKey, WorkflowId};
use nebula_storage_port::{
    Scope,
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

/// Payload-free terminal coordinator failure after bounded retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "resource fanout infrastructure remained unavailable after {attempts} attempts; code={error_code}"
)]
pub struct ResourceFanoutCoordinatorError {
    attempts: u32,
    error_code: &'static str,
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
    /// Returns a payload-free infrastructure error after releasing any exact item claim
    /// whose processing can be retried safely.
    #[tracing::instrument(skip_all, fields(error_code = tracing::field::Empty))]
    pub async fn drain_once(
        &self,
    ) -> Result<ResourceFanoutDrainOutcome, ResourceFanoutCoordinatorError> {
        let mut outcome = ResourceFanoutDrainOutcome::default();
        let mut first_item_error = None;
        let deliveries = self
            .recovery
            .claim_deliveries_globally(self.claim.clone())
            .await
            .map_err(|_| infrastructure_error("RESOURCE_FANOUT:CLAIM_DELIVERIES"))?;
        for scoped in deliveries {
            let (scope, delivery) = scoped.into_parts();
            match self.process_delivery(&scope, &delivery).await {
                Ok(()) => {
                    outcome.completed_deliveries = outcome.completed_deliveries.saturating_add(1);
                },
                Err(error) => {
                    first_item_error.get_or_insert(error);
                },
            }
        }

        let handoffs = self
            .recovery
            .claim_handoffs_globally(self.claim.clone())
            .await
            .map_err(|_| infrastructure_error("RESOURCE_FANOUT:CLAIM_HANDOFFS"))?;
        for scoped in handoffs {
            let (scope, handoff) = scoped.into_parts();
            match self.process_handoff(&scope, &handoff).await {
                Ok(()) => {
                    outcome.acknowledged_handoffs = outcome.acknowledged_handoffs.saturating_add(1);
                },
                Err(error) => {
                    first_item_error.get_or_insert(error);
                },
            }
        }
        first_item_error.map_or(Ok(outcome), Err)
    }

    /// Poll durable work until cancellation or a bounded persistent infrastructure failure.
    ///
    /// # Errors
    /// Returns the last payload-free infrastructure classification after the configured
    /// number of consecutive failed drains.
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
                        attempts = consecutive_failures,
                        "resource fanout drain failed"
                    );
                    if consecutive_failures >= self.max_consecutive_failures {
                        return Err(ResourceFanoutCoordinatorError {
                            attempts: consecutive_failures,
                            error_code: error.error_code,
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
    ) -> Result<(), ResourceFanoutCoordinatorError> {
        let subscription = match self
            .subscriptions
            .get(scope, delivery.subscription_id())
            .await
        {
            Ok(subscription) => subscription,
            Err(_) => {
                return self
                    .release_delivery(scope, delivery, "RESOURCE_FANOUT:LOAD_SUBSCRIPTION")
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
        if self
            .fanout
            .complete_delivery(CompleteResourceDeliveryRequest::new(
                scope.clone(),
                delivery.id(),
                delivery.token().clone(),
                completion,
            ))
            .await
            .is_err()
        {
            return self
                .release_delivery(scope, delivery, "RESOURCE_FANOUT:COMPLETE_DELIVERY")
                .await;
        }
        Ok(())
    }

    async fn release_delivery(
        &self,
        scope: &Scope,
        delivery: &ClaimedResourceDelivery,
        error_code: &'static str,
    ) -> Result<(), ResourceFanoutCoordinatorError> {
        self.fanout
            .release_delivery(ReleaseResourceDeliveryRequest::new(
                scope.clone(),
                delivery.id(),
                delivery.token().clone(),
            ))
            .await
            .map_err(|_| infrastructure_error("RESOURCE_FANOUT:RELEASE_DELIVERY"))?;
        Err(infrastructure_error(error_code))
    }

    async fn process_handoff(
        &self,
        scope: &Scope,
        handoff: &ClaimedResourceHandoff,
    ) -> Result<(), ResourceFanoutCoordinatorError> {
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
                self.acknowledge_handoff(claim, Some("RESOURCE_FANOUT:MISSING_HANDOFF_TARGET"))
                    .await?;
                return Ok(());
            },
            Err(_) => {
                return self
                    .release_handoff(claim, "RESOURCE_FANOUT:LOAD_HANDOFF_TARGET")
                    .await;
            },
        };
        let target = match WorkflowTriggerConsumerCodec::decode(
            subscription.consumer_kind(),
            subscription.consumer_identity_bytes(),
        ) {
            Ok(target) => target,
            Err(error) => {
                self.acknowledge_handoff(claim, Some(error.code())).await?;
                return Ok(());
            },
        };
        let input = match serde_json::from_slice::<Value>(handoff.envelope().canonical_payload()) {
            Ok(input) if handoff.envelope().schema_version() == RESOURCE_EVENT_SCHEMA_VERSION => {
                input
            },
            _ => {
                self.acknowledge_handoff(claim, Some("RESOURCE_FANOUT:INVALID_HANDOFF_ENVELOPE"))
                    .await?;
                return Ok(());
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
        if self
            .handoffs
            .heartbeat_handoff(HeartbeatResourceHandoffRequest::new(
                claim.clone(),
                self.claim.ttl(),
            ))
            .await
            .is_err()
        {
            return self
                .release_handoff(claim, "RESOURCE_FANOUT:HEARTBEAT_HANDOFF")
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
            Ok(_) => {
                self.acknowledge_handoff(claim, None).await?;
                Ok(())
            },
            Err(error) => {
                let error_code = error.code();
                match classify_workflow_start_error(&error) {
                    WorkflowStartFailureClass::Terminal => {
                        self.acknowledge_handoff(claim, Some(error_code)).await?;
                        Ok(())
                    },
                    WorkflowStartFailureClass::Retry => {
                        self.release_handoff(claim, error_code).await
                    },
                    WorkflowStartFailureClass::Invariant => {
                        tracing::error!(
                            delivery_id = ?handoff.delivery_id(),
                            error_code,
                            "resource handoff workflow start violated an integrity invariant"
                        );
                        self.release_handoff(claim, error_code).await
                    },
                }
            },
        }
    }

    async fn release_handoff(
        &self,
        claim: ResourceHandoffClaimRequest,
        error_code: &'static str,
    ) -> Result<(), ResourceFanoutCoordinatorError> {
        self.handoffs
            .release_handoff(claim)
            .await
            .map_err(|_| infrastructure_error("RESOURCE_FANOUT:RELEASE_HANDOFF"))?;
        Err(infrastructure_error(error_code))
    }

    async fn acknowledge_handoff(
        &self,
        claim: ResourceHandoffClaimRequest,
        terminal_error_code: Option<&'static str>,
    ) -> Result<(), ResourceFanoutCoordinatorError> {
        if let Some(error_code) = terminal_error_code {
            tracing::warn!(
                delivery_id = ?claim.delivery_id(),
                error_code,
                "resource handoff reached a permanent terminal outcome"
            );
        }
        let retry_claim = claim.clone();
        if self.handoffs.acknowledge_handoff(claim).await.is_err() {
            self.handoffs
                .release_handoff(retry_claim)
                .await
                .map_err(|_| infrastructure_error("RESOURCE_FANOUT:RELEASE_HANDOFF"))?;
            return Err(infrastructure_error("RESOURCE_FANOUT:ACK_HANDOFF"));
        }
        Ok(())
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

fn infrastructure_error(error_code: &'static str) -> ResourceFanoutCoordinatorError {
    tracing::Span::current().record("error_code", error_code);
    ResourceFanoutCoordinatorError {
        attempts: 1,
        error_code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_storage_port::dto::RevisionCatalogError;

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
