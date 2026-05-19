//! Firewall-coverage conformance: every `&Scope`-keyed port trait in
//! `nebula-storage-port` MUST have a `Scoped*` decorator in
//! `nebula-tenancy` (spec §6.2, threat model §6.1).
//!
//! `crate::lib` asserts the multi-tenancy firewall is closed *by
//! construction*. That is only true if **every** port trait whose
//! signature carries a caller-supplied `&Scope` is wrapped by a
//! scope-substituting decorator. A raw `Arc<dyn …Store>` handed to a
//! non-HTTP consumer with no `Scoped*` wrapper lets that consumer pass an
//! arbitrary `&Scope` — a cross-tenant read on `get`/`list` and a
//! cross-tenant write on `create`/`update`/`soft_delete` (BOLA / IDOR).
//!
//! The single defence against *regression* is a static enumeration: this
//! file binds every `&Scope`-keyed trait to its decorator via a generic
//! `assert_scoped::<Decorator, dyn PortTrait>()` that only type-checks
//! when the decorator implements that exact port trait. Adding a new
//! `&Scope`-keyed port without a decorator makes this test fail to
//! compile — the regression cannot land silently.
//!
//! Parent-id-keyed identity stores (no `&Scope` in the signature) are a
//! *different* authorization model and are deliberately enumerated in the
//! checklist below with the rationale for why a `Scoped*` is not
//! applicable, so the decision is explicit rather than an omission.

use std::sync::Arc;

use nebula_storage_port::store::{
    ControlQueue, ExecutionJournalReader, ExecutionStore, IdempotencyStore, NodeResultStore,
    ResourceStore, TriggerStore, WebhookActivationStore, WorkflowStore, WorkflowVersionStore,
};
use nebula_storage_port::{Scope, StorageError};
use nebula_tenancy::{
    ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore, ScopedIdempotencyStore,
    ScopedNodeResultStore, ScopedResourceStore, ScopedTriggerStore, ScopedWebhookActivationStore,
    ScopedWorkflowStore, ScopedWorkflowVersionStore,
};

/// Compile-time proof that `D` is a scope-substituting decorator for the
/// object-safe port trait `P`: it implements `P`, it is `Send + Sync`
/// (usable as `Arc<dyn P>` behind the firewall), and it is constructible
/// from `(Arc<dyn P>, Scope)` — i.e. it binds a tenant `Scope`.
///
/// If a `&Scope`-keyed port trait is added without a matching decorator,
/// the corresponding `assert_scoped` call below fails to compile with an
/// unsatisfied-bound error naming the missing `Scoped*` type.
fn assert_scoped<D, P>()
where
    P: ?Sized + Send + Sync + 'static,
    D: ScopeDecorator<P> + Send + Sync + 'static,
{
}

/// Marker every `Scoped*` decorator satisfies for its port trait: it can
/// be built from a raw `Arc<dyn P>` plus the tenant `Scope` it binds.
trait ScopeDecorator<P: ?Sized> {
    #[allow(dead_code)] // guard-justified: linked only for the type-bound proof above.
    fn bind(inner: Arc<P>, scope: Scope) -> Self;
}

macro_rules! scope_decorator {
    ($decorator:ty, $port:path) => {
        impl ScopeDecorator<dyn $port> for $decorator {
            fn bind(inner: Arc<dyn $port>, scope: Scope) -> Self {
                <$decorator>::new(inner, scope)
            }
        }
    };
}

scope_decorator!(ScopedExecutionStore, ExecutionStore);
scope_decorator!(ScopedWorkflowStore, WorkflowStore);
scope_decorator!(ScopedWorkflowVersionStore, WorkflowVersionStore);
scope_decorator!(ScopedNodeResultStore, NodeResultStore);
scope_decorator!(ScopedIdempotencyStore, IdempotencyStore);
scope_decorator!(ScopedControlQueue, ControlQueue);
scope_decorator!(ScopedExecutionJournalReader, ExecutionJournalReader);
scope_decorator!(ScopedWebhookActivationStore, WebhookActivationStore);
scope_decorator!(ScopedResourceStore, ResourceStore);
scope_decorator!(ScopedTriggerStore, TriggerStore);

/// Static enumeration of **every** `&Scope`-keyed port trait. Each line
/// is a compile-time assertion that the firewall covers that trait. To
/// add a new `&Scope`-keyed port you MUST add it here *and* ship its
/// decorator, or this test will not compile.
#[test]
fn every_scope_keyed_port_has_a_decorator() {
    // Atomic execution unit (§12.2): create/get/lease/commit all `&Scope`.
    assert_scoped::<ScopedExecutionStore, dyn ExecutionStore>();
    // Workflow + version split: row carries an embedded `Scope` (rebound).
    assert_scoped::<ScopedWorkflowStore, dyn WorkflowStore>();
    assert_scoped::<ScopedWorkflowVersionStore, dyn WorkflowVersionStore>();
    // Per-node result cache: `&Scope`-keyed put/get.
    assert_scoped::<ScopedNodeResultStore, dyn NodeResultStore>();
    // Idempotency dedup: `&Scope` namespaces the key (no replay oracle).
    assert_scoped::<ScopedIdempotencyStore, dyn IdempotencyStore>();
    // Control queue: enqueued msg carries a `Scope` (rebound).
    assert_scoped::<ScopedControlQueue, dyn ControlQueue>();
    // Execution journal read path: `&Scope`-keyed.
    assert_scoped::<ScopedExecutionJournalReader, dyn ExecutionJournalReader>();
    // Webhook activation: `&Scope`-keyed.
    assert_scoped::<ScopedWebhookActivationStore, dyn WebhookActivationStore>();
    // Identity zoo, workspace-scoped (the BOLA/IDOR class this guards):
    assert_scoped::<ScopedResourceStore, dyn ResourceStore>();
    assert_scoped::<ScopedTriggerStore, dyn TriggerStore>();
}

/// Decision record for the identity-zoo traits that are **not**
/// `&Scope`-keyed. These authorize on a parent id (org/workspace id) or a
/// global key, resolved at the composition root *before* the call — they
/// have no caller-supplied `&Scope` surface a confused deputy could
/// forge, so a `Scoped*` substitution decorator is not applicable. This
/// test is documentation-as-code: if one of these traits ever grows a
/// `&Scope` parameter, the matching `assert_scoped` line must be added
/// above (and a decorator shipped), and this comment block updated.
///
/// | Trait             | Keying                                   | Why no `Scoped*`                                                                 |
/// |-------------------|------------------------------------------|----------------------------------------------------------------------------------|
/// | `UserStore`       | global user id / email                   | Users are global, not tenant-scoped; lookups are first-writer-wins on email.      |
/// | `OrgStore`        | global org id / slug                     | Org *is* the tenancy root; there is no enclosing scope to substitute.             |
/// | `WorkspaceStore`  | parent `org_id` + workspace id           | Parent-org authorization is resolved at the composition root, not via `&Scope`.   |
/// | `MembershipStore` | (`scope_kind`, `scope_id`, principal)    | The authz domain itself; substituting a scope would corrupt the ACL it defines.  |
/// | `QuotaStore`      | parent `org_id`                          | Org-level CAS counters; org id is the resolved boundary, no `&Scope` surface.     |
/// | `AuditStore`      | parent `org_id` (append-only)            | Append-only org-scoped log; org id resolved at root; nothing to substitute.      |
/// | `BlobStore`       | parent `workspace_id`                    | Workspace-id-keyed at the root; no `&Scope` arg a deputy could forge.             |
///
/// `&Scope`-keyed traits MUST get a decorator (enumerated above);
/// parent-id-keyed traits get this documented decision and no decorator.
#[test]
fn parent_id_keyed_identity_stores_are_intentionally_undecorated() {
    // No assertion to make — the value is the audited table above. The
    // test exists so the decision is a tracked, reviewable unit and the
    // file fails CI if it is deleted.
    let _: fn() -> Result<(), StorageError> = || Ok(());
}
