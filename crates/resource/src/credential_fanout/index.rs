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
use nebula_core::{CredentialKey, ResourceKey, ScopeLevel};
use nebula_credential::{CredentialId, TenantScope};
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
    pub(super) fn add(&mut self, other: Self) {
        self.success += other.success;
        self.failed += other.failed;
        self.timed_out += other.timed_out;
        self.deferred += other.deferred;
        self.abandoned += other.abandoned;
        self.drain_timed_out += other.drain_timed_out;
        self.observation_timed_out += other.observation_timed_out;
    }

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
/// [`Bind`]). Published references are visible to fan-out; staged references
/// are held while `register_and_bind` waits for the manager lifecycle gate.
/// Removal and replacement release only published ownership, so they cannot
/// erase a concurrent registration that has staged but not yet published.
#[derive(Debug, Clone)]
struct BindRef {
    bind: Bind,
    published_context: Option<(TenantScope, CredentialKey)>,
    staged_context: Option<(TenantScope, CredentialKey)>,
    published: usize,
    staged: usize,
}

/// Per-credential row list. Most credentials resolve into one or two
/// resource rows, so two rows live inline in the map entry (no heap
/// allocation, better locality on the rotation fan-out read); larger
/// families spill to the heap transparently.
type BindRows = SmallVec<[BindRef; 2]>;
type PublishedBinding = (CredentialId, Bind, Option<(TenantScope, CredentialKey)>);

#[derive(Clone)]
pub(super) struct PendingRevokeAdmission {
    pub(super) credential_id: CredentialId,
    pub(super) key: ResourceKey,
    pub(super) slot: String,
    pub(super) managed: std::sync::Arc<dyn crate::registry::ManagedHandle>,
    state: RevokeAdmissionState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RevokeAdmissionState {
    Pending,
    Claimed,
    Admitted,
}

pub(super) struct RevokeAdmissionClaim<'a> {
    index: &'a ResourceFanoutIndex,
    pub(super) entry: PendingRevokeAdmission,
    settled: bool,
}

impl RevokeAdmissionClaim<'_> {
    pub(super) fn accepted(mut self) {
        self.index
            .settle_revoke_claim(&self.entry, Some(RevokeAdmissionState::Admitted));
        self.settled = true;
    }

    pub(super) fn retry(mut self) {
        self.index
            .settle_revoke_claim(&self.entry, Some(RevokeAdmissionState::Pending));
        self.settled = true;
    }

    pub(super) fn discard(mut self) {
        self.index.settle_revoke_claim(&self.entry, None);
        self.settled = true;
    }
}

impl Drop for RevokeAdmissionClaim<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.index
                .settle_revoke_claim(&self.entry, Some(RevokeAdmissionState::Pending));
        }
    }
}

impl std::fmt::Debug for PendingRevokeAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingRevokeAdmission")
            .field("credential_id", &self.credential_id)
            .field("key", &self.key)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

/// Maximum credential projections admitted across concurrent fan-out calls.
pub(super) const MAX_CONCURRENT_PROJECTIONS: usize = 32;

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
#[derive(Debug)]
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
    /// Owner-qualified replacement hints retained until every bound row can
    /// project them. Its key space is bounded by live reverse-index entries.
    material_contexts: DashMap<CredentialId, (TenantScope, CredentialKey, u64)>,
    material_context_sequence: std::sync::atomic::AtomicU64,
    pending_revoke_admissions: std::sync::Mutex<Vec<PendingRevokeAdmission>>,
    staged_revoke_intents: std::sync::Mutex<std::collections::HashSet<CredentialId>>,
    revoke_retry_notify: tokio::sync::Notify,
    /// Shared admission keeps direct dispatch and reconciliation under one
    /// provider/persistence concurrency budget.
    pub(super) projection_admission: tokio::sync::Semaphore,
}

impl Default for ResourceFanoutIndex {
    fn default() -> Self {
        Self {
            by_credential: DashMap::new(),
            material_contexts: DashMap::new(),
            material_context_sequence: std::sync::atomic::AtomicU64::new(1),
            pending_revoke_admissions: std::sync::Mutex::new(Vec::new()),
            staged_revoke_intents: std::sync::Mutex::new(std::collections::HashSet::new()),
            revoke_retry_notify: tokio::sync::Notify::new(),
            projection_admission: tokio::sync::Semaphore::new(MAX_CONCURRENT_PROJECTIONS),
        }
    }
}

impl ResourceFanoutIndex {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an event-only binding showing that the resource row
    /// `(resource_key, scope, slot_name, slot_identity)` resolved `cid` for
    /// one of its credential slots.
    ///
    /// This compatibility path omits the durable owner/key context required
    /// by startup reconciliation. Production registration stages that
    /// context beside the [`Bind`] and publishes both atomically with the
    /// manager row.
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
        self.add_bind_ref(cid, entry, None, None, false);
    }

    /// Publishes a rotation binding with the owner-qualified context needed
    /// for durable startup and periodic reconciliation.
    #[doc(hidden)]
    pub fn bind_with_context(
        &self,
        cid: CredentialId,
        binding: Bind,
        credential_scope: TenantScope,
        credential_key: CredentialKey,
    ) {
        self.add_bind_ref(
            cid,
            binding,
            Some(credential_scope.durable_owner_scope()),
            Some(credential_key),
            false,
        );
    }

    #[cfg(test)]
    pub(crate) fn stage_bind(&self, cid: CredentialId, bind: Bind) {
        self.add_bind_ref(cid, bind, None, None, true);
    }

    pub(crate) fn stage_bind_with_context(
        &self,
        cid: CredentialId,
        bind: Bind,
        credential_scope: TenantScope,
        credential_key: CredentialKey,
    ) {
        self.add_bind_ref(
            cid,
            bind,
            Some(credential_scope),
            Some(credential_key),
            true,
        );
    }

    fn add_bind_ref(
        &self,
        cid: CredentialId,
        bind: Bind,
        credential_scope: Option<TenantScope>,
        credential_key: Option<CredentialKey>,
        staged: bool,
    ) {
        let mut rows = self.by_credential.entry(cid).or_default();
        let context = credential_scope.zip(credential_key);
        match rows.iter_mut().find(|row| row.bind == bind) {
            Some(existing) => {
                if staged {
                    if existing.staged_context.is_none() {
                        existing.staged_context = context;
                    }
                    existing.staged += 1;
                } else {
                    if existing.published_context.is_none() {
                        existing.published_context = context;
                    }
                    existing.published += 1;
                }
            },
            None => rows.push(BindRef {
                bind,
                published_context: if staged { None } else { context.clone() },
                staged_context: if staged { context } else { None },
                published: usize::from(!staged),
                staged: usize::from(staged),
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
            .map(|rows| {
                rows.iter()
                    .filter(|row| row.published != 0)
                    .map(|row| row.bind.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn published_bindings(
        &self,
        credential_id: Option<CredentialId>,
    ) -> Vec<PublishedBinding> {
        if let Some(credential_id) = credential_id {
            return self
                .by_credential
                .get(&credential_id)
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.published != 0)
                        .map(|row| {
                            (
                                credential_id,
                                row.bind.clone(),
                                row.published_context.clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
        self.by_credential
            .iter()
            .flat_map(|entry| {
                let credential_id = *entry.key();
                entry
                    .value()
                    .iter()
                    .filter(|row| row.published != 0)
                    .map(move |row| {
                        (
                            credential_id,
                            row.bind.clone(),
                            row.published_context.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(super) fn remember_material_context(
        &self,
        cid: CredentialId,
        scope: TenantScope,
        credential_key: CredentialKey,
    ) -> u64 {
        let sequence = self
            .material_context_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.by_credential.contains_key(&cid) {
            self.material_contexts
                .insert(cid, (scope, credential_key, sequence));
        }
        sequence
    }

    pub(super) fn pending_material_contexts(
        &self,
    ) -> Vec<(CredentialId, TenantScope, CredentialKey, u64)> {
        self.material_contexts
            .retain(|cid, _| self.by_credential.contains_key(cid));
        self.material_contexts
            .iter()
            .map(|entry| {
                let (scope, key, sequence) = entry.value();
                (*entry.key(), scope.clone(), key.clone(), *sequence)
            })
            .collect()
    }

    pub(super) fn forget_material_context(&self, cid: &CredentialId, sequence: u64) {
        self.material_contexts
            .remove_if(cid, |_, context| context.2 == sequence);
    }

    pub(crate) fn remember_pending_revoke(
        &self,
        credential_id: CredentialId,
        key: ResourceKey,
        slot: &str,
        managed: std::sync::Arc<dyn crate::registry::ManagedHandle>,
    ) {
        let mut pending = self
            .pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.iter().any(|entry| {
            entry.credential_id == credential_id
                && entry.slot == slot
                && std::sync::Arc::ptr_eq(&entry.managed, &managed)
        }) {
            return;
        }
        pending.push(PendingRevokeAdmission {
            credential_id,
            key,
            slot: slot.to_owned(),
            managed,
            state: RevokeAdmissionState::Pending,
        });
        self.revoke_retry_notify.notify_one();
    }

    pub(crate) fn remember_staged_revoke(&self, credential_id: CredentialId) {
        self.staged_revoke_intents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(credential_id);
    }

    pub(crate) async fn revoke_retry_notified(&self) {
        self.revoke_retry_notify.notified().await;
    }

    pub(super) fn pending_revokes(&self) -> Vec<PendingRevokeAdmission> {
        self.pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|entry| entry.state == RevokeAdmissionState::Pending)
            .cloned()
            .collect()
    }

    pub(super) fn claim_revoke_admission(
        &self,
        credential_id: CredentialId,
        slot: &str,
        managed: &std::sync::Arc<dyn crate::registry::ManagedHandle>,
    ) -> Option<RevokeAdmissionClaim<'_>> {
        self.claim_revoke_admission_inner(credential_id, slot, managed, false)
    }

    pub(super) fn claim_pending_revoke(
        &self,
        entry: &PendingRevokeAdmission,
    ) -> Option<RevokeAdmissionClaim<'_>> {
        self.claim_revoke_admission_inner(entry.credential_id, &entry.slot, &entry.managed, true)
    }

    fn claim_revoke_admission_inner(
        &self,
        credential_id: CredentialId,
        slot: &str,
        managed: &std::sync::Arc<dyn crate::registry::ManagedHandle>,
        pending_only: bool,
    ) -> Option<RevokeAdmissionClaim<'_>> {
        let mut admissions = self
            .pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = if let Some(entry) = admissions.iter_mut().find(|entry| {
            entry.credential_id == credential_id
                && entry.slot == slot
                && std::sync::Arc::ptr_eq(&entry.managed, managed)
        }) {
            if entry.state != RevokeAdmissionState::Pending {
                return None;
            }
            entry.state = RevokeAdmissionState::Claimed;
            entry.clone()
        } else {
            if pending_only {
                return None;
            }
            let entry = PendingRevokeAdmission {
                credential_id,
                key: managed.resource_key(),
                slot: slot.to_owned(),
                managed: std::sync::Arc::clone(managed),
                state: RevokeAdmissionState::Claimed,
            };
            admissions.push(entry.clone());
            entry
        };
        Some(RevokeAdmissionClaim {
            index: self,
            entry,
            settled: false,
        })
    }

    fn settle_revoke_claim(
        &self,
        claim: &PendingRevokeAdmission,
        state: Option<RevokeAdmissionState>,
    ) {
        let mut admissions = self
            .pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(pos) = admissions.iter().position(|entry| {
            entry.credential_id == claim.credential_id
                && entry.slot == claim.slot
                && std::sync::Arc::ptr_eq(&entry.managed, &claim.managed)
        }) else {
            return;
        };
        if admissions[pos].state != RevokeAdmissionState::Claimed {
            return;
        }
        if let Some(state) = state {
            admissions[pos].state = state;
        } else {
            admissions.remove(pos);
        }
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
            for row in rows
                .iter_mut()
                .filter(|row| row.bind.resource_key == *resource_key && row.bind.scope == *scope)
            {
                row.published = 0;
            }
            rows.retain(|row| row.published != 0 || row.staged != 0);
            !rows.is_empty()
        });
        self.prune_orphan_contexts();
    }

    /// Removes every binding for a whole-key administrative removal.
    pub(crate) fn unbind_resource_key(&self, resource_key: &ResourceKey) {
        self.by_credential.retain(|_, rows| {
            for row in rows
                .iter_mut()
                .filter(|row| row.bind.resource_key == *resource_key)
            {
                row.published = 0;
            }
            rows.retain(|row| row.published != 0 || row.staged != 0);
            !rows.is_empty()
        });
        self.prune_orphan_contexts();
    }

    pub(crate) fn unbind_removed_resource_key(
        &self,
        resource_key: &ResourceKey,
        removed: &[std::sync::Arc<dyn crate::registry::ManagedHandle>],
    ) {
        self.unbind_resource_key(resource_key);
        self.pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|entry| {
                !removed
                    .iter()
                    .any(|handle| std::sync::Arc::ptr_eq(&entry.managed, handle))
            });
    }

    pub(crate) fn clear_for_manager_shutdown(&self) {
        self.by_credential.clear();
        self.material_contexts.clear();
        self.pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.staged_revoke_intents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
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
            for row in rows.iter_mut().filter(|row| {
                row.bind.resource_key == *resource_key
                    && row.bind.scope == *scope
                    && row.bind.slot_identity == *slot_identity
            }) {
                row.published = 0;
            }
            rows.retain(|row| row.published != 0 || row.staged != 0);
            !rows.is_empty()
        });
        self.prune_orphan_contexts();
    }

    pub(crate) fn unbind_removed_resource_identity(
        &self,
        resource_key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
        removed: &std::sync::Arc<dyn crate::registry::ManagedHandle>,
    ) {
        self.unbind_resource_identity(resource_key, scope, slot_identity);
        self.pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|entry| !std::sync::Arc::ptr_eq(&entry.managed, removed));
    }

    pub(crate) fn unbind_replaced_resource_identity(
        &self,
        resource_key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
        displaced: &std::sync::Arc<dyn crate::registry::ManagedHandle>,
    ) {
        self.unbind_resource_identity(resource_key, scope, slot_identity);
        self.pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|entry| !std::sync::Arc::ptr_eq(&entry.managed, displaced));
        self.prune_orphan_contexts();
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
            if let Some(row) = rows.iter_mut().find(|row| &row.bind == bind) {
                row.staged = row.staged.saturating_sub(1);
                if row.staged == 0 {
                    row.staged_context = None;
                }
            }
            rows.retain(|row| row.published != 0 || row.staged != 0);
            rows.is_empty()
        });
        self.prune_orphan_contexts();
    }

    pub(crate) fn publish_staged_entry(&self, cid: &CredentialId, bind: &Bind) -> bool {
        if let Some(mut rows) = self.by_credential.get_mut(cid)
            && let Some(row) = rows.iter_mut().find(|row| &row.bind == bind)
            && row.staged != 0
        {
            row.staged -= 1;
            row.published += 1;
            if row.published_context.is_none() {
                row.published_context = row.staged_context.clone();
            }
            if row.staged == 0 {
                row.staged_context = None;
            }
        }
        let should_revoke = self
            .staged_revoke_intents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(cid);
        if !self.has_staged_binding(cid) {
            self.staged_revoke_intents
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(cid);
        }
        should_revoke
    }

    fn prune_orphan_contexts(&self) {
        self.material_contexts
            .retain(|cid, _| self.by_credential.contains_key(cid));
        self.pending_revoke_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|entry| self.has_published_binding(&entry.credential_id));
        self.staged_revoke_intents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|credential_id| self.has_staged_binding(credential_id));
    }

    fn has_published_binding(&self, cid: &CredentialId) -> bool {
        self.by_credential
            .get(cid)
            .is_some_and(|rows| rows.iter().any(|row| row.published != 0))
    }

    pub(super) fn has_staged_binding(&self, cid: &CredentialId) -> bool {
        self.by_credential
            .get(cid)
            .is_some_and(|rows| rows.iter().any(|row| row.staged != 0))
    }
}

#[cfg(test)]
#[path = "index/tests.rs"]
mod tests;
