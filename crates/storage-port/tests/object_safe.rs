use nebula_storage_port::store::*;

#[allow(clippy::too_many_arguments)]
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

// Compile-time object-safety probe over the identity zoo: it is never
// called, so the argument count is not an ergonomics concern.
#[allow(clippy::too_many_arguments)]
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
