//! Structural registry-row identity (`DedupKey` / [`SlotIdentity`]).
//!
//! A registered resource's runtime is shared by everyone who resolves the
//! same row. Keying that row by `(ResourceKey, ScopeLevel)` alone is unsafe:
//! two registrations of the same resource type at the same scope whose
//! resolved credentials differ would collapse to one row — one shared
//! topology runtime serving two tenants' credentials (cross-tenant bleed).
//!
//! `DedupKey` folds a **resolved per-slot credential identity**
//! ([`SlotIdentity`]) into the row identity so a different resolved
//! credential is a structurally distinct row with its own topology runtime.
//! The identity is computed from the *resolved* slot bindings (the bound
//! credential per slot), **independent of the author's
//! [`ResourceConfig::fingerprint()`](crate::ResourceConfig::fingerprint)**
//! — relying on resource authors to override `fingerprint()` to keep
//! tenants apart is a discipline-based defence and is explicitly not the
//! mechanism here. `fingerprint()` remains a hot-reload change-detection
//! token only.
//!
//! ## Why a structural set, not a digest
//!
//! [`SlotIdentity::Structural`] carries the **ordered, canonical-sorted
//! resolved `(slot, credential)` pairs verbatim** and derives `Eq`/`Hash`
//! over them. Two structural identities are equal **iff** their sorted pair
//! lists are byte-for-byte equal — collision is impossible *by
//! construction* (exact string equality), not merely improbable. A 64-bit
//! digest equality would be a collidable space: two registrations whose
//! resolved credentials differ but whose digests collide would silently
//! merge into one registry row, bypassing the fail-closed ambiguity deny.
//! The structural set eliminates that class rather than shrinking it; an
//! incomplete/lossy connection-pool key is a known cross-tenant-leak
//! anti-pattern, so the full resolved set is the correct row-key shape.
//!
//! The empty binding set is [`SlotIdentity::Unbound`] so a resource that
//! declares no credential slots — or whose slots are not yet resolved —
//! keeps the single-row-per-`(key, scope)` behaviour (the shared-resource
//! dedup invariant).

use std::{hash::Hash, sync::Arc};

use nebula_core::{ResourceKey, ScopeLevel};

/// Resolved per-slot credential identity of a registry row.
///
/// This is the slot component of [`DedupKey`] and the registry's row-key.
/// Two rows at the same `(ResourceKey, ScopeLevel)` collide
/// (last-write-wins replace) **iff** their `SlotIdentity` is equal; a
/// different `SlotIdentity` is a *distinct* row with its own runtime — the
/// structural barrier against cross-tenant runtime bleed.
///
/// Equality and hashing are *exact and structural*: there is no digest, so
/// two distinct resolved binding sets can never alias.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SlotIdentity {
    /// No resolved slots (or slots not yet resolved). Keeps the
    /// single-row-per-`(key, scope)` dedup behaviour (the shared-resource
    /// dedup invariant: one `Resource::create` for N acquires).
    Unbound,
    /// The resolved `(slot, credential)` pairs, canonical-sorted and
    /// de-duplicated. Equality/hash is over the exact pair list, so a
    /// distinct resolved credential is a distinct identity by construction
    /// (collision-free — not a hash).
    Structural(Arc<[(String, String)]>),
}

impl SlotIdentity {
    /// Builds a structural identity from resolved `(slot, credential)`
    /// pairs.
    ///
    /// The pairs are canonical-sorted (ascending by `(slot, credential)`)
    /// and de-duplicated, so the identity is independent of the caller's
    /// iteration order and of duplicate `(slot, credential)` entries — a
    /// given resolved binding set always yields the byte-identical
    /// `Structural` value. An empty input yields [`SlotIdentity::Unbound`]
    /// (the no-resolved-slots case), never an empty `Structural`.
    ///
    /// This is the canonical wire form: any independent recomputation (the
    /// engine recomputes the same identity from the same `(slot,
    /// credential)` pairs) must construct it through this function so the
    /// two sides derive byte-identical keys.
    pub fn from_bindings<'a, I>(bindings: I) -> Self
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut pairs: Vec<(String, String)> = bindings
            .into_iter()
            .map(|(slot, cred)| (slot.to_owned(), cred.to_owned()))
            .collect();
        if pairs.is_empty() {
            return Self::Unbound;
        }
        pairs.sort_unstable();
        pairs.dedup();
        Self::Structural(Arc::from(pairs))
    }

    /// `true` for [`SlotIdentity::Unbound`] — the no-resolved-slots row that
    /// keeps the single-row-per-`(key, scope)` dedup behaviour.
    #[must_use]
    pub fn is_unbound(&self) -> bool {
        matches!(self, Self::Unbound)
    }
}

/// Structural identity of a registry row.
///
/// Two registrations collide (last-write-wins replace) **iff** all three
/// components are equal. A different [`SlotIdentity`] for the same
/// `(resource_key, scope)` is a *distinct* row with its own
/// `ManagedResource` and topology runtime — this is the structural barrier
/// against cross-tenant runtime bleed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey {
    /// The static, type-level resource key (`R::key()`).
    pub resource_key: ResourceKey,
    /// The scope the resource was registered at.
    pub scope: ScopeLevel,
    /// The resolved per-slot credential identity. [`SlotIdentity::Unbound`]
    /// when no slots are resolved.
    pub slot_identity: SlotIdentity,
}

impl DedupKey {
    /// Builds a key from its parts.
    pub fn new(resource_key: ResourceKey, scope: ScopeLevel, slot_identity: SlotIdentity) -> Self {
        Self {
            resource_key,
            scope,
            slot_identity,
        }
    }
}

// The legacy `slot_identity` u64 primitive + `from_opaque` +
// `SLOT_IDENTITY_UNBOUND` + `SlotIdentity::Opaque` were removed (R15: the
// collidable digest space is eliminated, not shrunk); the structural
// identity is the sole row-key shape.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_empty_is_unbound() {
        let empty: Vec<(&str, &str)> = Vec::new();
        assert_eq!(SlotIdentity::from_bindings(empty), SlotIdentity::Unbound);
        assert!(SlotIdentity::from_bindings(Vec::<(&str, &str)>::new()).is_unbound());
    }

    #[test]
    fn structural_is_order_and_dup_independent() {
        let a = SlotIdentity::from_bindings([("db", "cred-1"), ("cache", "cred-2")]);
        let b = SlotIdentity::from_bindings([("cache", "cred-2"), ("db", "cred-1")]);
        let c =
            SlotIdentity::from_bindings([("cache", "cred-2"), ("db", "cred-1"), ("db", "cred-1")]);
        assert_eq!(a, b, "canonical-sorted identity must be order-independent");
        assert_eq!(a, c, "duplicate pairs must not change the identity");
    }

    #[test]
    fn structural_distinct_credentials_never_equal() {
        // The collision-free guarantee: distinct resolved credentials are
        // distinct identities by construction (exact equality, no digest).
        let a = SlotIdentity::from_bindings([("db", "cred-tenant-a")]);
        let b = SlotIdentity::from_bindings([("db", "cred-tenant-b")]);
        assert_ne!(a, b);
    }

    #[test]
    fn structural_never_equals_unbound() {
        let structural = SlotIdentity::from_bindings([("db", "cred-x")]);
        assert_ne!(structural, SlotIdentity::Unbound);
    }
}
