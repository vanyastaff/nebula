//! Reverse index: `CredentialId` → affected resource rows.
//!
//! When a credential rotates, the engine must fan that single event out to
//! every resource registry row whose resolved slot binding consumed it.
//!
//! This module is the index half of that fan-out. It maps a rotated
//! `CredentialId` to the set of resource rows that bound it, so the
//! orchestrator can drive `Manager::{refresh_slot_for, revoke_slot_for}` per
//! row.
//!
//! # Why the bind struct carries `slot_identity`
//!
//! The resource registry is keyed structurally by
//! `(ResourceKey, ScopeLevel, slot_identity)` — see
//! [`crate::dedup`] and [`crate::SlotIdentity`]. Two
//! registrations of the same resource type at the same scope whose
//! resolved credentials differ are *distinct rows* (the multi-tenant
//! anti-bleed barrier). A `Manager::refresh_slot` call against a multi-row
//! `(key, scope)` fails closed (`Ambiguous`) precisely because it cannot pick
//! a row without the resolved identity.
//!
//! The reverse-index entry therefore records the resolved
//! [`SlotIdentity`] alongside
//! `(ResourceKey, ScopeLevel, slot_name)` so a rotation routes to the
//! *specific* resolved registry row rather than the whole `(key, scope)`
//! family. The identity is the **collision-free structural** value
//! (`SlotIdentity`, exact string equality over the canonical-sorted
//! resolved `(slot, credential)` pairs — *not* a collidable digest), so the
//! reverse-index key cannot alias two tenants' rows. This is
//! forward-correctness against the structural dedup model, not extra
//! precision for its own sake.
//!
//! The engine consumes each credential rotation signal and translates it into typed `Manager` port
//! calls; the resource layer never reaches back across the boundary. This index is an in-process,
//! in-memory routing table only — never persisted and never sent across a trust boundary.

use dashmap::DashMap;
use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::CredentialId;
use smallvec::SmallVec;

use crate::SlotIdentity;

/// One resource registry row affected by a credential rotation.
///
/// - `resource_key` / `scope`: the structural address of the registry row.
/// - `slot_name`: the credential slot on that row that resolved the rotated
///   credential.
/// - `slot_identity`: the resolved **collision-free structural** identity
///   ([`SlotIdentity`]); it disambiguates
///   multi-tenant rows that share `(resource_key, scope)` so a rotation
///   routes to exactly the row whose slot resolved to the rotated
///   credential. Equality is exact string equality (no digest), so two
///   distinct resolved binding sets can never alias this reverse-index key.
///
/// `SlotIdentity::Unbound` is the `slot_identity` for a row that resolved
/// no credential slots (single-row-per-`(key, scope)` behaviour); such rows
/// still appear here verbatim.
///
/// Fields are named (rather than a positional tuple) so call sites that
/// destructure a bind cannot transpose `resource_key`/`scope`/`slot_name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    /// Structural key of the affected resource registry row.
    pub resource_key: ResourceKey,
    /// Lifecycle scope of the affected resource registry row.
    pub scope: ScopeLevel,
    /// Credential slot on the row that resolved the rotated credential.
    pub slot_name: String,
    /// Resolved **collision-free structural** slot identity disambiguating
    /// multi-tenant rows (exact string equality, not a collidable digest).
    pub slot_identity: SlotIdentity,
}

/// Aggregate of a per-slot rotation fan-out across every affected resource
/// registry row.
///
/// One [`Bind`] contributes exactly one count, so
/// `success + failed + timed_out + deferred + abandoned == affected_rows`.
/// `drain_timed_out` is orthogonal and may overlap any of the five dispatch
/// outcomes,
/// while `observation_timed_out` is orthogonal to terminal execution and
/// overlaps `deferred`; both are excluded from [`dispatched`](Self::dispatched). Per-resource
/// timeout-isolation invariant: a slow, failed, or timed-out row never
/// aborts or fails its siblings — each row's outcome is independent. The
/// struct carries only counts (no key/slot/credential material) so it is safe
/// to log or emit as a metrics/dashboard signal; it is **not** a substitute
/// for an audit write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use = "fan-out outcome counts must be observed and accounted for"]
#[non_exhaustive]
pub struct RotationOutcome {
    /// Rows whose `Manager::{refresh,revoke}_slot_for` hook returned `Ok`.
    pub(super) success: usize,
    /// Rows whose hook returned `Err` (resolution miss, hook failure, …).
    pub(super) failed: usize,
    /// Rows whose hook did not complete within the per-resource timeout.
    pub(super) timed_out: usize,
    /// Rows whose hook was accepted but remains queue-owned and unobserved.
    pub(super) deferred: usize,
    /// Rows whose admitted queue task terminated without a hook result.
    pub(super) abandoned: usize,
    /// Revoke rows whose best-effort lease drain timed out before hook
    /// submission. Orthogonal to the terminal outcome counts.
    pub(super) drain_timed_out: usize,
    /// Rows whose accepted hook had not started when the caller's
    /// post-admission observation budget elapsed.
    pub(super) observation_timed_out: usize,
}

impl RotationOutcome {
    /// Rows whose resource hook completed successfully.
    #[must_use]
    pub fn success(&self) -> usize {
        self.success
    }

    /// Rows whose resource hook failed.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.failed
    }

    /// Rows whose resource dispatch exceeded its timeout budget.
    #[must_use]
    pub fn timed_out(&self) -> usize {
        self.timed_out
    }

    /// Rows whose hook work is accepted and queue-owned but not observed to
    /// completion by this fan-out.
    #[must_use]
    pub fn deferred(&self) -> usize {
        self.deferred
    }

    /// Rows whose admitted queue task terminated without a hook result.
    #[must_use]
    pub fn abandoned(&self) -> usize {
        self.abandoned
    }

    /// Revoke rows whose best-effort lease drain timed out. This may overlap
    /// any dispatched hook outcome and is excluded from [`dispatched`](Self::dispatched).
    #[must_use]
    pub fn drain_timed_out(&self) -> usize {
        self.drain_timed_out
    }

    /// Rows whose post-admission observation budget elapsed before hook
    /// execution started.
    #[must_use]
    pub fn observation_timed_out(&self) -> usize {
        self.observation_timed_out
    }

    /// Total rows the fan-out dispatched to
    /// (`success + failed + timed_out + deferred + abandoned`).
    #[must_use]
    pub fn dispatched(&self) -> usize {
        self.success + self.failed + self.timed_out + self.deferred + self.abandoned
    }
}

/// One reverse-index row plus the number of live registrations that
/// resolved it.
///
/// Identical resolved rows dedupe to a single fan-out target (one
/// [`Bind`]); `refs` counts how many `register_and_bind` stagings
/// currently depend on it. A failed registration releases exactly one
/// reference; the row is removed only when the last referent is gone, so
/// a failing staging can never delete a row a concurrent successful
/// registration still holds. `refs` is `>= 1` for any present entry (the
/// entry is removed at zero), so a plain `usize` with that invariant is
/// sufficient — no `NonZero` ceremony.
#[derive(Debug, Clone)]
struct BindRef {
    bind: Bind,
    refs: usize,
}

/// Per-credential row list. Most credentials resolve into one or two
/// resource rows, so two rows live inline in the map entry (no heap
/// allocation, better locality on the rotation fan-out read); larger
/// families spill to the heap transparently.
type BindRows = SmallVec<[BindRef; 2]>;

/// Reverse index from a rotated `CredentialId` to the resource registry
/// rows that resolved it.
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
    /// `CredentialId` -> refcounted rows whose resolved slot bound that
    /// credential. See [`BindRows`] for the inline-buffer rationale.
    ///
    /// Deliberately **one-way**: the unbind paths scan-and-`retain` across
    /// credentials (O(total binds), average O(rows-per-credential) per
    /// bucket) instead of keeping a resource→credential reverse map. Unbind
    /// runs only on registration removal (cold), while the reverse map would
    /// buy that cold path speed at the price of a two-map consistency
    /// protocol on every bind/rollback (the TOCTOU discipline below relies
    /// on a single shard lock). Rotation fan-out — the hot read — is already
    /// a single-key lookup.
    by_credential: DashMap<CredentialId, BindRows>,
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
    /// Re-binding an identical row under the same credential is idempotent
    /// *at the fan-out level* — [`affected`](Self::affected) still returns
    /// one entry and a rotation fans out once — but each call takes a
    /// reference (see `BindRef`). The presence check and the
    /// increment/insert both run under the `DashMap` shard lock held by
    /// `entry(cid)`, so a concurrent `bind` / `unbind_staged_entry` for the
    /// same `cid` cannot interleave between them. This closes the
    /// stage-then-roll-back TOCTOU in
    /// `ResourceActivatorRegistry::register_and_bind`:
    /// a failing registration releases only its own reference and can
    /// never delete a row a concurrent successful registration still holds.
    pub fn bind(
        &self,
        cid: CredentialId,
        resource_key: ResourceKey,
        scope: ScopeLevel,
        slot_name: impl Into<String>,
        slot_identity: SlotIdentity,
    ) {
        let entry = Bind {
            resource_key,
            scope,
            slot_name: slot_name.into(),
            slot_identity,
        };
        let mut rows = self.by_credential.entry(cid).or_default();
        match rows.iter_mut().find(|r| r.bind == entry) {
            Some(existing) => existing.refs += 1,
            None => rows.push(BindRef {
                bind: entry,
                refs: 1,
            }),
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
            .map(|rows| rows.iter().map(|r| r.bind.clone()).collect())
            .unwrap_or_default()
    }

    /// Removes every binding under `(resource_key, scope)` across all
    /// `slot_identity` values and all credentials.
    ///
    /// This is unconditional: it does not preserve multi-tenant siblings that
    /// share `(resource_key, scope)` but differ in `slot_identity`. For
    /// row-granular removal that keeps such siblings intact, use
    /// `unbind_resource_identity`.
    pub fn unbind_resource(&self, resource_key: &ResourceKey, scope: &ScopeLevel) {
        self.by_credential.retain(|_, rows| {
            rows.retain(|r| r.bind.resource_key != *resource_key || r.bind.scope != *scope);
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
        slot_identity: &SlotIdentity,
    ) {
        self.by_credential.retain(|_, rows| {
            rows.retain(|r| {
                r.bind.resource_key != *resource_key
                    || r.bind.scope != *scope
                    || r.bind.slot_identity != *slot_identity
            });
            !rows.is_empty()
        });
    }

    /// Removes exactly one `(cid, bind)` tuple — the precise per-entry
    /// inverse of a single [`bind`](Self::bind) call.
    ///
    /// Unlike [`unbind_resource_identity`](Self::unbind_resource_identity)
    /// (which drops *every* credential's binding for a
    /// `(resource_key, scope, slot_identity)` row), this removes only the
    /// one entry under `cid` that structurally equals `bind`, leaving any
    /// other credential's binding for the same resolved row — and any
    /// pre-existing identical binding under a different cid — untouched.
    ///
    /// It is the compensation primitive for the *stage-bind-before-
    /// register-then-roll-back-on-failure* ordering in
    /// `ResourceActivatorRegistry::register_and_bind`: it **releases one
    /// reference** taken by [`bind`](Self::bind) and removes the row only
    /// when the last referent is gone. A registration that fails after
    /// staging therefore drops just its own reference; a concurrent (or
    /// prior) successful registration of the identical resolved row keeps
    /// its reference, so its live fan-out row survives. This makes the
    /// rollback correct without the registrar having to decide "did I
    /// insert this entry?" — a decision that could not be made atomically
    /// with the insert and was the source of the cross-registration
    /// corruption.
    pub fn unbind_staged_entry(&self, cid: &CredentialId, bind: &Bind) {
        // `remove_if_mut` holds the shard lock across the whole closure:
        // the matching entry is decremented (or removed at the last
        // reference) and the credential bucket is dropped iff it became
        // empty — atomically, with no TOCTOU between the decrement, the
        // emptiness check, and the bucket removal (mirrors the
        // `retain(!is_empty())` discipline of the bulk unbinds). `bind`
        // de-dups into one refcounted entry, so at most one structurally-
        // equal entry exists; an absent `(cid, bind)` is a no-op.
        self.by_credential.remove_if_mut(cid, |_, rows| {
            if let Some(pos) = rows.iter().position(|r| &r.bind == bind) {
                if rows[pos].refs > 1 {
                    rows[pos].refs -= 1;
                } else {
                    rows.remove(pos);
                }
            }
            rows.is_empty()
        });
    }
}

#[cfg(test)]
#[path = "index/tests.rs"]
mod tests;
