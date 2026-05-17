//! Structural registry-row identity (`DedupKey`).
//!
//! A registered resource's runtime is shared by everyone who resolves the
//! same row. Keying that row by `(ResourceKey, ScopeLevel)` alone is unsafe:
//! two registrations of the same resource type at the same scope whose
//! resolved credentials differ would collapse to one row — one shared
//! topology runtime serving two tenants' credentials (cross-tenant bleed).
//!
//! `DedupKey` folds a **resolved per-slot credential identity** into the row
//! identity so a different resolved credential is a structurally distinct row
//! with its own runtime. The identity is computed from the *resolved* slot
//! bindings (e.g. the bound `CredentialKey` per slot), **independent of the
//! author's [`ResourceConfig::fingerprint()`](crate::ResourceConfig::fingerprint)**
//! — relying on resource authors to override `fingerprint()` to keep tenants
//! apart is a discipline-based defence and is explicitly not the mechanism
//! here. `fingerprint()` remains a hot-reload change-detection token only.
//!
//! The empty binding set hashes to a fixed sentinel
//! ([`SLOT_IDENTITY_UNBOUND`]) so a resource that declares no credential
//! slots — or whose slots are not yet resolved — keeps the historical
//! single-row-per-`(key, scope)` behaviour (the shared-resource dedup
//! invariant).

use std::hash::{Hash, Hasher};

use nebula_core::{ResourceKey, ScopeLevel};

/// Stable slot-identity value for a registration that resolves **no**
/// credential slots (the empty binding set).
///
/// Registrations that carry no resolved slot identity all share this value,
/// so they continue to collapse to a single registry row per
/// `(ResourceKey, ScopeLevel)` — preserving the same-credential
/// shared-resource dedup contract (one `Resource::create` for N acquires).
pub const SLOT_IDENTITY_UNBOUND: u64 = 0;

/// Computes the stable per-registration slot identity from resolved slot
/// bindings.
///
/// `bindings` is an iterator of `(slot_key, resolved_credential_identity)`
/// pairs. The pairs are sorted by slot key before hashing so identity is
/// order-independent (the caller's map iteration order must not change the
/// result). An empty iterator yields [`SLOT_IDENTITY_UNBOUND`].
///
/// Uses the standard-library [`DefaultHasher`](std::collections::hash_map::DefaultHasher)
/// — no new dependency, and the value is only ever compared for equality
/// in-process (never persisted or sent across a trust boundary), so hash
/// stability across toolchains is not required.
pub fn slot_identity<'a, I>(bindings: I) -> u64
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut pairs: Vec<(&str, &str)> = bindings.into_iter().collect();
    if pairs.is_empty() {
        return SLOT_IDENTITY_UNBOUND;
    }
    pairs.sort_unstable();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (slot, id) in &pairs {
        slot.hash(&mut hasher);
        id.hash(&mut hasher);
    }
    let h = hasher.finish();
    // Keep the sentinel reserved for "no resolved slots". The probability of
    // a real binding set hashing to 0 is ~2^-64; if it does, nudge off the
    // reserved value so it cannot be mistaken for "unbound".
    if h == SLOT_IDENTITY_UNBOUND { 1 } else { h }
}

/// Structural identity of a registry row.
///
/// Two registrations collide (last-write-wins replace) **iff** all three
/// components are equal. A different `slot_identity` for the same
/// `(resource_key, scope)` is a *distinct* row with its own
/// `ManagedResource` and topology runtime — this is the structural barrier
/// against cross-tenant runtime bleed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey {
    /// The static, type-level resource key (`R::key()`).
    pub resource_key: ResourceKey,
    /// The scope the resource was registered at.
    pub scope: ScopeLevel,
    /// Stable hash over the resolved per-slot credential bindings.
    /// [`SLOT_IDENTITY_UNBOUND`] when no slots are resolved.
    pub slot_identity: u64,
}

impl DedupKey {
    /// Builds a key from its parts.
    pub fn new(resource_key: ResourceKey, scope: ScopeLevel, slot_identity: u64) -> Self {
        Self {
            resource_key,
            scope,
            slot_identity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bindings_are_unbound_sentinel() {
        let empty: Vec<(&str, &str)> = Vec::new();
        assert_eq!(slot_identity(empty), SLOT_IDENTITY_UNBOUND);
    }

    #[test]
    fn order_independent() {
        let a = slot_identity([("db", "cred-1"), ("cache", "cred-2")]);
        let b = slot_identity([("cache", "cred-2"), ("db", "cred-1")]);
        assert_eq!(a, b, "slot identity must not depend on iteration order");
    }

    #[test]
    fn different_resolved_credential_differs() {
        let a = slot_identity([("db", "cred-tenant-a")]);
        let b = slot_identity([("db", "cred-tenant-b")]);
        assert_ne!(
            a, b,
            "different resolved credential for the same slot must yield a \
             different identity"
        );
    }

    #[test]
    fn same_resolved_credential_matches() {
        let a = slot_identity([("db", "cred-x")]);
        let b = slot_identity([("db", "cred-x")]);
        assert_eq!(a, b);
    }

    #[test]
    fn non_empty_binding_never_collides_with_unbound() {
        // A real binding must never be mistaken for "no resolved slots".
        let id = slot_identity([("db", "cred-x")]);
        assert_ne!(id, SLOT_IDENTITY_UNBOUND);
    }
}
