//! Atomic tenant provisioning: org + default workspace + owner grant in one
//! `BEGIN IMMEDIATE` transaction, replay-safe for an identical request.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMembershipRole, TenantProvisioningConflict, TenantProvisioningOutcome,
    TenantProvisioningRequest,
};
use nebula_storage_port::store::TenantProvisioningStore;
use sqlx::{Row, SqlitePool};

use super::encode_instant;
use super::org::insert_org;
use super::workspace::insert_workspace;
use crate::sql_error::storage_error;
use crate::tenant_provisioning::{REQUEST_VERSION, replay_outcome, request_digest};

/// SQLite atomic tenant-provisioning store.
#[derive(Clone, Debug)]
pub struct SqliteTenantProvisioningStore {
    pool: SqlitePool,
}

impl SqliteTenantProvisioningStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl TenantProvisioningStore for SqliteTenantProvisioningStore {
    #[tracing::instrument(skip_all)]
    async fn provision_tenant(
        &self,
        request: TenantProvisioningRequest,
    ) -> Result<TenantProvisioningOutcome, StorageError> {
        let org_values = request.org();
        let workspace_values = request.default_workspace();
        let digest = request_digest(&request)?;
        let created_at = chrono::Utc::now();
        let org = org_values.materialize(created_at);
        let workspace = workspace_values.materialize(org.id.clone(), created_at);
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let receipt = sqlx::query(
            "SELECT request_version, request_digest, initial_workspace_id \
             FROM tenant_provisioning_receipts WHERE org_id = ?1",
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
        let occupied: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM orgs WHERE id = ?1 OR (slug = ?2 AND deleted_at IS NULL)) \
                OR EXISTS (SELECT 1 FROM workspaces WHERE id = ?3) \
                OR EXISTS (SELECT 1 FROM tenant_provisioning_receipts WHERE initial_workspace_id = ?3)",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .bind(&workspace.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage_error)?;
        if occupied {
            return Ok(TenantProvisioningOutcome::Conflict(
                TenantProvisioningConflict::ExistingState,
            ));
        }

        insert_org(&mut *tx, &org).await?;
        insert_workspace(&mut tx, &workspace).await?;
        sqlx::query(
            "INSERT INTO org_memberships \
             (org_id, principal_kind, principal_id, role, added_by, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&org.id)
        .bind(request.owner_principal_kind().as_str())
        .bind(request.owner_principal_id())
        .bind(OrgMembershipRole::Owner.as_str())
        .bind(request.owner_added_by())
        .bind(encode_instant(created_at))
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        sqlx::query(
            "INSERT INTO tenant_provisioning_receipts \
             (org_id, initial_workspace_id, request_version, request_digest, recorded_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&org.id)
        .bind(&workspace.id)
        .bind(REQUEST_VERSION)
        .bind(digest.as_slice())
        .bind(encode_instant(created_at))
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(TenantProvisioningOutcome::Created)
    }
}
