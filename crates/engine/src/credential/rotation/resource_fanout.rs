//! Engine-owned reverse index: `CredentialId` -> affected resource rows.
//!
//! Per ADR-0030, `nebula-engine` (exec layer) owns credential rotation
//! orchestration; `nebula-resource` exposes only the typed
//! `Manager::{refresh_slot, revoke_slot}` port. When a credential rotates,
//! the engine must fan that single event out to every resource registry row
//! whose resolved slot binding consumed it.
//!
//! This module is the index half of that fan-out. It maps a rotated
//! `CredentialId` to the set of resource rows that bound it, so the
//! orchestrator can drive `Manager::{refresh_slot, revoke_slot}` per row.
//!
//! # Why the bind tuple carries `slot_identity`
//!
//! The resource registry is keyed structurally by
//! `(ResourceKey, ScopeLevel, slot_identity)` — see
//! [`nebula_resource::dedup`] and [`nebula_resource::SLOT_IDENTITY_UNBOUND`].
//! Two registrations of the same resource type at the same scope whose
//! resolved credentials differ are *distinct rows* (the multi-tenant
//! anti-bleed barrier). A `Manager::refresh_slot` call against a multi-row
//! `(key, scope)` fails closed (`Ambiguous`) precisely because it cannot pick
//! a row without the resolved identity.
//!
//! The reverse-index entry therefore records the resolved `slot_identity`
//! alongside `(ResourceKey, ScopeLevel, slot_name)` so a rotation routes to
//! the *specific* resolved registry row rather than the whole `(key, scope)`
//! family. This is forward-correctness against the structural dedup model,
//! not extra precision for its own sake.
//!
//! Per ADR-0036 (event-driven cross-crate flow) the engine consumes the
//! credential rotation signal and translates it into typed `Manager` port
//! calls; per ADR-0044 the resource layer never reaches back across the
//! boundary. This index is an in-process, in-memory routing table only —
//! never persisted and never sent across a trust boundary.

use dashmap::DashMap;
use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::CredentialId;

/// One resource registry row affected by a credential rotation.
///
/// `(resource_key, scope, slot_name, slot_identity)`. The trailing
/// `slot_identity` is the resolved structural identity from
/// [`nebula_resource::dedup::slot_identity`]; it disambiguates multi-tenant
/// rows that share `(resource_key, scope)` so a rotation routes to exactly
/// the row whose slot resolved to the rotated credential.
///
/// [`nebula_resource::SLOT_IDENTITY_UNBOUND`] is the identity for a row that
/// resolved no credential slots (single-row-per-`(key, scope)` legacy
/// behaviour); such rows still appear here verbatim.
pub type Bind = (ResourceKey, ScopeLevel, String, u64);

/// Engine-owned reverse index from a rotated `CredentialId` to the resource
/// registry rows that resolved it.
///
/// Concurrency-safe and lock-free for readers via [`DashMap`]; the
/// orchestrator binds rows as resources register and looks them up on a
/// rotation signal. Insert order within a single credential is preserved so
/// fan-out is deterministic for a given registration sequence.
///
/// This is a pure in-process routing table — see the module docs for why it
/// is never persisted or sent across a trust boundary.
#[derive(Debug, Default)]
pub struct ResourceFanoutIndex {
    /// `CredentialId` -> rows whose resolved slot bound that credential.
    ///
    /// `nebula-engine` has no direct `smallvec` dependency, so the
    /// per-credential row list is a plain `Vec`. Promoting this to a small
    /// inline buffer is a deferred, dependency-gated optimisation.
    by_credential: DashMap<CredentialId, Vec<Bind>>,
}

impl ResourceFanoutIndex {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that the resource row
    /// `(resource_key, scope, slot_name, slot_identity)` resolved `cid` for
    /// one of its credential slots.
    ///
    /// Re-binding an identical tuple under the same credential is idempotent
    /// (no duplicate entry) so a resource that re-registers without changing
    /// its resolved binding does not fan out twice.
    pub fn bind(
        &self,
        cid: CredentialId,
        resource_key: ResourceKey,
        scope: ScopeLevel,
        slot_name: impl Into<String>,
        slot_identity: u64,
    ) {
        let entry: Bind = (resource_key, scope, slot_name.into(), slot_identity);
        let mut rows = self.by_credential.entry(cid).or_default();
        if !rows.contains(&entry) {
            rows.push(entry);
        }
    }

    /// Returns every resource row that resolved `cid`, in registration order.
    ///
    /// Empty when no row bound the credential — the orchestrator treats that
    /// as a no-op rotation fan-out.
    #[must_use]
    pub fn affected(&self, cid: &CredentialId) -> Vec<Bind> {
        self.by_credential
            .get(cid)
            .map(|rows| rows.clone())
            .unwrap_or_default()
    }

    /// Drops every binding for the resource row identified by
    /// `(resource_key, scope)` across all credentials.
    ///
    /// Used when a resource registry row is removed and its scope had no
    /// multi-tenant siblings — every slot identity under that
    /// `(key, scope)` goes away. For removing one specific resolved row out
    /// of a multi-tenant `(key, scope)` family, use
    /// [`unbind_resource_identity`](Self::unbind_resource_identity).
    pub fn unbind_resource(&self, resource_key: &ResourceKey, scope: &ScopeLevel) {
        self.by_credential.retain(|_, rows| {
            rows.retain(|(rk, sc, _, _)| rk != resource_key || sc != scope);
            !rows.is_empty()
        });
    }

    /// Drops bindings for the single resolved registry row
    /// `(resource_key, scope, slot_identity)`, leaving multi-tenant siblings
    /// that share `(resource_key, scope)` but differ in `slot_identity`
    /// intact.
    ///
    /// This is the precise inverse of [`bind`](Self::bind) at row
    /// granularity: when one resolved row is removed from a multi-row
    /// `(key, scope)` family, only that row's fan-out entries must go. Kept
    /// alongside [`unbind_resource`](Self::unbind_resource) because the
    /// orchestrator removes a *specific* resolved row on resource removal —
    /// matching the structural dedup model where `(key, scope)` alone is not
    /// a unique row.
    pub fn unbind_resource_identity(
        &self,
        resource_key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: u64,
    ) {
        self.by_credential.retain(|_, rows| {
            rows.retain(|(rk, sc, _, sid)| {
                rk != resource_key || sc != scope || *sid != slot_identity
            });
            !rows.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::WorkflowId;

    use super::*;

    fn cred() -> CredentialId {
        CredentialId::new()
    }

    fn rk(name: &str) -> ResourceKey {
        ResourceKey::new(name).expect("valid resource key")
    }

    fn wf_scope() -> ScopeLevel {
        ScopeLevel::Workflow(WorkflowId::new())
    }

    #[test]
    fn index_bind_lookup_unbind_with_identity() {
        let idx = ResourceFanoutIndex::new();
        let cid = cred();
        let key = rk("pg");
        let scope = wf_scope();
        idx.bind(cid, key.clone(), scope.clone(), "db", 0x1234);
        assert_eq!(
            idx.affected(&cid),
            vec![(key.clone(), scope.clone(), "db".to_string(), 0x1234)]
        );
        idx.unbind_resource(&key, &scope);
        assert!(idx.affected(&cid).is_empty());
    }

    #[test]
    fn distinct_slot_identity_same_resource_are_distinct_binds() {
        // Same ResourceKey + scope, different resolved slot_identity (e.g.
        // two tenants resolving the same resource type to different
        // credentials) MUST be separate entries so the orchestrator routes
        // each rotation to its own resolved registry row.
        let idx = ResourceFanoutIndex::new();
        let key = rk("pg");
        let scope = wf_scope();
        let c1 = cred();
        let c2 = cred();
        idx.bind(c1, key.clone(), scope.clone(), "db", 0xAAAA);
        idx.bind(c2, key.clone(), scope.clone(), "db", 0xBBBB);
        assert_eq!(
            idx.affected(&c1),
            vec![(key.clone(), scope.clone(), "db".into(), 0xAAAA)]
        );
        assert_eq!(idx.affected(&c2), vec![(key, scope, "db".into(), 0xBBBB)]);
    }

    #[test]
    fn rebinding_identical_tuple_is_idempotent() {
        let idx = ResourceFanoutIndex::new();
        let cid = cred();
        let key = rk("pg");
        let scope = wf_scope();
        idx.bind(cid, key.clone(), scope.clone(), "db", 0x1234);
        idx.bind(cid, key, scope, "db", 0x1234);
        assert_eq!(idx.affected(&cid).len(), 1);
    }

    #[test]
    fn unbind_resource_identity_keeps_multi_tenant_siblings() {
        // Two tenants resolve the same (ResourceKey, scope) to different
        // credentials -> two distinct slot identities. Removing one resolved
        // row must NOT collapse the sibling that shares (key, scope).
        let idx = ResourceFanoutIndex::new();
        let key = rk("pg");
        let scope = wf_scope();
        let c1 = cred();
        let c2 = cred();
        idx.bind(c1, key.clone(), scope.clone(), "db", 0xAAAA);
        idx.bind(c2, key.clone(), scope.clone(), "db", 0xBBBB);

        idx.unbind_resource_identity(&key, &scope, 0xAAAA);

        assert!(
            idx.affected(&c1).is_empty(),
            "removed resolved row must be gone"
        );
        assert_eq!(
            idx.affected(&c2),
            vec![(key, scope, "db".into(), 0xBBBB)],
            "sibling sharing (key, scope) but a different identity must survive"
        );
    }
}
