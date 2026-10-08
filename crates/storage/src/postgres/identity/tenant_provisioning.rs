//! Atomic tenant provisioning: org + default workspace + owner grant in one
//! transaction under ordered org-id, slug and workspace-id advisory locks.
//! Permanent request receipts keep replay independent of current tenant state.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMembershipRole, TenantProvisioningConflict, TenantProvisioningOutcome,
    TenantProvisioningRequest,
};
use nebula_storage_port::store::TenantProvisioningStore;
use sqlx::{PgPool, Row};

use super::advisory_xact_lock;
use super::org::insert_org;
use super::workspace::insert_workspace;
use crate::sql_error::storage_error;
use crate::tenant_provisioning::{REQUEST_VERSION, replay_outcome, request_digest};

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
        let digest = request_digest(&request)?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        // Backend-authored instants come from the transaction clock, as every
        // other PostgreSQL write here (`now()`).
        let created_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT now()")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage_error)?;
        let org = org_values.materialize(created_at);
        let workspace = workspace_values.materialize(org.id.clone(), created_at);
        // Stable order also serializes attempts to reuse a purged workspace ID.
        let mut lock_keys = [
            format!("tenant-provisioning:id:{}", org.id),
            format!("tenant-provisioning:slug:{}", org.slug),
            format!("tenant-provisioning:workspace:{}", workspace.id),
        ];
        lock_keys.sort();
        for key in &lock_keys {
            advisory_xact_lock(&mut tx, key).await?;
        }
        let receipt = sqlx::query(
            "SELECT request_version, request_digest, initial_workspace_id \
             FROM tenant_provisioning_receipts WHERE org_id = $1",
        )
        .bind(&org.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        if let Some(receipt) = receipt {
            return replay_outcome(
                receipt.try_get("request_version").map_err(storage_error)?,
                receipt.try_get("request_digest").map_err(storage_error)?,
                receipt
                    .try_get("initial_workspace_id")
                    .map_err(storage_error)?,
                &digest,
            );
        }
        // A fresh org cannot have children: the tenant FKs already enforce it.
        // Ordinary writers need not acquire our advisory locks; unique indexes
        // arbitrate any collision that occurs after this snapshot.
        let occupied: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM orgs WHERE id = $1 OR (slug = $2 AND deleted_at IS NULL)) \
                OR EXISTS (SELECT 1 FROM workspaces WHERE id = $3) \
                OR EXISTS (SELECT 1 FROM tenant_provisioning_receipts WHERE initial_workspace_id = $3)",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .bind(&workspace.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage_error)?;
        if occupied {
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
        sqlx::query(
            "INSERT INTO tenant_provisioning_receipts \
             (org_id, initial_workspace_id, request_version, request_digest, recorded_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&org.id)
        .bind(&workspace.id)
        .bind(REQUEST_VERSION)
        .bind(digest.as_slice())
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(TenantProvisioningOutcome::Created)
    }
}
