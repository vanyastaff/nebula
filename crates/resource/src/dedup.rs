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
//! digest equality (the historical [`slot_identity`] primitive) is a
//! collidable space: two registrations whose resolved credentials differ
//! but whose digests collide would silently merge into one registry row,
//! bypassing the fail-closed ambiguity deny. The structural set eliminates
//! that class rather than shrinking it; an incomplete/lossy connection-pool
//! key is a known cross-tenant-leak anti-pattern, so the full resolved set
//! is the correct row-key shape.
//!
//! ## Legacy `u64` bridge
//!
//! The historical surface threads a `u64` slot-identity value end to end
//! (`RegisterOptions::with_slot_identity`, `acquire_erased`, the engine's
//! recomputed value). That value maps into [`SlotIdentity::Opaque`] so the
//! pre-existing surface keeps compiling and preserves its exact
//! `u64`-equality semantics during the migration — the structural path is
//! *added alongside*, the `Opaque` bridge is removed once every consumer is
//! migrated. An [`SlotIdentity::Opaque`] can never compare equal to a
//! [`SlotIdentity::Structural`] or [`SlotIdentity::Unbound`], so the two
//! identity spaces stay disjoint.
//!
//! The empty binding set is [`SlotIdentity::Unbound`] (the historical
//! [`SLOT_IDENTITY_UNBOUND`] sentinel) so a resource that declares no
//! credential slots — or whose slots are not yet resolved — keeps the
//! historical single-row-per-`(key, scope)` behaviour (the shared-resource
//! dedup invariant).

use std::{hash::Hash, sync::Arc};

use nebula_core::{ResourceKey, ScopeLevel};

/// Stable slot-identity value for a registration that resolves **no**
/// credential slots (the empty binding set), on the legacy `u64` surface.
///
/// Registrations that carry no resolved slot identity all share this value,
/// so they continue to collapse to a single registry row per
/// `(ResourceKey, ScopeLevel)` — preserving the same-credential
/// shared-resource dedup contract (one `Resource::create` for N acquires).
/// [`SlotIdentity::from_opaque`] maps this value to
/// [`SlotIdentity::Unbound`].
pub const SLOT_IDENTITY_UNBOUND: u64 = 0;

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
    /// No resolved slots (or slots not yet resolved). Equivalent to the
    /// legacy [`SLOT_IDENTITY_UNBOUND`] sentinel — keeps the historical
    /// single-row-per-`(key, scope)` dedup behaviour.
    Unbound,
    /// The resolved `(slot, credential)` pairs, canonical-sorted and
    /// de-duplicated. Equality/hash is over the exact pair list, so a
    /// distinct resolved credential is a distinct identity by construction
    /// (collision-free — not a hash).
    Structural(Arc<[(String, String)]>),
    /// Legacy `u64` slot-identity bridge.
    ///
    /// Carries a value computed by the historical [`slot_identity`]
    /// primitive (still threaded by unmigrated callers / the engine). It
    /// preserves the pre-existing `u64`-equality semantics for those
    /// callers but is **never equal** to a [`SlotIdentity::Structural`] or
    /// [`SlotIdentity::Unbound`], so the legacy and structural identity
    /// spaces are disjoint. Removed once every consumer is migrated to the
    /// structural form.
    Opaque(u64),
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

    /// Maps a legacy `u64` slot-identity value onto a [`SlotIdentity`].
    ///
    /// [`SLOT_IDENTITY_UNBOUND`] becomes [`SlotIdentity::Unbound`] so the
    /// legacy "no resolved slots" sentinel keeps collapsing to the shared
    /// row; every other value becomes [`SlotIdentity::Opaque`], preserving
    /// the pre-existing `u64`-equality semantics for unmigrated callers.
    #[must_use]
    pub fn from_opaque(value: u64) -> Self {
        if value == SLOT_IDENTITY_UNBOUND {
            Self::Unbound
        } else {
            Self::Opaque(value)
        }
    }

    /// `true` for [`SlotIdentity::Unbound`] — the no-resolved-slots row that
    /// keeps the historical single-row-per-`(key, scope)` dedup behaviour.
    #[must_use]
    pub fn is_unbound(&self) -> bool {
        matches!(self, Self::Unbound)
    }
}

/// Computes the stable per-registration slot identity from resolved slot
/// bindings (legacy `u64` form).
///
/// `bindings` is an iterator of `(slot_key, resolved_credential_identity)`
/// pairs. The pairs are sorted by slot key before hashing so identity is
/// order-independent (the caller's map iteration order must not change the
/// result). An empty iterator yields [`SLOT_IDENTITY_UNBOUND`].
///
/// Uses the standard-library [`DefaultHasher`](std::collections::hash_map::DefaultHasher)
/// — the value is only ever compared for equality in-process (never
/// persisted or sent across a trust boundary), so hash stability across
/// toolchains is not required.
///
/// This is the historical collidable primitive. New code derives the
/// collision-free [`SlotIdentity::from_bindings`] instead. It is hidden
/// from the public API surface (so it is not a discoverable extension
/// point for new cross-crate callers) and retained only to keep the
/// not-yet-migrated `u64` callers — including the engine's independent
/// recompute — compiling until those consumers move to the structural
/// form, after which it is removed.
#[doc(hidden)]
pub fn slot_identity<'a, I>(bindings: I) -> u64
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    use std::hash::Hasher as _;

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
    fn structural_never_equals_opaque_or_unbound() {
        let structural = SlotIdentity::from_bindings([("db", "cred-x")]);
        assert_ne!(structural, SlotIdentity::Unbound);
        assert_ne!(structural, SlotIdentity::Opaque(1));
        // Even if a digest of the same bindings happened to be some u64,
        // the structural value is in a disjoint space from `Opaque`.
        let digest = slot_identity([("db", "cred-x")]);
        assert_ne!(structural, SlotIdentity::Opaque(digest));
    }

    #[test]
    fn opaque_bridge_preserves_u64_equality_and_unbound() {
        assert_eq!(
            SlotIdentity::from_opaque(SLOT_IDENTITY_UNBOUND),
            SlotIdentity::Unbound
        );
        assert_eq!(SlotIdentity::from_opaque(7), SlotIdentity::Opaque(7));
        assert_eq!(
            SlotIdentity::from_opaque(7),
            SlotIdentity::from_opaque(7),
            "equal legacy u64 values stay equal under the bridge"
        );
        assert_ne!(
            SlotIdentity::from_opaque(7),
            SlotIdentity::from_opaque(8),
            "distinct legacy u64 values stay distinct under the bridge"
        );
    }
}
