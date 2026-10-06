//! Atomic tenant provisioning: org + default workspace + owner grant in one
//! `BEGIN IMMEDIATE` transaction, replay-safe for an identical request.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMembershipRole, TenantProvisioningConflict, TenantProvisioningOutcome,
    TenantProvisioningRequest,
};
use nebula_storage_port::store::TenantProvisioningStore;
use sqlx::SqlitePool;

use super::membership::decode_membership;
use super::now_rfc3339;
use super::org::{decode_org, insert_org};
use super::workspace::{decode_workspace, insert_workspace};
use crate::sql_error::storage_error;

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
        let created_at = now_rfc3339();
        let org = org_values.materialize(created_at.clone());
        let workspace = workspace_values.materialize(org.id.clone(), created_at);
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let org_rows = sqlx::query(
            "SELECT * FROM port_orgs WHERE id = ?1 OR (slug = ?2 AND deleted_at IS NULL)",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let workspace_rows = sqlx::query(
            "SELECT * FROM port_workspaces \
             WHERE id = ?2 OR (org_id = ?1 AND (slug = ?3 OR is_default = 1) AND deleted_at IS NULL)",
        )
        .bind(&org.id)
        .bind(&workspace.id)
        .bind(&workspace.slug)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let owner_row = sqlx::query(
            "SELECT * FROM port_memberships \
             WHERE scope_kind = 'org' AND scope_id = ?1 AND principal_kind = ?2 AND principal_id = ?3",
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
        let exact_owner = owner_row
            .as_ref()
            .map(decode_membership)
            .transpose()?
            .is_some_and(|row| {
                row.role == OrgMembershipRole::Owner.as_str()
                    && row.added_by.as_deref() == request.owner_added_by()
            });
        if exact_org && exact_workspace && exact_owner {
            return Ok(TenantProvisioningOutcome::Replayed);
        }
        if !org_rows.is_empty() || !workspace_rows.is_empty() || owner_row.is_some() {
            return Ok(TenantProvisioningOutcome::Conflict(
                TenantProvisioningConflict::ExistingState,
            ));
        }

        insert_org(&mut *tx, &org).await?;
        insert_workspace(&mut tx, &workspace).await?;
        sqlx::query(
            "INSERT INTO port_memberships \
             (scope_kind, scope_id, principal_kind, principal_id, role, added_at, added_by) \
             VALUES ('org', ?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&org.id)
        .bind(request.owner_principal_kind().as_str())
        .bind(request.owner_principal_id())
        .bind(OrgMembershipRole::Owner.as_str())
        .bind(now_rfc3339())
        .bind(request.owner_added_by())
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(TenantProvisioningOutcome::Created)
    }
}
