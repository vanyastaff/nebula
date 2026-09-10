use nebula_storage_port::store::*;

#[expect(clippy::too_many_arguments)]
fn _assert_object_safe(
    _a: &dyn ExecutionStore,
    _b: &dyn ExecutionJournalReader,
    _c: &dyn NodeResultStore,
    _d: &dyn CheckpointStore,
    _e: &dyn IdempotencyGuard,
    _f: &dyn IdempotencyStore,
    _g: &dyn WorkflowStore,
    _h: &dyn WorkflowVersionStore,
    _i: &dyn ControlQueue,
    _j: &dyn WebhookActivationStore,
    _k: &dyn RefreshClaimStore,
    _l: &dyn ResumeTokenStore,
) {
}

// Runtime authority crosses these object-safe ports through durable handoff, start
// materialization and effect ledger, and the engine holds every one of them
// as `Arc<dyn _>`, so dyn-compatibility is a contract and not an accident.
fn _assert_runtime_authority_object_safe(
    _a: &dyn ExecutionTurnHandoff,
    _b: &dyn StartAcceptanceStore,
    _c: &dyn JobDispatchQueue,
    _d: &dyn OperationLedger,
    _e: &dyn OperationLedgerAdjudicator,
    _f: &dyn TurnRecovery,
    _g: &dyn StartReservationMaintenance,
) {
}

fn _assert_resource_runtime_object_safe(
    _shared_resources: &dyn SharedResourceStore,
    _subscriptions: &dyn ResourceSubscriptionStore,
    _source_leases: &dyn ResourceSourceLeaseStore,
    _event_fanout: &dyn ResourceEventFanoutStore,
    _execution_handoffs: &dyn ResourceExecutionHandoffStore,
) {
}

// Compile-time object-safety probe over the identity zoo: it is never
// called, so the argument count is not an ergonomics concern.
#[expect(clippy::too_many_arguments)]
fn _assert_identity_object_safe(
    _a: &dyn UserStore,
    _b: &dyn OrgStore,
    _c: &dyn WorkspaceStore,
    _d: &dyn MembershipStore,
    _e: &dyn ResourceStore,
    _f: &dyn TriggerStore,
    _g: &dyn QuotaStore,
    _h: &dyn AuditStore,
    _i: &dyn BlobStore,
) {
}

#[test]
fn traits_are_object_safe() {
    // Compiling `&dyn Trait` for every port trait above proves the whole
    // family is dyn-compatible — the contract the engine/api rely on.
}
