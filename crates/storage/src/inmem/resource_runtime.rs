//! In-memory reference adapter for shared resources and durable event fanout.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use nebula_core::accessor::Clock;
use nebula_storage_port::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, AcknowledgeResourceHandoffOutcome,
    AcquireResourceSourceLeaseOutcome, AcquireResourceSourceLeaseRequest,
    ClaimResourceDeliveriesRequest, ClaimResourceHandoffsRequest, ClaimResourceRuntimeWorkRequest,
    ClaimedResourceDelivery, ClaimedResourceHandoff, CompleteResourceDeliveryOutcome,
    CompleteResourceDeliveryRequest, EventEnvelope, EventOccurrenceKey, EventOccurrenceNamespace,
    HeartbeatResourceDeliveryRequest, HeartbeatResourceHandoffRequest,
    HeartbeatResourceSourceLeaseRequest, PutResourceSubscriptionOutcome,
    PutResourceSubscriptionRequest, ReconciliationCursor, ReleaseResourceDeliveryRequest,
    ReleaseResourceSourceLeaseRequest, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
    ResourceConsumerIdentity, ResourceConsumerKind, ResourceDeliveryClaimToken,
    ResourceDeliveryCompletion, ResourceDeliveryId, ResourceEventAcceptance, ResourceEventId,
    ResourceEventRecord, ResourceEventState, ResourceHandoffClaimRequest,
    ResourceHandoffClaimToken, ResourceLeaseGeneration, ResourceLeaseHolder, ResourceLeaseTtl,
    ResourcePageSize, ResourceSourceLease, ResourceSourceLeaseToken, ResourceSubscriptionId,
    ResourceSubscriptionPage, ResourceSubscriptionRecord, ResourceSubscriptionState,
    ResourceSubscriptionVersion, ScopedClaimedResourceDelivery, ScopedClaimedResourceHandoff,
    SharedResourceId, SharedResourceIdentity, SharedResourcePage, SharedResourceRecord,
    TerminalDeliveryIneligibility, TransitionResourceSubscriptionRequest,
};
use nebula_storage_port::store::{
    ResourceEventFanoutStore, ResourceExecutionHandoffStore, ResourceRuntimeRecovery,
    ResourceSourceLeaseStore, ResourceSubscriptionStore, SharedResourceStore,
};
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;
use uuid::Uuid;

type ScopedResourceKey = (Scope, SharedResourceId);
type ScopedSubscriptionKey = (Scope, ResourceSubscriptionId);
type ScopedEventKey = (Scope, ResourceEventId);
type ScopedDeliveryKey = (Scope, ResourceDeliveryId);
type ScopedHandoffKey = (Scope, ResourceDeliveryId);

#[derive(Default)]
struct State {
    force_identity_digest_collision: bool,
    next_reconciliation_sequence: u64,
    next_subscription_sequence: u64,
    next_delivery_sequence: u64,
    next_handoff_sequence: u64,
    resources: HashMap<ScopedResourceKey, SharedResourceRecord>,
    resource_identity_buckets: HashMap<(Scope, [u8; 32]), Vec<SharedResourceId>>,
    subscriptions: HashMap<ScopedSubscriptionKey, SubscriptionRow>,
    subscription_identities: HashMap<
        (
            Scope,
            SharedResourceId,
            ResourceConsumerKind,
            ResourceConsumerIdentity,
        ),
        ResourceSubscriptionId,
    >,
    source_leases: HashMap<ScopedResourceKey, SourceLeaseRow>,
    events: HashMap<ScopedEventKey, EventRow>,
    event_occurrences: HashMap<
        (
            Scope,
            SharedResourceId,
            EventOccurrenceNamespace,
            EventOccurrenceKey,
        ),
        ResourceEventId,
    >,
    deliveries: HashMap<ScopedDeliveryKey, DeliveryRow>,
    handoffs: HashMap<ScopedHandoffKey, HandoffRow>,
}

#[derive(Clone)]
struct SubscriptionRow {
    id: ResourceSubscriptionId,
    resource_id: SharedResourceId,
    consumer_kind: ResourceConsumerKind,
    consumer_identity: ResourceConsumerIdentity,
    state: ResourceSubscriptionState,
    version: ResourceSubscriptionVersion,
    reconciliation_sequence: u64,
}

impl SubscriptionRow {
    fn snapshot(&self) -> ResourceSubscriptionRecord {
        ResourceSubscriptionRecord::new(
            self.id,
            self.resource_id,
            self.consumer_kind.clone(),
            self.consumer_identity.clone(),
            self.state,
            self.version,
            self.reconciliation_sequence,
        )
    }
}

#[derive(Clone)]
struct SourceLeaseRow {
    holder: ResourceLeaseHolder,
    token: ResourceSourceLeaseToken,
    expires_at: DateTime<Utc>,
}

impl SourceLeaseRow {
    fn snapshot(&self, resource_id: SharedResourceId) -> ResourceSourceLease {
        ResourceSourceLease::new(
            resource_id,
            self.holder.clone(),
            self.token.clone(),
            self.expires_at,
        )
    }
}

#[derive(Clone)]
struct EventRow {
    id: ResourceEventId,
    resource_id: SharedResourceId,
    namespace: EventOccurrenceNamespace,
    occurrence_key: EventOccurrenceKey,
    envelope: EventEnvelope,
    accepted_at: DateTime<Utc>,
    source_generation: ResourceLeaseGeneration,
    state: ResourceEventState,
}

impl EventRow {
    fn snapshot(&self) -> ResourceEventRecord {
        ResourceEventRecord::new(
            self.id,
            self.resource_id,
            self.namespace.clone(),
            self.occurrence_key.clone(),
            self.envelope.clone(),
            ResourceEventAcceptance::new(self.accepted_at, self.source_generation),
            self.state,
        )
    }
}

#[derive(Clone)]
struct DeliveryClaim {
    holder: ResourceLeaseHolder,
    token: ResourceDeliveryClaimToken,
    expires_at: DateTime<Utc>,
}

#[derive(Clone)]
struct DeliveryRow {
    sequence: u64,
    id: ResourceDeliveryId,
    event_id: ResourceEventId,
    subscription_id: ResourceSubscriptionId,
    envelope: EventEnvelope,
    last_generation: ResourceLeaseGeneration,
    claim: Option<DeliveryClaim>,
    terminal: Option<DeliveryTerminal>,
}

#[derive(Clone)]
struct DeliveryTerminal {
    token: ResourceDeliveryClaimToken,
    requested_completion: ResourceDeliveryCompletion,
    actual_completion: ResourceDeliveryCompletion,
}

impl DeliveryRow {
    fn claimed_snapshot(&self) -> Result<ClaimedResourceDelivery, StorageError> {
        let claim = self.claim.as_ref().ok_or_else(|| {
            StorageError::Internal("resource delivery claim disappeared".to_owned())
        })?;
        debug_assert!(!claim.holder.as_str().is_empty());
        Ok(ClaimedResourceDelivery::new(
            self.id,
            self.event_id,
            self.subscription_id,
            self.envelope.clone(),
            claim.token.clone(),
        ))
    }
}

#[derive(Clone)]
struct HandoffClaim {
    token: ResourceHandoffClaimToken,
    expires_at: DateTime<Utc>,
}

#[derive(Clone)]
struct HandoffRow {
    sequence: u64,
    delivery_id: ResourceDeliveryId,
    event_id: ResourceEventId,
    subscription_id: ResourceSubscriptionId,
    envelope: EventEnvelope,
    last_generation: ResourceLeaseGeneration,
    claim: Option<HandoffClaim>,
    acknowledged_by: Option<ResourceHandoffClaimToken>,
}

impl HandoffRow {
    fn claimed_snapshot(&self) -> Result<ClaimedResourceHandoff, StorageError> {
        let claim = self.claim.as_ref().ok_or_else(|| {
            StorageError::Internal("resource handoff claim disappeared".to_owned())
        })?;
        Ok(ClaimedResourceHandoff::new(
            self.delivery_id,
            self.event_id,
            self.subscription_id,
            self.envelope.clone(),
            claim.token.clone(),
        ))
    }
}

/// In-memory reference implementation of all shared-resource runtime roles.
///
/// One mutex guards every map and index, matching the transaction boundary
/// required of deployment adapters. This adapter is for tests and conformance,
/// not a supported persistence backend.
#[derive(Clone)]
pub struct InMemoryResourceRuntime {
    inner: Arc<Mutex<State>>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for InMemoryResourceRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryResourceRuntime")
            .finish_non_exhaustive()
    }
}

impl Default for InMemoryResourceRuntime {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(State::default())),
            clock: Arc::new(nebula_core::accessor::SystemClock),
        }
    }
}

impl InMemoryResourceRuntime {
    /// Create an empty adapter driven by the system clock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty adapter driven by an injected authoritative clock.
    #[must_use]
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State::default())),
            clock,
        }
    }
}

#[async_trait::async_trait]
impl SharedResourceStore for InMemoryResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "shared_resource", storage.operation = "resolve"))]
    async fn resolve(
        &self,
        request: ResolveSharedResourceRequest,
    ) -> Result<ResolveSharedResourceOutcome, StorageError> {
        let mut state = self.inner.lock();
        let digest_bucket = identity_bucket(&state, request.identity());
        let bucket_key = (request.scope().clone(), digest_bucket);
        if let Some(resource_ids) = state.resource_identity_buckets.get(&bucket_key)
            && let Some(record) = resource_ids.iter().find_map(|resource_id| {
                state
                    .resources
                    .get(&(request.scope().clone(), *resource_id))
                    .filter(|record| record.identity() == request.identity())
            })
        {
            return Ok(ResolveSharedResourceOutcome::Existing(record.clone()));
        }

        let reconciliation_sequence = state
            .next_reconciliation_sequence
            .checked_add(1)
            .ok_or_else(generation_exhausted)?;
        let resource_id = next_shared_resource_id(&state);
        let record = SharedResourceRecord::new(
            resource_id,
            request.identity().clone(),
            reconciliation_sequence,
        );
        state.next_reconciliation_sequence = reconciliation_sequence;
        state
            .resources
            .insert((request.scope().clone(), resource_id), record.clone());
        state
            .resource_identity_buckets
            .entry(bucket_key)
            .or_default()
            .push(resource_id);
        Ok(ResolveSharedResourceOutcome::Created(record))
    }

    async fn get(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<Option<SharedResourceRecord>, StorageError> {
        Ok(self
            .inner
            .lock()
            .resources
            .get(&(scope.clone(), resource_id))
            .cloned())
    }

    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<SharedResourcePage, StorageError> {
        let state = self.inner.lock();
        let exclusive_sequence = after.map_or(0, ReconciliationCursor::sequence);
        let mut resources = state
            .resources
            .iter()
            .filter_map(|((resource_scope, _), record)| {
                (resource_scope == scope && record.reconciliation_sequence() > exclusive_sequence)
                    .then_some(record.clone())
            })
            .collect::<Vec<_>>();
        resources.sort_unstable_by_key(SharedResourceRecord::reconciliation_sequence);
        resources.truncate(usize::from(page_size.get()));
        let next_cursor = (resources.len() == usize::from(page_size.get()))
            .then(|| resources.last())
            .flatten()
            .map(|record| ReconciliationCursor::from_sequence(record.reconciliation_sequence()));
        Ok(SharedResourcePage::new(resources, next_cursor))
    }
}

#[async_trait::async_trait]
impl ResourceSubscriptionStore for InMemoryResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_subscription", storage.operation = "put", resource_id = ?request.resource_id()))]
    async fn put(
        &self,
        request: PutResourceSubscriptionRequest,
    ) -> Result<PutResourceSubscriptionOutcome, StorageError> {
        let mut state = self.inner.lock();
        ensure_resource_exists(&state, request.scope(), request.resource_id())?;
        let identity_key = (
            request.scope().clone(),
            request.resource_id(),
            request.consumer_kind().clone(),
            ResourceConsumerIdentity::try_from_vec(request.consumer_identity_bytes().to_vec())
                .map_err(|_| invalid_internal_value())?,
        );
        if let Some(subscription_id) = state.subscription_identities.get(&identity_key)
            && let Some(row) = state
                .subscriptions
                .get(&(request.scope().clone(), *subscription_id))
        {
            return Ok(PutResourceSubscriptionOutcome::Existing(row.snapshot()));
        }

        let subscription_id = next_subscription_id(&state);
        let reconciliation_sequence = state
            .next_subscription_sequence
            .checked_add(1)
            .ok_or_else(generation_exhausted)?;
        let row = SubscriptionRow {
            id: subscription_id,
            resource_id: request.resource_id(),
            consumer_kind: request.consumer_kind().clone(),
            consumer_identity: identity_key.3.clone(),
            state: ResourceSubscriptionState::Active,
            version: ResourceSubscriptionVersion::new(1),
            reconciliation_sequence,
        };
        state.next_subscription_sequence = reconciliation_sequence;
        state
            .subscription_identities
            .insert(identity_key, subscription_id);
        state
            .subscriptions
            .insert((request.scope().clone(), subscription_id), row.clone());
        Ok(PutResourceSubscriptionOutcome::Created(row.snapshot()))
    }

    async fn get(
        &self,
        scope: &Scope,
        subscription_id: ResourceSubscriptionId,
    ) -> Result<Option<ResourceSubscriptionRecord>, StorageError> {
        Ok(self
            .inner
            .lock()
            .subscriptions
            .get(&(scope.clone(), subscription_id))
            .map(SubscriptionRow::snapshot))
    }

    async fn list_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError> {
        let state = self.inner.lock();
        Ok(subscription_page(
            state
                .subscriptions
                .iter()
                .filter(|((subscription_scope, _), row)| {
                    subscription_scope == scope
                        && row.resource_id == resource_id
                        && row.state == ResourceSubscriptionState::Active
                })
                .map(|(_, row)| row.snapshot())
                .filter(|record| {
                    record.reconciliation_sequence()
                        > after.map_or(0, ReconciliationCursor::sequence)
                })
                .collect(),
            page_size,
        ))
    }

    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError> {
        let state = self.inner.lock();
        Ok(subscription_page(
            state
                .subscriptions
                .iter()
                .filter(|((subscription_scope, _), _)| subscription_scope == scope)
                .map(|(_, row)| row.snapshot())
                .filter(|record| {
                    record.reconciliation_sequence()
                        > after.map_or(0, ReconciliationCursor::sequence)
                })
                .collect(),
            page_size,
        ))
    }

    async fn count_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<u64, StorageError> {
        self.inner
            .lock()
            .subscriptions
            .iter()
            .filter(|((subscription_scope, _), row)| {
                subscription_scope == scope
                    && row.resource_id == resource_id
                    && row.state == ResourceSubscriptionState::Active
            })
            .count()
            .try_into()
            .map_err(|_| StorageError::Internal("active subscription count exceeds u64".to_owned()))
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_subscription", storage.operation = "transition", subscription_id = ?request.subscription_id()))]
    async fn transition(
        &self,
        request: TransitionResourceSubscriptionRequest,
    ) -> Result<ResourceSubscriptionRecord, StorageError> {
        let mut state = self.inner.lock();
        let row = state
            .subscriptions
            .get_mut(&(request.scope().clone(), request.subscription_id()))
            .ok_or_else(|| opaque_not_found("resource subscription"))?;
        if row.version != request.expected_version() {
            let replay_version = request.expected_version().get().checked_add(1);
            if replay_version == Some(row.version.get()) && row.state == request.target_state() {
                return Ok(row.snapshot());
            }
            return Err(StorageError::Conflict {
                entity: "resource subscription",
                id: "[opaque]".to_owned(),
                expected: request.expected_version().get(),
                actual: row.version.get(),
            });
        }
        if row.state == ResourceSubscriptionState::Tombstoned {
            return Err(StorageError::Conflict {
                entity: "resource subscription",
                id: "[opaque]".to_owned(),
                expected: request.expected_version().get(),
                actual: row.version.get(),
            });
        }
        let next_version = row
            .version
            .get()
            .checked_add(1)
            .ok_or_else(generation_exhausted)?;
        row.state = request.target_state();
        row.version = ResourceSubscriptionVersion::new(next_version);
        Ok(row.snapshot())
    }
}

#[async_trait::async_trait]
impl ResourceSourceLeaseStore for InMemoryResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_source_lease", storage.operation = "acquire", resource_id = ?request.resource_id()))]
    async fn acquire(
        &self,
        request: AcquireResourceSourceLeaseRequest,
    ) -> Result<AcquireResourceSourceLeaseOutcome, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        ensure_resource_exists(&state, request.scope(), request.resource_id())?;
        let key = (request.scope().clone(), request.resource_id());
        if let Some(existing) = state.source_leases.get(&key)
            && now < existing.expires_at
        {
            return Ok(AcquireResourceSourceLeaseOutcome::Contended {
                expires_at: existing.expires_at,
            });
        }
        let generation = state.source_leases.get(&key).map_or(
            Ok(ResourceLeaseGeneration::new(1)),
            |existing| {
                existing
                    .token
                    .generation()
                    .checked_next()
                    .map_err(|_| generation_exhausted())
            },
        )?;
        let row = SourceLeaseRow {
            holder: request.holder().clone(),
            token: ResourceSourceLeaseToken::new(Uuid::new_v4(), generation),
            expires_at,
        };
        let lease = row.snapshot(request.resource_id());
        state.source_leases.insert(key, row);
        Ok(AcquireResourceSourceLeaseOutcome::Acquired(lease))
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_source_lease", storage.operation = "heartbeat", resource_id = ?request.resource_id(), generation = request.token().generation().get()))]
    async fn heartbeat(
        &self,
        request: HeartbeatResourceSourceLeaseRequest,
    ) -> Result<ResourceSourceLease, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        let row = state
            .source_leases
            .get_mut(&(request.scope().clone(), request.resource_id()))
            .ok_or_else(|| opaque_fenced("resource source lease"))?;
        ensure_live_source_token(row, request.token(), now)?;
        row.expires_at = expires_at;
        Ok(row.snapshot(request.resource_id()))
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_source_lease", storage.operation = "release", resource_id = ?request.resource_id(), generation = request.token().generation().get()))]
    async fn release(
        &self,
        request: ReleaseResourceSourceLeaseRequest,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let key = (request.scope().clone(), request.resource_id());
        if let Some(row) = state.source_leases.get_mut(&key)
            && row.token == *request.token()
        {
            row.expires_at = now;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ResourceEventFanoutStore for InMemoryResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "accept", resource_id = ?request.resource_id(), source_generation = request.source_token().generation().get()))]
    async fn accept(
        &self,
        request: AcceptResourceEventRequest,
    ) -> Result<AcceptResourceEventOutcome, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let source_lease = state
            .source_leases
            .get(&(request.scope().clone(), request.resource_id()))
            .ok_or_else(|| opaque_fenced("resource source lease"))?;
        ensure_live_source_token(source_lease, request.source_token(), now)?;

        let occurrence_key = (
            request.scope().clone(),
            request.resource_id(),
            request.namespace().clone(),
            EventOccurrenceKey::try_from_vec(request.occurrence_key_bytes().to_vec())
                .map_err(|_| invalid_internal_value())?,
        );
        if let Some(event_id) = state.event_occurrences.get(&occurrence_key)
            && let Some(event) = state.events.get(&(request.scope().clone(), *event_id))
        {
            let outcome = if event.envelope == *request.envelope() {
                AcceptResourceEventOutcome::Replayed {
                    event_id: *event_id,
                }
            } else {
                AcceptResourceEventOutcome::Conflict {
                    event_id: *event_id,
                }
            };
            trace_acceptance_outcome(&outcome);
            return Ok(outcome);
        }

        let mut subscription_ids = state
            .subscriptions
            .iter()
            .filter_map(|((subscription_scope, subscription_id), row)| {
                (subscription_scope == request.scope()
                    && row.resource_id == request.resource_id()
                    && row.state == ResourceSubscriptionState::Active)
                    .then_some(*subscription_id)
            })
            .collect::<Vec<_>>();
        subscription_ids.sort_unstable();
        let delivery_count = u32::try_from(subscription_ids.len()).map_err(|_| {
            StorageError::Internal("resource delivery count exceeds u32".to_owned())
        })?;
        let has_deliveries = !subscription_ids.is_empty();
        let event_id = next_event_id(&state);
        let mut next_delivery_sequence = state.next_delivery_sequence;
        let mut deliveries = Vec::with_capacity(subscription_ids.len());
        for subscription_id in subscription_ids {
            next_delivery_sequence = next_delivery_sequence
                .checked_add(1)
                .ok_or_else(generation_exhausted)?;
            deliveries.push(DeliveryRow {
                sequence: next_delivery_sequence,
                id: next_delivery_id(&state, &deliveries),
                event_id,
                subscription_id,
                envelope: request.envelope().clone(),
                last_generation: ResourceLeaseGeneration::new(0),
                claim: None,
                terminal: None,
            });
        }
        let event = EventRow {
            id: event_id,
            resource_id: request.resource_id(),
            namespace: request.namespace().clone(),
            occurrence_key: occurrence_key.3.clone(),
            envelope: request.envelope().clone(),
            accepted_at: now,
            source_generation: request.source_token().generation(),
            state: if has_deliveries {
                ResourceEventState::Pending
            } else {
                ResourceEventState::Complete
            },
        };
        state.event_occurrences.insert(occurrence_key, event_id);
        state
            .events
            .insert((request.scope().clone(), event_id), event);
        state.next_delivery_sequence = next_delivery_sequence;
        for delivery in deliveries {
            state
                .deliveries
                .insert((request.scope().clone(), delivery.id), delivery);
        }
        let outcome = AcceptResourceEventOutcome::Accepted {
            event_id,
            delivery_count,
        };
        trace_acceptance_outcome(&outcome);
        Ok(outcome)
    }

    async fn get_event(
        &self,
        scope: &Scope,
        event_id: ResourceEventId,
    ) -> Result<Option<ResourceEventRecord>, StorageError> {
        Ok(self
            .inner
            .lock()
            .events
            .get(&(scope.clone(), event_id))
            .map(EventRow::snapshot))
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "claim_deliveries"))]
    async fn claim_deliveries(
        &self,
        request: ClaimResourceDeliveriesRequest,
    ) -> Result<Vec<ClaimedResourceDelivery>, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        let mut candidates = state
            .deliveries
            .iter()
            .filter_map(|((delivery_scope, delivery_id), row)| {
                (delivery_scope == request.scope()
                    && row.terminal.is_none()
                    && row
                        .claim
                        .as_ref()
                        .is_none_or(|claim| now >= claim.expires_at))
                .then_some((row.sequence, *delivery_id))
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(sequence, _)| *sequence);
        candidates.truncate(usize::from(request.batch_size().get()));

        let prepared = candidates
            .into_iter()
            .map(|(_, delivery_id)| {
                let row = state
                    .deliveries
                    .get(&(request.scope().clone(), delivery_id))
                    .ok_or_else(|| opaque_not_found("resource delivery"))?;
                let generation = row
                    .last_generation
                    .checked_next()
                    .map_err(|_| generation_exhausted())?;
                Ok((
                    delivery_id,
                    generation,
                    ResourceDeliveryClaimToken::new(Uuid::new_v4(), generation),
                ))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        let mut claimed = Vec::with_capacity(prepared.len());
        for (delivery_id, generation, token) in prepared {
            let row = state
                .deliveries
                .get_mut(&(request.scope().clone(), delivery_id))
                .ok_or_else(|| opaque_not_found("resource delivery"))?;
            row.last_generation = generation;
            row.claim = Some(DeliveryClaim {
                holder: request.holder().clone(),
                token,
                expires_at,
            });
            claimed.push(row.claimed_snapshot()?);
        }
        tracing::debug!(storage.outcome = "claimed", claim_count = claimed.len());
        Ok(claimed)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "heartbeat_delivery", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn heartbeat_delivery(
        &self,
        request: HeartbeatResourceDeliveryRequest,
    ) -> Result<ClaimedResourceDelivery, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        let row = state
            .deliveries
            .get_mut(&(request.scope().clone(), request.delivery_id()))
            .ok_or_else(|| opaque_fenced("resource delivery"))?;
        let claim = row
            .claim
            .as_mut()
            .filter(|claim| claim.token == *request.token() && now < claim.expires_at)
            .ok_or_else(|| opaque_fenced("resource delivery"))?;
        claim.expires_at = expires_at;
        row.claimed_snapshot()
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "release_delivery", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn release_delivery(
        &self,
        request: ReleaseResourceDeliveryRequest,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if let Some(row) = state
            .deliveries
            .get_mut(&(request.scope().clone(), request.delivery_id()))
            && row
                .claim
                .as_ref()
                .is_some_and(|claim| claim.token == *request.token())
        {
            row.claim = None;
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "complete_delivery", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn complete_delivery(
        &self,
        request: CompleteResourceDeliveryRequest,
    ) -> Result<CompleteResourceDeliveryOutcome, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let key = (request.scope().clone(), request.delivery_id());
        if let Some(row) = state.deliveries.get(&key)
            && let Some(terminal) = &row.terminal
        {
            let terminal_token = terminal.token.clone();
            let requested_completion = terminal.requested_completion;
            let actual_completion = terminal.actual_completion;
            if terminal_token != *request.token() || requested_completion != request.completion() {
                return Err(opaque_fenced("resource delivery"));
            }
            let event_id = row.event_id;
            let subscription_id = row.subscription_id;
            let envelope = row.envelope.clone();
            if actual_completion == ResourceDeliveryCompletion::Delivered {
                ensure_handoff(
                    &mut state,
                    request.scope(),
                    request.delivery_id(),
                    event_id,
                    subscription_id,
                    envelope,
                )?;
            }
            let event_is_terminal = state
                .events
                .get(&(request.scope().clone(), event_id))
                .is_some_and(|event| event.state == ResourceEventState::Complete);
            let outcome = CompleteResourceDeliveryOutcome::AlreadyCompleted {
                completion: actual_completion,
                event_is_terminal,
            };
            trace_completion_outcome(outcome);
            return Ok(outcome);
        }
        let persisted_completion = effective_delivery_completion(
            &state,
            request.scope(),
            request.delivery_id(),
            request.completion(),
        )?;
        let (event_id, subscription_id, envelope, terminal_token) = {
            let row = state
                .deliveries
                .get(&key)
                .ok_or_else(|| opaque_fenced("resource delivery"))?;
            let claim = row
                .claim
                .as_ref()
                .filter(|claim| claim.token == *request.token() && now < claim.expires_at)
                .ok_or_else(|| opaque_fenced("resource delivery"))?;
            (
                row.event_id,
                row.subscription_id,
                row.envelope.clone(),
                claim.token.clone(),
            )
        };
        if persisted_completion == ResourceDeliveryCompletion::Delivered {
            ensure_handoff(
                &mut state,
                request.scope(),
                request.delivery_id(),
                event_id,
                subscription_id,
                envelope,
            )?;
        }
        {
            let row = state
                .deliveries
                .get_mut(&key)
                .ok_or_else(|| opaque_fenced("resource delivery"))?;
            row.terminal = Some(DeliveryTerminal {
                token: terminal_token,
                requested_completion: request.completion(),
                actual_completion: persisted_completion,
            });
            row.claim = None;
        }
        let event_became_terminal =
            state
                .deliveries
                .iter()
                .all(|((delivery_scope, _), delivery)| {
                    delivery_scope != request.scope()
                        || delivery.event_id != event_id
                        || delivery.terminal.is_some()
                });
        if event_became_terminal
            && let Some(event) = state.events.get_mut(&(request.scope().clone(), event_id))
        {
            event.state = ResourceEventState::Complete;
        }
        let outcome = CompleteResourceDeliveryOutcome::Completed {
            completion: persisted_completion,
            event_became_terminal,
        };
        trace_completion_outcome(outcome);
        Ok(outcome)
    }
}

#[async_trait::async_trait]
impl ResourceExecutionHandoffStore for InMemoryResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "claim"))]
    async fn claim_handoffs(
        &self,
        request: ClaimResourceHandoffsRequest,
    ) -> Result<Vec<ClaimedResourceHandoff>, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        let mut keys = state
            .handoffs
            .iter()
            .filter_map(|((handoff_scope, delivery_id), row)| {
                (handoff_scope == request.scope()
                    && row.acknowledged_by.is_none()
                    && row
                        .claim
                        .as_ref()
                        .is_none_or(|claim| now >= claim.expires_at))
                .then_some((row.sequence, *delivery_id))
            })
            .collect::<Vec<_>>();
        keys.sort_unstable_by_key(|(sequence, _)| *sequence);
        keys.truncate(usize::from(request.batch_size().get()));
        let prepared = keys
            .into_iter()
            .map(|(_, delivery_id)| {
                let row = state
                    .handoffs
                    .get(&(request.scope().clone(), delivery_id))
                    .ok_or_else(|| opaque_not_found("resource execution handoff"))?;
                let generation = row
                    .last_generation
                    .checked_next()
                    .map_err(|_| generation_exhausted())?;
                Ok((delivery_id, generation))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let mut claimed = Vec::with_capacity(prepared.len());
        for (delivery_id, generation) in prepared {
            let row = state
                .handoffs
                .get_mut(&(request.scope().clone(), delivery_id))
                .ok_or_else(|| opaque_not_found("resource execution handoff"))?;
            row.last_generation = generation;
            row.claim = Some(HandoffClaim {
                token: ResourceHandoffClaimToken::new(Uuid::new_v4(), generation),
                expires_at,
            });
            claimed.push(row.claimed_snapshot()?);
        }
        tracing::debug!(storage.outcome = "claimed", claim_count = claimed.len());
        Ok(claimed)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "heartbeat", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn heartbeat_handoff(
        &self,
        request: HeartbeatResourceHandoffRequest,
    ) -> Result<ClaimedResourceHandoff, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        let row = state
            .handoffs
            .get_mut(&(request.scope().clone(), request.delivery_id()))
            .ok_or_else(|| opaque_fenced("resource execution handoff"))?;
        let claim = row
            .claim
            .as_mut()
            .filter(|claim| claim.token == *request.token() && now < claim.expires_at)
            .ok_or_else(|| opaque_fenced("resource execution handoff"))?;
        claim.expires_at = expires_at;
        row.claimed_snapshot()
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "release", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn release_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if let Some(row) = state
            .handoffs
            .get_mut(&(request.scope().clone(), request.delivery_id()))
            && row
                .claim
                .as_ref()
                .is_some_and(|claim| claim.token == *request.token())
        {
            row.claim = None;
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "acknowledge", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn acknowledge_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<AcknowledgeResourceHandoffOutcome, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let row = state
            .handoffs
            .get_mut(&(request.scope().clone(), request.delivery_id()))
            .ok_or_else(|| opaque_fenced("resource execution handoff"))?;
        if let Some(token) = &row.acknowledged_by {
            return if token == request.token() {
                tracing::debug!(storage.outcome = "already_acknowledged");
                Ok(AcknowledgeResourceHandoffOutcome::AlreadyAcknowledged)
            } else {
                Err(opaque_fenced("resource execution handoff"))
            };
        }
        let claim = row
            .claim
            .as_ref()
            .filter(|claim| claim.token == *request.token() && now < claim.expires_at)
            .ok_or_else(|| opaque_fenced("resource execution handoff"))?;
        row.acknowledged_by = Some(claim.token.clone());
        row.claim = None;
        tracing::debug!(storage.outcome = "acknowledged");
        Ok(AcknowledgeResourceHandoffOutcome::Acknowledged)
    }
}

#[async_trait::async_trait]
impl ResourceRuntimeRecovery for InMemoryResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_runtime_recovery", storage.operation = "claim_deliveries"))]
    async fn claim_deliveries_globally(
        &self,
        request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceDelivery>, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        let mut candidates = state
            .deliveries
            .iter()
            .filter_map(|((scope, delivery_id), row)| {
                (row.terminal.is_none()
                    && row
                        .claim
                        .as_ref()
                        .is_none_or(|claim| now >= claim.expires_at))
                .then_some((row.sequence, scope.clone(), *delivery_id))
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(sequence, _, _)| *sequence);
        candidates.truncate(usize::from(request.batch_size().get()));

        let prepared = candidates
            .into_iter()
            .map(|(_, scope, delivery_id)| {
                let row = state
                    .deliveries
                    .get(&(scope.clone(), delivery_id))
                    .ok_or_else(|| opaque_not_found("resource delivery"))?;
                let generation = row
                    .last_generation
                    .checked_next()
                    .map_err(|_| generation_exhausted())?;
                Ok((scope, delivery_id, generation, Uuid::new_v4()))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        let mut claimed = Vec::with_capacity(prepared.len());
        for (scope, delivery_id, generation, claim_id) in prepared {
            let row = state
                .deliveries
                .get_mut(&(scope.clone(), delivery_id))
                .ok_or_else(|| opaque_not_found("resource delivery"))?;
            row.last_generation = generation;
            row.claim = Some(DeliveryClaim {
                holder: request.holder().clone(),
                token: ResourceDeliveryClaimToken::new(claim_id, generation),
                expires_at,
            });
            claimed.push(ScopedClaimedResourceDelivery::new(
                scope,
                row.claimed_snapshot()?,
            ));
        }
        tracing::debug!(storage.outcome = "claimed", claim_count = claimed.len());
        Ok(claimed)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_runtime_recovery", storage.operation = "claim_handoffs"))]
    async fn claim_handoffs_globally(
        &self,
        request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceHandoff>, StorageError> {
        let mut state = self.inner.lock();
        let now = self.clock.now();
        let expires_at = checked_expiry(now, request.ttl())?;
        let mut candidates = state
            .handoffs
            .iter()
            .filter_map(|((scope, delivery_id), row)| {
                (row.acknowledged_by.is_none()
                    && row
                        .claim
                        .as_ref()
                        .is_none_or(|claim| now >= claim.expires_at))
                .then_some((row.sequence, scope.clone(), *delivery_id))
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(sequence, _, _)| *sequence);
        candidates.truncate(usize::from(request.batch_size().get()));

        let prepared = candidates
            .into_iter()
            .map(|(_, scope, delivery_id)| {
                let row = state
                    .handoffs
                    .get(&(scope.clone(), delivery_id))
                    .ok_or_else(|| opaque_not_found("resource execution handoff"))?;
                let generation = row
                    .last_generation
                    .checked_next()
                    .map_err(|_| generation_exhausted())?;
                Ok((scope, delivery_id, generation, Uuid::new_v4()))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        let mut claimed = Vec::with_capacity(prepared.len());
        for (scope, delivery_id, generation, claim_id) in prepared {
            let row = state
                .handoffs
                .get_mut(&(scope.clone(), delivery_id))
                .ok_or_else(|| opaque_not_found("resource execution handoff"))?;
            row.last_generation = generation;
            row.claim = Some(HandoffClaim {
                token: ResourceHandoffClaimToken::new(claim_id, generation),
                expires_at,
            });
            claimed.push(ScopedClaimedResourceHandoff::new(
                scope,
                row.claimed_snapshot()?,
            ));
        }
        tracing::debug!(storage.outcome = "claimed", claim_count = claimed.len());
        Ok(claimed)
    }
}

fn ensure_handoff(
    state: &mut State,
    scope: &Scope,
    delivery_id: ResourceDeliveryId,
    event_id: ResourceEventId,
    subscription_id: ResourceSubscriptionId,
    envelope: EventEnvelope,
) -> Result<(), StorageError> {
    let key = (scope.clone(), delivery_id);
    if let Some(existing) = state.handoffs.get(&key) {
        if existing.event_id == event_id
            && existing.subscription_id == subscription_id
            && existing.envelope == envelope
        {
            return Ok(());
        }
        return Err(StorageError::Internal(
            "resource execution handoff identity conflict".to_owned(),
        ));
    }
    let sequence = state
        .next_handoff_sequence
        .checked_add(1)
        .ok_or_else(generation_exhausted)?;
    state.next_handoff_sequence = sequence;
    state.handoffs.insert(
        key,
        HandoffRow {
            sequence,
            delivery_id,
            event_id,
            subscription_id,
            envelope,
            last_generation: ResourceLeaseGeneration::new(0),
            claim: None,
            acknowledged_by: None,
        },
    );
    tracing::debug!(storage.outcome = "handoff_confirmed", ?delivery_id);
    Ok(())
}

fn trace_acceptance_outcome(outcome: &AcceptResourceEventOutcome) {
    match outcome {
        AcceptResourceEventOutcome::Accepted {
            event_id,
            delivery_count,
        } => tracing::debug!(storage.outcome = "accepted", ?event_id, delivery_count),
        AcceptResourceEventOutcome::Replayed { event_id } => {
            tracing::debug!(storage.outcome = "replayed", ?event_id);
        },
        AcceptResourceEventOutcome::Conflict { event_id } => {
            tracing::warn!(storage.outcome = "conflict", ?event_id);
        },
    }
}

fn trace_completion_outcome(outcome: CompleteResourceDeliveryOutcome) {
    let (outcome_name, completion) = match outcome {
        CompleteResourceDeliveryOutcome::Completed { completion, .. } => ("completed", completion),
        CompleteResourceDeliveryOutcome::AlreadyCompleted { completion, .. } => {
            ("completion_replayed", completion)
        },
    };
    let terminal = match completion {
        ResourceDeliveryCompletion::Delivered => "delivered",
        ResourceDeliveryCompletion::Ineligible(reason) => reason.as_str(),
    };
    tracing::debug!(storage.outcome = outcome_name, terminal);
}

fn identity_bucket(state: &State, identity: &SharedResourceIdentity) -> [u8; 32] {
    if state.force_identity_digest_collision {
        [0; 32]
    } else {
        *identity.digest()
    }
}

fn subscription_page(
    mut subscriptions: Vec<ResourceSubscriptionRecord>,
    page_size: ResourcePageSize,
) -> ResourceSubscriptionPage {
    subscriptions.sort_unstable_by_key(ResourceSubscriptionRecord::reconciliation_sequence);
    subscriptions.truncate(usize::from(page_size.get()));
    let next_cursor = (subscriptions.len() == usize::from(page_size.get()))
        .then(|| subscriptions.last())
        .flatten()
        .map(|record| ReconciliationCursor::from_sequence(record.reconciliation_sequence()));
    ResourceSubscriptionPage::new(subscriptions, next_cursor)
}

fn effective_delivery_completion(
    state: &State,
    scope: &Scope,
    delivery_id: ResourceDeliveryId,
    requested: ResourceDeliveryCompletion,
) -> Result<ResourceDeliveryCompletion, StorageError> {
    if requested != ResourceDeliveryCompletion::Delivered {
        return Ok(requested);
    }
    let delivery = state
        .deliveries
        .get(&(scope.clone(), delivery_id))
        .ok_or_else(|| opaque_fenced("resource delivery"))?;
    let subscription = state
        .subscriptions
        .get(&(scope.clone(), delivery.subscription_id))
        .ok_or_else(invalid_internal_value)?;
    Ok(match subscription.state {
        ResourceSubscriptionState::Active => ResourceDeliveryCompletion::Delivered,
        ResourceSubscriptionState::Disabled => ResourceDeliveryCompletion::Ineligible(
            TerminalDeliveryIneligibility::SubscriptionDisabled,
        ),
        ResourceSubscriptionState::Tombstoned => ResourceDeliveryCompletion::Ineligible(
            TerminalDeliveryIneligibility::SubscriptionTombstoned,
        ),
    })
}

fn checked_expiry(
    now: DateTime<Utc>,
    ttl: ResourceLeaseTtl,
) -> Result<DateTime<Utc>, StorageError> {
    let duration = TimeDelta::from_std(ttl.get())
        .map_err(|_| StorageError::Internal("resource lease TTL conversion failed".to_owned()))?;
    now.checked_add_signed(duration)
        .ok_or_else(|| StorageError::Internal("resource lease expiry overflow".to_owned()))
}

fn ensure_resource_exists(
    state: &State,
    scope: &Scope,
    resource_id: SharedResourceId,
) -> Result<(), StorageError> {
    if state.resources.contains_key(&(scope.clone(), resource_id)) {
        Ok(())
    } else {
        Err(opaque_not_found("shared resource"))
    }
}

fn ensure_live_source_token(
    lease: &SourceLeaseRow,
    token: &ResourceSourceLeaseToken,
    now: DateTime<Utc>,
) -> Result<(), StorageError> {
    if lease.token == *token && now < lease.expires_at {
        Ok(())
    } else {
        Err(opaque_fenced("resource source lease"))
    }
}

fn opaque_not_found(entity: &'static str) -> StorageError {
    StorageError::not_found(entity, "[opaque]")
}

fn opaque_fenced(entity: &'static str) -> StorageError {
    tracing::warn!(storage.outcome = "fenced", storage.entity = entity);
    StorageError::FencedOut {
        entity,
        id: "[opaque]".to_owned(),
    }
}

fn generation_exhausted() -> StorageError {
    StorageError::Internal("resource generation exhausted".to_owned())
}

fn invalid_internal_value() -> StorageError {
    StorageError::Internal("validated resource value could not be reconstructed".to_owned())
}

fn next_shared_resource_id(state: &State) -> SharedResourceId {
    loop {
        let candidate = SharedResourceId::from_bytes(*Uuid::new_v4().as_bytes());
        if state
            .resources
            .keys()
            .all(|(_, resource_id)| *resource_id != candidate)
        {
            return candidate;
        }
    }
}

fn next_subscription_id(state: &State) -> ResourceSubscriptionId {
    loop {
        let candidate = ResourceSubscriptionId::from_bytes(*Uuid::new_v4().as_bytes());
        if state
            .subscriptions
            .keys()
            .all(|(_, subscription_id)| *subscription_id != candidate)
        {
            return candidate;
        }
    }
}

fn next_event_id(state: &State) -> ResourceEventId {
    loop {
        let candidate = ResourceEventId::from_bytes(*Uuid::new_v4().as_bytes());
        if state
            .events
            .keys()
            .all(|(_, event_id)| *event_id != candidate)
        {
            return candidate;
        }
    }
}

fn next_delivery_id(state: &State, prepared: &[DeliveryRow]) -> ResourceDeliveryId {
    loop {
        let candidate = ResourceDeliveryId::from_bytes(*Uuid::new_v4().as_bytes());
        if state
            .deliveries
            .keys()
            .all(|(_, delivery_id)| *delivery_id != candidate)
            && prepared.iter().all(|row| row.id != candidate)
        {
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    use chrono::{DateTime, Utc};
    use nebula_core::accessor::Clock;
    use nebula_storage_port::dto::{
        AcceptResourceEventRequest, AcquireResourceSourceLeaseOutcome,
        AcquireResourceSourceLeaseRequest, ClaimResourceDeliveriesRequest, EventEnvelope,
        EventOccurrenceKey, EventOccurrenceNamespace, HeartbeatResourceSourceLeaseRequest,
        PutResourceSubscriptionRequest, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
        ResourceCompatibilityVersion, ResourceConfigurationIdentity, ResourceConsumerIdentity,
        ResourceConsumerKind, ResourceDeliveryId, ResourceEventId, ResourceKind,
        ResourceLeaseGeneration, ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize,
        ResourceSlotIdentity, ResourceSubscriptionId, SharedResourceIdentity,
    };
    use nebula_storage_port::store::{
        ResourceEventFanoutStore, ResourceSourceLeaseStore, ResourceSubscriptionStore,
        SharedResourceStore,
    };
    use nebula_storage_port::{Scope, StorageError};

    use super::{DeliveryRow, InMemoryResourceRuntime};

    #[derive(Debug)]
    struct ContendedClock {
        timestamp_seconds: AtomicI64,
        reads: AtomicUsize,
        monotonic_origin: Instant,
    }

    impl ContendedClock {
        fn new(timestamp_seconds: i64) -> Self {
            Self {
                timestamp_seconds: AtomicI64::new(timestamp_seconds),
                reads: AtomicUsize::new(0),
                monotonic_origin: Instant::now(),
            }
        }

        fn advance(&self, seconds: i64) {
            self.timestamp_seconds.fetch_add(seconds, Ordering::SeqCst);
        }
    }

    impl Clock for ContendedClock {
        fn now(&self) -> DateTime<Utc> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            DateTime::from_timestamp(self.timestamp_seconds.load(Ordering::SeqCst), 0)
                .expect("test timestamp is valid")
        }

        fn monotonic(&self) -> Instant {
            self.monotonic_origin
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn heartbeat_samples_clock_only_after_waiting_for_state_lock() {
        let clock = Arc::new(ContendedClock::new(1_800_000_000));
        let store = InMemoryResourceRuntime::with_clock(clock.clone());
        let scope = Scope::new("clock-workspace", "clock-org");
        let identity = SharedResourceIdentity::new(
            ResourceKind::new("clock.resource").expect("valid kind"),
            ResourceCompatibilityVersion::new(1),
            ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
                .expect("valid configuration"),
            ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
        );
        let resource_id = match store
            .resolve(ResolveSharedResourceRequest::new(scope.clone(), identity))
            .await
            .expect("resource resolves")
        {
            ResolveSharedResourceOutcome::Created(record)
            | ResolveSharedResourceOutcome::Existing(record) => record.id(),
        };
        let lease = match store
            .acquire(AcquireResourceSourceLeaseRequest::new(
                scope.clone(),
                resource_id,
                ResourceLeaseHolder::new("source").expect("valid holder"),
                ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
            ))
            .await
            .expect("source acquires")
        {
            AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
            AcquireResourceSourceLeaseOutcome::Contended { .. } => {
                panic!("fresh source must acquire")
            },
        };
        let reads_before_contention = clock.reads.load(Ordering::SeqCst);
        let state_guard = store.inner.lock();
        let start = Arc::new(Barrier::new(2));
        let task_start = start.clone();
        let task_store = store.clone();
        let handle = tokio::runtime::Handle::current();
        let request = HeartbeatResourceSourceLeaseRequest::new(
            scope,
            resource_id,
            lease.token().clone(),
            ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
        );
        let heartbeat = std::thread::spawn(move || {
            task_start.wait();
            handle.block_on(task_store.heartbeat(request))
        });
        start.wait();
        for _ in 0..1_000 {
            std::thread::yield_now();
        }
        assert_eq!(clock.reads.load(Ordering::SeqCst), reads_before_contention);
        clock.advance(2);
        drop(state_guard);
        assert_matches!(
            heartbeat.join().expect("heartbeat thread joins"),
            Err(StorageError::FencedOut { .. })
        );
    }

    #[tokio::test]
    async fn forced_digest_collision_still_uses_exact_identity() {
        let store = InMemoryResourceRuntime::new();
        store.inner.lock().force_identity_digest_collision = true;
        let scope = Scope::new("collision-workspace", "collision-org");
        let make_identity = |configuration: &[u8]| {
            SharedResourceIdentity::new(
                ResourceKind::new("collision.resource").expect("valid kind"),
                ResourceCompatibilityVersion::new(1),
                ResourceConfigurationIdentity::try_from_vec(configuration.to_vec())
                    .expect("valid configuration"),
                ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
            )
        };
        let first = store
            .resolve(ResolveSharedResourceRequest::new(
                scope.clone(),
                make_identity(b"first"),
            ))
            .await
            .expect("first identity resolves");
        let second = store
            .resolve(ResolveSharedResourceRequest::new(
                scope.clone(),
                make_identity(b"second"),
            ))
            .await
            .expect("colliding exact identity resolves");
        let replay = store
            .resolve(ResolveSharedResourceRequest::new(
                scope,
                make_identity(b"first"),
            ))
            .await
            .expect("first identity replays");
        let id = |outcome: ResolveSharedResourceOutcome| match outcome {
            ResolveSharedResourceOutcome::Created(record)
            | ResolveSharedResourceOutcome::Existing(record) => record.id(),
        };
        let first_id = id(first);
        assert_ne!(first_id, id(second));
        assert_eq!(first_id, id(replay));
    }

    #[tokio::test]
    async fn source_and_delivery_generation_overflow_leave_state_unchanged() {
        let clock = Arc::new(ContendedClock::new(1_800_000_000));
        let store = InMemoryResourceRuntime::with_clock(clock);
        let scope = Scope::new("overflow-workspace", "overflow-org");
        let identity = SharedResourceIdentity::new(
            ResourceKind::new("overflow.resource").expect("valid kind"),
            ResourceCompatibilityVersion::new(1),
            ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
                .expect("valid configuration"),
            ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
        );
        let resource_id = match store
            .resolve(ResolveSharedResourceRequest::new(scope.clone(), identity))
            .await
            .expect("resource resolves")
        {
            ResolveSharedResourceOutcome::Created(record)
            | ResolveSharedResourceOutcome::Existing(record) => record.id(),
        };
        store
            .acquire(AcquireResourceSourceLeaseRequest::new(
                scope.clone(),
                resource_id,
                ResourceLeaseHolder::new("source").expect("valid holder"),
                ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
            ))
            .await
            .expect("source acquires");
        {
            let mut state = store.inner.lock();
            let lease = state
                .source_leases
                .get_mut(&(scope.clone(), resource_id))
                .expect("source row exists");
            lease.token = nebula_storage_port::dto::ResourceSourceLeaseToken::new(
                lease.token.claim_id(),
                ResourceLeaseGeneration::new(u64::MAX),
            );
            lease.expires_at =
                DateTime::from_timestamp(1_700_000_000, 0).expect("test timestamp is valid");
        }
        assert_matches!(
            store
                .acquire(AcquireResourceSourceLeaseRequest::new(
                    scope.clone(),
                    resource_id,
                    ResourceLeaseHolder::new("source-next").expect("valid holder"),
                    ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
                ))
                .await,
            Err(StorageError::Internal(_))
        );
        assert_eq!(
            store
                .inner
                .lock()
                .source_leases
                .get(&(scope.clone(), resource_id))
                .expect("source row remains")
                .token
                .generation(),
            ResourceLeaseGeneration::new(u64::MAX)
        );

        let delivery_id = ResourceDeliveryId::from_bytes([7; 16]);
        store.inner.lock().deliveries.insert(
            (scope.clone(), delivery_id),
            DeliveryRow {
                sequence: 1,
                id: delivery_id,
                event_id: ResourceEventId::from_bytes([8; 16]),
                subscription_id: ResourceSubscriptionId::from_bytes([9; 16]),
                envelope: EventEnvelope::try_from_vec(1, b"payload".to_vec())
                    .expect("valid envelope"),
                last_generation: ResourceLeaseGeneration::new(u64::MAX),
                claim: None,
                terminal: None,
            },
        );
        assert_matches!(
            store
                .claim_deliveries(ClaimResourceDeliveriesRequest::new(
                    scope.clone(),
                    ResourceLeaseHolder::new("fanout").expect("valid holder"),
                    ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
                    ResourcePageSize::new(1).expect("valid batch"),
                ))
                .await,
            Err(StorageError::Internal(_))
        );
        let state = store.inner.lock();
        let delivery = state
            .deliveries
            .get(&(scope, delivery_id))
            .expect("delivery row remains");
        assert_eq!(
            delivery.last_generation,
            ResourceLeaseGeneration::new(u64::MAX)
        );
        assert!(delivery.claim.is_none());
    }

    #[tokio::test]
    async fn accept_sequence_exhaustion_leaves_no_partial_event_or_delivery() {
        let store = InMemoryResourceRuntime::new();
        let scope = Scope::new("accept-overflow-workspace", "accept-overflow-org");
        let identity = SharedResourceIdentity::new(
            ResourceKind::new("accept-overflow.resource").expect("valid kind"),
            ResourceCompatibilityVersion::new(1),
            ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
                .expect("valid configuration"),
            ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
        );
        let resource_id = match store
            .resolve(ResolveSharedResourceRequest::new(scope.clone(), identity))
            .await
            .expect("resource resolves")
        {
            ResolveSharedResourceOutcome::Created(record)
            | ResolveSharedResourceOutcome::Existing(record) => record.id(),
        };
        store
            .put(PutResourceSubscriptionRequest::new(
                scope.clone(),
                resource_id,
                ResourceConsumerKind::new("workflow-trigger").expect("valid consumer kind"),
                ResourceConsumerIdentity::try_from_vec(b"consumer".to_vec())
                    .expect("valid consumer identity"),
            ))
            .await
            .expect("subscription stores");
        let lease = match store
            .acquire(AcquireResourceSourceLeaseRequest::new(
                scope.clone(),
                resource_id,
                ResourceLeaseHolder::new("source").expect("valid holder"),
                ResourceLeaseTtl::new(Duration::from_secs(30)).expect("valid TTL"),
            ))
            .await
            .expect("source acquires")
        {
            AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
            AcquireResourceSourceLeaseOutcome::Contended { .. } => {
                panic!("fresh source must acquire")
            },
        };
        {
            let mut state = store.inner.lock();
            state.next_delivery_sequence = u64::MAX;
        }

        let result = store
            .accept(AcceptResourceEventRequest::new(
                scope,
                resource_id,
                lease.token().clone(),
                EventOccurrenceNamespace::new("overflow").expect("valid namespace"),
                EventOccurrenceKey::try_from_vec(b"occurrence".to_vec()).expect("valid occurrence"),
                EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
            ))
            .await;

        assert_matches!(result, Err(StorageError::Internal(_)));
        let state = store.inner.lock();
        assert!(state.events.is_empty());
        assert!(state.event_occurrences.is_empty());
        assert!(state.deliveries.is_empty());
        assert_eq!(state.next_delivery_sequence, u64::MAX);
    }
}
