//! Atomic tenant provisioning: org + default workspace + owner grant in one
//! transaction under the org-id and org-slug advisory locks, replay-safe for
//! an identical request.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMembershipRole, TenantProvisioningConflict, TenantProvisioningOutcome,
    TenantProvisioningRequest,
};
use nebula_storage_port::store::TenantProvisioningStore;
use sqlx::PgPool;

use super::membership::org_role;
use super::org::{decode_org, insert_org};
use super::workspace::{decode_workspace, insert_workspace};
use super::{advisory_xact_lock, optional};
use crate::sql_error::storage_error;

/// PostgreSQL atomic tenant-provisioning store.
#[derive(Clone, Debug)]
pub struct PgTenantProvisioningStore {
    pool: PgPool,
}

impl PgTenantProvisioningStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const EXISTING_STATE: TenantProvisioningOutcome =
    TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState);

#[async_trait::async_trait]
impl TenantProvisioningStore for PgTenantProvisioningStore {
    #[tracing::instrument(skip_all)]
    async fn provision_tenant(
        &self,
        request: TenantProvisioningRequest,
    ) -> Result<TenantProvisioningOutcome, StorageError> {
        let org_values = request.org();
        let workspace_values = request.default_workspace();
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        // Backend-authored instants come from the transaction clock, as every
        // other PostgreSQL write here (`now()`).
        let created_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT now()")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage_error)?;
        let org = org_values.materialize(created_at);
        let workspace = workspace_values.materialize(org.id.clone(), created_at);
        // Sorted so concurrent provisioners acquire the pair in one order.
        let mut lock_keys = [
            format!("tenant-provisioning:id:{}", org.id),
            format!("tenant-provisioning:slug:{}", org.slug),
        ];
        lock_keys.sort();
        for key in &lock_keys {
            advisory_xact_lock(&mut tx, key).await?;
        }
        let org_rows = sqlx::query(
            "SELECT * FROM orgs WHERE id = $1 OR (slug = $2 AND deleted_at IS NULL) FOR UPDATE",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let workspace_rows = sqlx::query(
            "SELECT * FROM workspaces \
             WHERE id = $2 OR (org_id = $1 AND (slug = $3 OR is_default) AND deleted_at IS NULL) \
             FOR UPDATE",
        )
        .bind(&org.id)
        .bind(&workspace.id)
        .bind(&workspace.slug)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let owner_row = sqlx::query(
            "SELECT role, added_by FROM org_memberships \
             WHERE org_id = $1 AND principal_kind = $2 AND principal_id = $3 FOR UPDATE",
        )
        .bind(&org.id)
        .bind(request.owner_principal_kind().as_str())
        .bind(request.owner_principal_id())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;

        let exact_org = match org_rows.as_slice() {
            [only] => org_values.matches_persisted(&decode_org(only)?),
            _ => false,
        };
        let exact_workspace = match workspace_rows.as_slice() {
            [only] => workspace_values.matches_persisted(&org.id, &decode_workspace(only)?),
            _ => false,
        };
        let exact_owner = match &owner_row {
            Some(row) => {
                org_role(row)? == OrgMembershipRole::Owner
                    && optional::<String>(row, "added_by")?.as_deref() == request.owner_added_by()
            },
            None => false,
        };
        if exact_org && exact_workspace && exact_owner {
            return Ok(TenantProvisioningOutcome::Replayed);
        }
        if !org_rows.is_empty() || !workspace_rows.is_empty() || owner_row.is_some() {
            return Ok(EXISTING_STATE);
        }

        // Another writer may take the org id, slug or the workspace id first.
        match insert_org(&mut *tx, &org).await {
            Ok(()) => {},
            Err(StorageError::Duplicate { .. }) => return Ok(EXISTING_STATE),
            Err(error) => return Err(error),
        }
        match insert_workspace(&mut tx, &workspace).await {
            Ok(()) => {},
            Err(StorageError::Duplicate { .. }) => return Ok(EXISTING_STATE),
            Err(error) => return Err(error),
        }
        sqlx::query(
            "INSERT INTO org_memberships \
             (org_id, principal_kind, principal_id, role, added_by, added_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&org.id)
        .bind(request.owner_principal_kind().as_str())
        .bind(request.owner_principal_id())
        .bind(OrgMembershipRole::Owner.as_str())
        .bind(request.owner_added_by())
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(TenantProvisioningOutcome::Created)
    }
}
