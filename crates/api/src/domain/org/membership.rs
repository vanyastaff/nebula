//! Canonical in-memory [`MembershipStore`] implementation.
//!
//! This is the §4.5-honest production-quality default backing for org
//! membership — the same "the real impl *is* the in-memory one" posture
//! the durable control queue ([`InMemoryControlQueueRepo`]) and the
//! Plane-A identity backend ([`InMemoryAuthBackend`]) carry. There is no
//! storage-backed alternative to wire: `nebula_storage` ships no
//! `MembershipRepo` (the trait is an API-tier port, not a storage repo —
//! cf. ADR-0047 §3 / the Phase-2 `AuthBackend` precedent).
//!
//! ## One shared store — RBAC coherence
//!
//! The composition root wires **one** `Arc<InMemoryMembershipStore>` into
//! [`crate::AppState::membership_store`]; [`crate::middleware::rbac`] and
//! the `…/orgs/{org}/members` handlers therefore read and write the *same*
//! map. A membership added by `POST /members` is visible to the very next
//! RBAC check (no propagation window) — locked by
//! `tests/org_e2e.rs::added_member_is_immediately_rbac_authorized`.
//!
//! ## Durability (canon §11.6 / §11.5 — operator-facing)
//!
//! State is **process-local**: org memberships are held in a
//! [`tokio::sync::RwLock`]-guarded map, lost on restart and **not** shared
//! across replicas. This is the identical local-first caveat the in-memory
//! `AuthBackend` and the `memory` idempotency backend carry; it closes
//! when a storage-backed membership adapter lands (none exists today).
//! The composition root seeds a deterministic bootstrap org owner so the
//! RBAC gate is usable rather than dead-locked (documented in
//! `apps/server/src/compose.rs`).
//!
//! [`InMemoryControlQueueRepo`]: nebula_storage::repos::InMemoryControlQueueRepo
//! [`InMemoryAuthBackend`]: crate::domain::auth::backend::InMemoryAuthBackend

use std::{collections::HashMap, str::FromStr, sync::Arc};

use async_trait::async_trait;
use nebula_core::{OrgId, OrgRole, UserId, WorkspaceId, WorkspaceRole, scope::Principal};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::{
    error::ApiError,
    state::{AddMemberOutcome, MembershipStore, OrgMember, RemoveMemberOutcome},
};

/// Failure parsing the composition-root bootstrap-seed identities.
///
/// Surfaced by [`InMemoryMembershipStore::seeded_bootstrap`] so the
/// composition root can fail closed (never silently fall back to an
/// unseeded, dead-locked RBAC gate — `feedback_no_shims`). Kept in the
/// API tier so `apps/server` needs only `nebula_api` (ADR-0047 §3: the
/// composition root does not import `nebula_core` id types directly).
#[derive(Debug, Error)]
pub enum BootstrapSeedError {
    /// `org_id` is not a valid `org_<ULID>`.
    #[error("bootstrap org id {value:?} is not a valid org_<ULID>: {reason}")]
    OrgId {
        /// The offending value.
        value: String,
        /// Parse-error description.
        reason: String,
    },
    /// `owner_id` is not a valid `usr_<ULID>`.
    #[error("bootstrap owner id {value:?} is not a valid usr_<ULID>: {reason}")]
    OwnerId {
        /// The offending value.
        value: String,
        /// Parse-error description.
        reason: String,
    },
}

/// Stable identity key for a [`Principal`].
///
/// `Principal` intentionally does not derive `Hash` in `nebula-core`, so
/// the in-memory index keys members by their canonical string identity
/// (round-trip-stable for every variant) while storing the full
/// `Principal` in the value so reads return it faithfully.
///
/// Key injectivity is guaranteed by the **typed-ULID prefixes** baked
/// into each id's `Display` (`usr_…` / `svc_…` / `wf_…` are
/// non-overlapping prefixes from `domain_key`), *not* by the leading
/// `user:`/`svc:`/`wf:` discriminant tag here — the tag is purely
/// human-readable and must not be treated as the thing preventing
/// cross-variant collisions.
fn principal_key(p: &Principal) -> String {
    match p {
        Principal::User(id) => format!("user:{id}"),
        Principal::ServiceAccount(id) => format!("svc:{id}"),
        Principal::Workflow {
            workflow_id,
            trigger_id,
        } => match trigger_id {
            Some(t) => format!("wf:{workflow_id}:{t}"),
            None => format!("wf:{workflow_id}"),
        },
        Principal::System => "system".to_owned(),
    }
}

/// One stored membership row (the `Principal` is retained so reads return
/// the exact identity, not a reparsed approximation).
#[derive(Debug, Clone)]
struct Entry {
    principal: Principal,
    role: OrgRole,
}

/// Is this an org-administrative ("privileged") role?
///
/// The org-lockout invariant is "an org always retains ≥ 1 principal with
/// a privileged role" — only `OrgOwner`/`OrgAdmin` can satisfy the
/// admin-gated permissions (`MemberInvite`/`MemberRemove`/`OrgUpdate`/
/// `OrgDelete`), so an org with zero of them is permanently un-administer-
/// able. `OrgRole` is `#[non_exhaustive]`; any *future* variant is treated
/// as **non**-privileged here (fail-safe: a new role does not silently
/// count as an admin and let the last real admin be demoted away).
fn is_privileged(role: OrgRole) -> bool {
    matches!(role, OrgRole::OrgAdmin | OrgRole::OrgOwner)
}

/// The single org-lockout decision, shared by both guarded mutations so
/// the add-demotion path and the remove path **cannot drift**
/// (`feedback_type_enforce_not_discipline`).
///
/// Given the org's *current* member map and the write about to be applied
/// to `target_key`, return `true` iff the write is **safe** (leaves ≥ 1
/// privileged principal). `next_role` is `Some(role)` for an upsert (the
/// post-write role of `target_key`) or `None` for a removal.
///
/// Counts the privileged principals **excluding** `target_key`, then adds
/// the target back iff its post-write role is privileged. A write is
/// refused exactly when that post-write privileged count would be zero
/// *and* it actually changes things (the target was or would stop being
/// the last privileged principal). Removing/keeping a non-privileged
/// member can never reduce the privileged set, so it is always safe.
fn write_keeps_an_admin(
    members: &HashMap<String, Entry>,
    target_key: &str,
    next_role: Option<OrgRole>,
) -> bool {
    let privileged_excluding_target = members
        .iter()
        .filter(|(k, e)| k.as_str() != target_key && is_privileged(e.role))
        .count();
    let target_privileged_after = next_role.is_some_and(is_privileged);
    privileged_excluding_target + usize::from(target_privileged_after) >= 1
}

/// In-memory, process-local [`MembershipStore`] — the canonical default.
#[derive(Debug, Default)]
pub struct InMemoryMembershipStore {
    /// `org_id → (principal_key → Entry)`. Workspace-level explicit roles
    /// are not modelled here (none of the graduated endpoints touch them);
    /// `get_workspace_role` returns `None` so RBAC falls back to the
    /// org-implied role via `effective_workspace_role`, exactly as when no
    /// store is wired.
    orgs: RwLock<HashMap<OrgId, HashMap<String, Entry>>>,
}

impl InMemoryMembershipStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a store pre-seeded with one membership, synchronously.
    ///
    /// The composition root (`apps/server/src/compose.rs`) is sync and
    /// builds the state before the async runtime owns it; this avoids a
    /// `block_on`/`blocking_write` by populating the map directly at
    /// construction (no contention possible — nothing else holds a
    /// reference yet). This seed *is* the root-of-trust bootstrap (the
    /// first org admin), so it intentionally bypasses the handler authz
    /// gate — without it the RBAC gate would dead-lock (no member can be
    /// added because adding requires an existing admin).
    #[must_use]
    pub fn seeded(org_id: OrgId, principal: Principal, role: OrgRole) -> Self {
        let mut orgs = HashMap::new();
        let mut members = HashMap::new();
        members.insert(principal_key(&principal), Entry { principal, role });
        orgs.insert(org_id, members);
        Self {
            orgs: RwLock::new(orgs),
        }
    }

    /// Parse string bootstrap identities and return a seeded store
    /// granting `OrgOwner` on `org_id_str` to `owner_id_str`.
    ///
    /// The composition root (`apps/server`) calls this so it depends only
    /// on `nebula_api` — the `nebula_core` id parsing stays in the API
    /// tier (ADR-0047 §3). A malformed identity is a hard
    /// [`BootstrapSeedError`] (fail closed — never seed nothing, which
    /// would dead-lock the RBAC gate).
    pub fn seeded_bootstrap(
        org_id_str: &str,
        owner_id_str: &str,
    ) -> Result<Self, BootstrapSeedError> {
        let org_id = OrgId::from_str(org_id_str).map_err(|e| BootstrapSeedError::OrgId {
            value: org_id_str.to_owned(),
            reason: e.to_string(),
        })?;
        let owner_id = UserId::from_str(owner_id_str).map_err(|e| BootstrapSeedError::OwnerId {
            value: owner_id_str.to_owned(),
            reason: e.to_string(),
        })?;
        Ok(Self::seeded(
            org_id,
            Principal::User(owner_id),
            OrgRole::OrgOwner,
        ))
    }

    /// Wrap in an `Arc` for `AppState::with_membership_store`.
    #[must_use]
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// Seed a membership directly (test bootstrap only).
    ///
    /// Used by `tests/common` to seed an org admin before a request — it
    /// bypasses the handler authz gate intentionally (it *is* the
    /// root-of-trust seed, not a request path). The sync [`Self::seeded`]
    /// is the composition-root counterpart.
    pub async fn seed(&self, org_id: OrgId, principal: Principal, role: OrgRole) {
        let mut guard = self.orgs.write().await;
        guard
            .entry(org_id)
            .or_default()
            .insert(principal_key(&principal), Entry { principal, role });
    }
}

#[async_trait]
impl MembershipStore for InMemoryMembershipStore {
    async fn get_org_role(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<Option<OrgRole>, ApiError> {
        let guard = self.orgs.read().await;
        Ok(guard
            .get(&org_id)
            .and_then(|members| members.get(&principal_key(principal)))
            .map(|e| e.role))
    }

    async fn get_workspace_role(
        &self,
        _workspace_id: WorkspaceId,
        _principal: &Principal,
    ) -> Result<Option<WorkspaceRole>, ApiError> {
        // No explicit workspace-role grants in this index — RBAC derives
        // the effective workspace role from the org role
        // (`effective_workspace_role`), identical to the no-store path.
        Ok(None)
    }

    async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>, ApiError> {
        let guard = self.orgs.read().await;
        Ok(guard
            .get(&org_id)
            .map(|members| {
                members
                    .values()
                    .map(|e| OrgMember {
                        principal: e.principal.clone(),
                        role: e.role,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn add_member(
        &self,
        org_id: OrgId,
        principal: &Principal,
        role: OrgRole,
    ) -> Result<(), ApiError> {
        let mut guard = self.orgs.write().await;
        guard.entry(org_id).or_default().insert(
            principal_key(principal),
            Entry {
                principal: principal.clone(),
                role,
            },
        );
        Ok(())
    }

    async fn remove_member(&self, org_id: OrgId, principal: &Principal) -> Result<bool, ApiError> {
        let mut guard = self.orgs.write().await;
        let Some(members) = guard.get_mut(&org_id) else {
            return Ok(false);
        };
        Ok(members.remove(&principal_key(principal)).is_some())
    }

    async fn add_member_guarded(
        &self,
        org_id: OrgId,
        principal: &Principal,
        role: OrgRole,
    ) -> Result<AddMemberOutcome, ApiError> {
        // ONE write-guard for the count-and-mutate: the org-lockout
        // decision and the upsert are atomic, so no concurrent
        // demotion/removal can observe a stale privileged count and slip
        // the org below one admin (closes the TOCTOU the handler-level
        // check had).
        let mut guard = self.orgs.write().await;
        let key = principal_key(principal);
        let members = guard.entry(org_id).or_default();

        if !write_keeps_an_admin(members, &key, Some(role)) {
            return Ok(AddMemberOutcome::WouldLockOut);
        }
        members.insert(
            key,
            Entry {
                principal: principal.clone(),
                role,
            },
        );
        Ok(AddMemberOutcome::Added)
    }

    async fn remove_member_guarded(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<RemoveMemberOutcome, ApiError> {
        let mut guard = self.orgs.write().await;
        let key = principal_key(principal);
        let Some(members) = guard.get_mut(&org_id) else {
            return Ok(RemoveMemberOutcome::NotFound);
        };
        if !members.contains_key(&key) {
            // Membership re-checked *inside* the lock: an existence
            // TOCTOU collapses to a clean NotFound (no disclosure).
            return Ok(RemoveMemberOutcome::NotFound);
        }
        if !write_keeps_an_admin(members, &key, None) {
            return Ok(RemoveMemberOutcome::WouldLockOut);
        }
        members.remove(&key);
        Ok(RemoveMemberOutcome::Removed)
    }

    async fn list_orgs_for_principal(
        &self,
        principal: &Principal,
    ) -> Result<Vec<(OrgId, OrgRole)>, ApiError> {
        let key = principal_key(principal);
        let guard = self.orgs.read().await;
        Ok(guard
            .iter()
            .filter_map(|(org_id, members)| members.get(&key).map(|e| (*org_id, e.role)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::UserId;

    use super::*;

    fn user() -> Principal {
        Principal::User(UserId::new())
    }

    #[tokio::test]
    async fn add_then_get_role_is_visible() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let p = user();

        assert_eq!(store.get_org_role(org, &p).await.unwrap(), None);
        store.add_member(org, &p, OrgRole::OrgAdmin).await.unwrap();
        assert_eq!(
            store.get_org_role(org, &p).await.unwrap(),
            Some(OrgRole::OrgAdmin),
            "add_member must be immediately visible to get_org_role (RBAC coherence)"
        );
    }

    #[tokio::test]
    async fn add_member_upserts_role() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let p = user();
        store.add_member(org, &p, OrgRole::OrgMember).await.unwrap();
        store.add_member(org, &p, OrgRole::OrgAdmin).await.unwrap();
        let members = store.list_members(org).await.unwrap();
        assert_eq!(members.len(), 1, "upsert must not duplicate the row");
        assert_eq!(members[0].role, OrgRole::OrgAdmin);
    }

    #[tokio::test]
    async fn remove_member_reports_presence() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let p = user();
        assert!(
            !store.remove_member(org, &p).await.unwrap(),
            "removing a non-member must report false (handler → 404)"
        );
        store.add_member(org, &p, OrgRole::OrgMember).await.unwrap();
        assert!(store.remove_member(org, &p).await.unwrap());
        assert_eq!(store.get_org_role(org, &p).await.unwrap(), None);
    }

    #[tokio::test]
    async fn list_orgs_for_principal_is_cross_org() {
        let store = InMemoryMembershipStore::new();
        let p = user();
        let org_a = OrgId::new();
        let org_b = OrgId::new();
        let other = user();
        store
            .add_member(org_a, &p, OrgRole::OrgOwner)
            .await
            .unwrap();
        store
            .add_member(org_b, &p, OrgRole::OrgMember)
            .await
            .unwrap();
        store
            .add_member(org_b, &other, OrgRole::OrgAdmin)
            .await
            .unwrap();

        let mut mine = store.list_orgs_for_principal(&p).await.unwrap();
        mine.sort_by_key(|(_, r)| *r);
        assert_eq!(mine.len(), 2);
        // Membership in `other`'s org must not leak into `p`'s enumeration.
        assert!(mine.iter().all(|(o, _)| *o == org_a || *o == org_b));
        assert_eq!(
            store.list_orgs_for_principal(&other).await.unwrap(),
            vec![(org_b, OrgRole::OrgAdmin)]
        );
    }

    #[tokio::test]
    async fn isolation_between_orgs() {
        let store = InMemoryMembershipStore::new();
        let org_a = OrgId::new();
        let org_b = OrgId::new();
        let p = user();
        store
            .add_member(org_a, &p, OrgRole::OrgAdmin)
            .await
            .unwrap();
        assert_eq!(
            store.get_org_role(org_b, &p).await.unwrap(),
            None,
            "a member of org A must have no role in org B"
        );
        assert!(store.list_members(org_b).await.unwrap().is_empty());
    }

    // ── org-lockout invariant at the atomic store seam ───────────────────

    #[tokio::test]
    async fn guarded_add_refuses_sole_owner_self_demote() {
        // C1 at the seam: the ONLY privileged principal demoting itself
        // must be refused (the post-write privileged count would be 0).
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let owner = user();
        store
            .add_member(org, &owner, OrgRole::OrgOwner)
            .await
            .unwrap();

        assert_eq!(
            store
                .add_member_guarded(org, &owner, OrgRole::OrgMember)
                .await
                .unwrap(),
            AddMemberOutcome::WouldLockOut
        );
        // The store must be UNCHANGED — refusal is not a partial write.
        assert_eq!(
            store.get_org_role(org, &owner).await.unwrap(),
            Some(OrgRole::OrgOwner),
            "a refused demotion must not mutate the row"
        );
    }

    #[tokio::test]
    async fn guarded_add_allows_demote_when_another_admin_remains() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let owner = user();
        let admin2 = user();
        store
            .add_member(org, &owner, OrgRole::OrgOwner)
            .await
            .unwrap();
        store
            .add_member(org, &admin2, OrgRole::OrgAdmin)
            .await
            .unwrap();

        assert_eq!(
            store
                .add_member_guarded(org, &admin2, OrgRole::OrgMember)
                .await
                .unwrap(),
            AddMemberOutcome::Added,
            "demotion is fine while another privileged principal remains"
        );
        assert_eq!(
            store.get_org_role(org, &admin2).await.unwrap(),
            Some(OrgRole::OrgMember)
        );
    }

    #[tokio::test]
    async fn guarded_remove_refuses_last_admin_and_is_idor_safe() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let owner = user();
        store
            .add_member(org, &owner, OrgRole::OrgOwner)
            .await
            .unwrap();

        // Last privileged → WouldLockOut, row untouched.
        assert_eq!(
            store.remove_member_guarded(org, &owner).await.unwrap(),
            RemoveMemberOutcome::WouldLockOut
        );
        assert_eq!(
            store.get_org_role(org, &owner).await.unwrap(),
            Some(OrgRole::OrgOwner)
        );
        // Non-member → NotFound (no disclosure), even on an unknown org.
        assert_eq!(
            store.remove_member_guarded(org, &user()).await.unwrap(),
            RemoveMemberOutcome::NotFound
        );
        assert_eq!(
            store
                .remove_member_guarded(OrgId::new(), &owner)
                .await
                .unwrap(),
            RemoveMemberOutcome::NotFound
        );
    }

    #[tokio::test]
    async fn guarded_remove_non_privileged_never_locks_out() {
        // Removing a plain member can never reduce the privileged set —
        // the guard must not over-block it down to the last admin.
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let owner = user();
        let plain = user();
        store
            .add_member(org, &owner, OrgRole::OrgOwner)
            .await
            .unwrap();
        store
            .add_member(org, &plain, OrgRole::OrgMember)
            .await
            .unwrap();
        assert_eq!(
            store.remove_member_guarded(org, &plain).await.unwrap(),
            RemoveMemberOutcome::Removed
        );
    }

    // ── I2: true concurrency against the shared Arc store ────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_remove_of_last_two_admins_keeps_one() {
        // THE TOCTOU regression: two admins, two concurrent removals (one
        // each). Pre-fix, both could observe privileged==2, both pass the
        // handler check, both delete → zero admins. With the invariant at
        // the seam under one lock, EXACTLY ONE removal succeeds and the
        // org always retains ≥ 1 privileged principal.
        for _ in 0..64 {
            let store = Arc::new(InMemoryMembershipStore::new());
            let org = OrgId::new();
            let a = user();
            let b = user();
            store.add_member(org, &a, OrgRole::OrgAdmin).await.unwrap();
            store.add_member(org, &b, OrgRole::OrgAdmin).await.unwrap();

            let (s1, s2) = (Arc::clone(&store), Arc::clone(&store));
            let (a1, b1) = (a.clone(), b.clone());
            let h1 = tokio::spawn(async move { s1.remove_member_guarded(org, &a1).await.unwrap() });
            let h2 = tokio::spawn(async move { s2.remove_member_guarded(org, &b1).await.unwrap() });
            let (r1, r2) = (h1.await.unwrap(), h2.await.unwrap());

            let removed = [r1, r2]
                .iter()
                .filter(|o| **o == RemoveMemberOutcome::Removed)
                .count();
            let locked = [r1, r2]
                .iter()
                .filter(|o| **o == RemoveMemberOutcome::WouldLockOut)
                .count();
            assert_eq!(
                (removed, locked),
                (1, 1),
                "exactly one concurrent removal may succeed; the other must \
                 be refused WouldLockOut"
            );
            let privileged = store
                .list_members(org)
                .await
                .unwrap()
                .into_iter()
                .filter(|m| is_privileged(m.role))
                .count();
            assert_eq!(
                privileged, 1,
                "the org must always retain exactly one privileged principal"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_demote_and_remove_keeps_one_admin() {
        // Cross-path race: one task demotes admin A via add_member_guarded
        // while another removes admin B via remove_member_guarded. Only
        // one may win; the org must keep ≥ 1 privileged principal.
        for _ in 0..64 {
            let store = Arc::new(InMemoryMembershipStore::new());
            let org = OrgId::new();
            let a = user();
            let b = user();
            store.add_member(org, &a, OrgRole::OrgAdmin).await.unwrap();
            store.add_member(org, &b, OrgRole::OrgAdmin).await.unwrap();

            let (s1, s2) = (Arc::clone(&store), Arc::clone(&store));
            let (a1, b1) = (a.clone(), b.clone());
            let demote = tokio::spawn(async move {
                s1.add_member_guarded(org, &a1, OrgRole::OrgMember)
                    .await
                    .unwrap()
            });
            let remove =
                tokio::spawn(async move { s2.remove_member_guarded(org, &b1).await.unwrap() });
            let (d, r) = (demote.await.unwrap(), remove.await.unwrap());

            let demote_won = d == AddMemberOutcome::Added;
            let remove_won = r == RemoveMemberOutcome::Removed;
            assert!(
                demote_won ^ remove_won,
                "exactly one of {{demote, remove}} may win (got demote={demote_won}, \
                 remove={remove_won})"
            );
            let privileged = store
                .list_members(org)
                .await
                .unwrap()
                .into_iter()
                .filter(|m| is_privileged(m.role))
                .count();
            assert!(
                privileged >= 1,
                "the org must always retain ≥ 1 privileged principal (got {privileged})"
            );
        }
    }
}
