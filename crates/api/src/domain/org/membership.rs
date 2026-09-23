//! Canonical in-memory reference implementation of the API-tier
//! [`MembershipStore`].
//!
//! This adapter implements the complete authorization-policy contract,
//! including consistent role snapshots and atomic last-admin guards, but its
//! state is process-local. `nebula-storage-port` defines the durable snapshot
//! and guarded-mutation contracts, with backend implementations in storage.
//! The first-party server translates those contracts into this API policy
//! port through one backend-bound tenant directory.
//!
//! ## One shared store — RBAC coherence
//!
//! When provisioned, **one** `Arc<InMemoryMembershipStore>` is wired into
//! [`crate::AppState::membership_store`]; [`crate::middleware::rbac`], the
//! credential command authority, and the `…/orgs/{org}/members` handlers
//! therefore read and write the *same* map. A membership added by `POST /members` is visible to the very next
//! RBAC check (no propagation window) — locked by
//! `tests/org_e2e.rs::added_member_is_immediately_rbac_authorized`.
//!
//! ## Provisioning (honest capability contract / credential secrecy — NOT auto-wired)
//!
//! This adapter is the process-local reference implementation. The first-party
//! server instead wires the durable storage-backed tenant directory and can
//! bootstrap it only from explicit operator configuration tied to an existing
//! authenticatable owner. Internal/reference API composition can exercise this seam via
//! [`InMemoryMembershipStore::seeded_bootstrap`] +
//! [`crate::AppState::with_membership_store`], registering the same owner
//! identity in the wired `AuthBackend`.
//!
//! ## Durability
//!
//! State is **process-local**: org memberships are held in a
//! [`tokio::sync::RwLock`]-guarded map, lost on restart and **not** shared
//! across replicas. This is the identical local-first caveat the in-memory
//! `AuthBackend` and the `memory` idempotency backend carry. Closing it
//! requires a durable adapter for the API policy port (or an apps-owned bridge
//! over the lower-level storage port) that preserves one-snapshot reads and
//! atomic guarded mutations.
//!
//! [`InMemoryAuthBackend`]: crate::domain::auth::backend::InMemoryAuthBackend

use std::{collections::HashMap, str::FromStr, sync::Arc};

use async_trait::async_trait;
use nebula_core::{OrgId, OrgRole, UserId, WorkspaceId, WorkspaceRole, scope::Principal};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::{
    error::ApiError,
    state::{
        AddMemberOutcome, MembershipStore, OrgMember, RemoveMemberOutcome,
        TenantMembershipSnapshot, WorkspaceMember,
    },
};

/// Failure parsing the bootstrap-seed identities passed to
/// [`InMemoryMembershipStore::seeded_bootstrap`].
///
/// The constructor fails closed on a malformed id (never silently seeds
/// nothing, which would leave a wired-but-empty store dead-locking the
/// RBAC gate — `feedback_no_shims`). String-typed parsing is kept in the
/// API tier so an integrator crate that does not depend on `nebula_core`
/// can still provision a store (3).
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
        // Non-exhaustive: future principal kinds must be assigned a unique key prefix.
        _ => "unknown".to_owned(),
    }
}

/// One stored membership row (the `Principal` is retained so reads return
/// the exact identity, not a reparsed approximation).
#[derive(Debug, Clone)]
struct Entry {
    principal: Principal,
    role: OrgRole,
}

#[derive(Debug, Clone)]
struct WorkspaceEntry {
    principal: Principal,
    role: WorkspaceRole,
}

#[derive(Debug, Default)]
struct MembershipState {
    orgs: HashMap<OrgId, HashMap<String, Entry>>,
    workspaces: HashMap<(OrgId, WorkspaceId), HashMap<String, WorkspaceEntry>>,
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
/// refused whenever that post-write privileged count would be zero, including
/// a first insert of a non-privileged member into an empty organization.
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

/// In-memory, process-local [`MembershipStore`] reference adapter.
#[derive(Debug, Default)]
pub struct InMemoryMembershipStore {
    /// One lock covers org and workspace roles so authorization snapshots
    /// cannot splice grants observed at different instants.
    state: RwLock<MembershipState>,
}

impl InMemoryMembershipStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a store pre-seeded with one membership, synchronously.
    ///
    /// Populates the map directly at construction (no contention — nothing
    /// else holds a reference yet), so a sync caller can build a
    /// provisioned store without a `block_on`/`blocking_write`. This seed
    /// *is* the root-of-trust bootstrap (the first org admin) and
    /// intentionally bypasses the handler authz gate — it is the
    /// provisioning entry point, not a request path. The string-id
    /// wrapper [`Self::seeded_bootstrap`] (a technical reference-composition
    /// helper) and the `tests/common::org_support`
    /// fixtures build on this; it is **not** auto-called by the default
    /// `apps/server` binary (see [`Self::seeded_bootstrap`] for why).
    #[must_use]
    pub fn seeded(org_id: OrgId, principal: Principal, role: OrgRole) -> Self {
        let mut orgs = HashMap::new();
        let mut members = HashMap::new();
        members.insert(principal_key(&principal), Entry { principal, role });
        orgs.insert(org_id, members);
        Self {
            state: RwLock::new(MembershipState {
                orgs,
                workspaces: HashMap::new(),
            }),
        }
    }

    /// Parse string bootstrap identities and return a seeded store
    /// granting `OrgOwner` on `org_id_str` to `owner_id_str`.
    ///
    /// This is a **technical reference/test provisioning helper** for the org
    /// member-management feature: internal composition wires
    /// `AppState::with_membership_store(this)` **and** registers the same
    /// `owner_id_str` in its `AuthBackend` so the bootstrap owner can actually
    /// authenticate. It is not a supported downstream deployment surface;
    /// first-party operator provisioning lives in `apps/server` and uses the
    /// durable storage authority instead.
    ///
    /// String-typed (not `OrgId`/`UserId`) so technical composition code need
    /// not duplicate parsing — the id
    /// parsing stays in the API tier (3). A malformed identity
    /// is a hard [`BootstrapSeedError`] (fail closed — never seed
    /// nothing, which would dead-lock the RBAC gate).
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
    #[cfg(any(test, feature = "test-util"))]
    pub async fn seed_for_test(&self, org_id: OrgId, principal: Principal, role: OrgRole) {
        let mut guard = self.state.write().await;
        guard
            .orgs
            .entry(org_id)
            .or_default()
            .insert(principal_key(&principal), Entry { principal, role });
    }
}

#[async_trait]
impl MembershipStore for InMemoryMembershipStore {
    async fn get_tenant_membership(
        &self,
        org_id: OrgId,
        workspace_id: Option<WorkspaceId>,
        principal: &Principal,
    ) -> Result<TenantMembershipSnapshot, ApiError> {
        let guard = self.state.read().await;
        let org_role = guard
            .orgs
            .get(&org_id)
            .and_then(|members| members.get(&principal_key(principal)))
            .map(|entry| entry.role);
        let workspace_role = workspace_id.and_then(|workspace_id| {
            guard
                .workspaces
                .get(&(org_id, workspace_id))
                .and_then(|members| members.get(&principal_key(principal)))
                .map(|entry| entry.role)
        });
        Ok(TenantMembershipSnapshot {
            org_role,
            workspace_role,
        })
    }

    async fn get_org_role(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<Option<OrgRole>, ApiError> {
        let guard = self.state.read().await;
        Ok(guard
            .orgs
            .get(&org_id)
            .and_then(|members| members.get(&principal_key(principal)))
            .map(|e| e.role))
    }

    async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>, ApiError> {
        let guard = self.state.read().await;
        Ok(guard
            .orgs
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

    async fn list_workspace_members(
        &self,
        org_id: OrgId,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceMember>, ApiError> {
        let guard = self.state.read().await;
        Ok(guard
            .workspaces
            .get(&(org_id, workspace_id))
            .map(|members| {
                members
                    .values()
                    .map(|entry| WorkspaceMember {
                        principal: entry.principal.clone(),
                        role: entry.role,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn upsert_workspace_member(
        &self,
        org_id: OrgId,
        workspace_id: WorkspaceId,
        principal: &Principal,
        role: WorkspaceRole,
    ) -> Result<(), ApiError> {
        let mut guard = self.state.write().await;
        let key = principal_key(principal);
        if !guard
            .orgs
            .get(&org_id)
            .is_some_and(|members| members.contains_key(&key))
        {
            return Err(ApiError::NotFound("member not found".to_owned()));
        }
        guard
            .workspaces
            .entry((org_id, workspace_id))
            .or_default()
            .insert(
                key,
                WorkspaceEntry {
                    principal: principal.clone(),
                    role,
                },
            );
        Ok(())
    }

    async fn remove_workspace_member(
        &self,
        org_id: OrgId,
        workspace_id: WorkspaceId,
        principal: &Principal,
    ) -> Result<bool, ApiError> {
        let mut guard = self.state.write().await;
        Ok(guard
            .workspaces
            .get_mut(&(org_id, workspace_id))
            .is_some_and(|members| members.remove(&principal_key(principal)).is_some()))
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
        let mut guard = self.state.write().await;
        let key = principal_key(principal);
        let members = guard.orgs.entry(org_id).or_default();

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
        let mut guard = self.state.write().await;
        let key = principal_key(principal);
        let Some(members) = guard.orgs.get_mut(&org_id) else {
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
        for ((workspace_org_id, _), workspace_members) in &mut guard.workspaces {
            if *workspace_org_id == org_id {
                workspace_members.remove(&key);
            }
        }
        Ok(RemoveMemberOutcome::Removed)
    }

    async fn list_orgs_for_principal(
        &self,
        principal: &Principal,
    ) -> Result<Vec<(OrgId, OrgRole)>, ApiError> {
        let key = principal_key(principal);
        let guard = self.state.read().await;
        Ok(guard
            .orgs
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

    // ── seeded_bootstrap: the operator/integrator provisioning ctor ──────

    #[tokio::test]
    async fn seeded_bootstrap_grants_owner_and_parses_ids() {
        let owner = UserId::new();
        let org = OrgId::new();
        let store = InMemoryMembershipStore::seeded_bootstrap(&org.to_string(), &owner.to_string())
            .expect("valid prefixed-ULID identities must parse");

        assert_eq!(
            store
                .get_org_role(org, &Principal::User(owner))
                .await
                .unwrap(),
            Some(OrgRole::OrgOwner),
            "seeded_bootstrap must grant OrgOwner to the bootstrap owner"
        );
        // Nobody else, no other org.
        assert_eq!(
            store.get_org_role(org, &user()).await.unwrap(),
            None,
            "only the bootstrap owner is seeded"
        );
    }

    #[tokio::test]
    async fn tenant_membership_snapshot_resolves_roles_together() {
        let owner = UserId::new();
        let org = OrgId::new();
        let workspace = WorkspaceId::new();
        let store = InMemoryMembershipStore::seeded(org, Principal::User(owner), OrgRole::OrgOwner);

        assert_eq!(
            store
                .get_tenant_membership(org, Some(workspace), &Principal::User(owner))
                .await
                .unwrap(),
            TenantMembershipSnapshot {
                org_role: Some(OrgRole::OrgOwner),
                workspace_role: None,
            }
        );
    }

    #[test]
    fn seeded_bootstrap_rejects_malformed_org_id() {
        let err = InMemoryMembershipStore::seeded_bootstrap(
            "not-an-org-ulid",
            &UserId::new().to_string(),
        )
        .expect_err("a malformed org id must fail closed, not seed nothing");
        assert!(
            matches!(err, BootstrapSeedError::OrgId { .. }),
            "expected BootstrapSeedError::OrgId, got {err:?}"
        );
    }

    #[test]
    fn seeded_bootstrap_rejects_malformed_owner_id() {
        let err =
            InMemoryMembershipStore::seeded_bootstrap(&OrgId::new().to_string(), "not-a-usr-ulid")
                .expect_err("a malformed owner id must fail closed, not seed nothing");
        assert!(
            matches!(err, BootstrapSeedError::OwnerId { .. }),
            "expected BootstrapSeedError::OwnerId, got {err:?}"
        );
    }

    #[tokio::test]
    async fn add_then_get_role_is_visible() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let p = user();

        assert_eq!(store.get_org_role(org, &p).await.unwrap(), None);
        assert_eq!(
            store
                .add_member_guarded(org, &p, OrgRole::OrgAdmin)
                .await
                .unwrap(),
            AddMemberOutcome::Added,
        );
        assert_eq!(
            store.get_org_role(org, &p).await.unwrap(),
            Some(OrgRole::OrgAdmin),
            "guarded addition must be immediately visible to get_org_role (RBAC coherence)"
        );
    }

    #[tokio::test]
    async fn add_member_upserts_role() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let p = user();
        store
            .seed_for_test(org, p.clone(), OrgRole::OrgMember)
            .await;
        assert_eq!(
            store
                .add_member_guarded(org, &p, OrgRole::OrgAdmin)
                .await
                .unwrap(),
            AddMemberOutcome::Added,
        );
        let members = store.list_members(org).await.unwrap();
        assert_eq!(members.len(), 1, "upsert must not duplicate the row");
        assert_eq!(members[0].role, OrgRole::OrgAdmin);
    }

    #[tokio::test]
    async fn remove_member_reports_presence() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let p = user();
        assert_eq!(
            store.remove_member_guarded(org, &p).await.unwrap(),
            RemoveMemberOutcome::NotFound,
            "removing a non-member must report absence (handler → 404)"
        );
        store.seed_for_test(org, user(), OrgRole::OrgOwner).await;
        store
            .seed_for_test(org, p.clone(), OrgRole::OrgMember)
            .await;
        assert_eq!(
            store.remove_member_guarded(org, &p).await.unwrap(),
            RemoveMemberOutcome::Removed
        );
        assert_eq!(store.get_org_role(org, &p).await.unwrap(), None);
    }

    #[tokio::test]
    async fn workspace_upsert_requires_current_org_membership_at_write_seam() {
        let store = InMemoryMembershipStore::new();
        let org = OrgId::new();
        let workspace = WorkspaceId::new();
        let owner = user();
        let target = user();
        store.seed_for_test(org, owner, OrgRole::OrgOwner).await;

        let error = store
            .upsert_workspace_member(org, workspace, &target, WorkspaceRole::WorkspaceViewer)
            .await
            .expect_err("a workspace grant must not outlive its org membership authority");
        assert!(matches!(error, ApiError::NotFound(_)));
        assert!(
            store
                .list_workspace_members(org, workspace)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn list_orgs_for_principal_is_cross_org() {
        let store = InMemoryMembershipStore::new();
        let p = user();
        let org_a = OrgId::new();
        let org_b = OrgId::new();
        let other = user();
        store
            .seed_for_test(org_a, p.clone(), OrgRole::OrgOwner)
            .await;
        store
            .seed_for_test(org_b, p.clone(), OrgRole::OrgMember)
            .await;
        store
            .seed_for_test(org_b, other.clone(), OrgRole::OrgAdmin)
            .await;

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
            .seed_for_test(org_a, p.clone(), OrgRole::OrgAdmin)
            .await;
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
            .seed_for_test(org, owner.clone(), OrgRole::OrgOwner)
            .await;

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
            .seed_for_test(org, owner.clone(), OrgRole::OrgOwner)
            .await;
        store
            .seed_for_test(org, admin2.clone(), OrgRole::OrgAdmin)
            .await;

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
            .seed_for_test(org, owner.clone(), OrgRole::OrgOwner)
            .await;

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
            .seed_for_test(org, owner.clone(), OrgRole::OrgOwner)
            .await;
        store
            .seed_for_test(org, plain.clone(), OrgRole::OrgMember)
            .await;
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
            store.seed_for_test(org, a.clone(), OrgRole::OrgAdmin).await;
            store.seed_for_test(org, b.clone(), OrgRole::OrgAdmin).await;

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
            store.seed_for_test(org, a.clone(), OrgRole::OrgAdmin).await;
            store.seed_for_test(org, b.clone(), OrgRole::OrgAdmin).await;

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
