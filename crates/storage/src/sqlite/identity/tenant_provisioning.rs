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
use crate::auth::{
    InitialOwnerStatus,
    initial_owner::{Enrollment, decode_enrollment},
};
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

    /// Accept only the identity owner's frozen enrollment command. Replay is historical.
    /// No supplied account or command can replace the saved enrollment.
    ///
    /// # Errors
    /// Corrupt enrollment or database failures return value-free storage errors.
    #[tracing::instrument(skip_all)]
    pub async fn accept_initial_owner(&self) -> Result<InitialOwnerStatus, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let (user_id, request) = match read_enrollment(&mut tx).await? {
            Enrollment::Available => return Ok(InitialOwnerStatus::Available),
            Enrollment::Sealed => return Ok(InitialOwnerStatus::Sealed),
            Enrollment::Pending { user_id, request } => (user_id, request),
        };
        match provision_in_transaction(tx, *request, Some(user_id)).await {
            Ok(TenantProvisioningOutcome::Created | TenantProvisioningOutcome::Replayed) => {
                Ok(InitialOwnerStatus::Accepted)
            },
            Ok(TenantProvisioningOutcome::Conflict(_)) => Ok(InitialOwnerStatus::Conflict),
            Err(StorageError::NotFound { entity: "user", .. }) => {
                Ok(InitialOwnerStatus::OwnerUnavailable)
            },
            Err(error) => Err(error),
        }
    }

    /// Observe enrollment and its historical receipt in one database snapshot.
    ///
    /// # Errors
    /// Returns a storage error if the snapshot cannot be read or decoded.
    #[tracing::instrument(skip_all)]
    pub async fn initial_owner_status(&self) -> Result<InitialOwnerStatus, StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let status = match read_enrollment(&mut tx).await? {
            Enrollment::Available => InitialOwnerStatus::Available,
            Enrollment::Sealed => InitialOwnerStatus::Sealed,
            Enrollment::Pending { user_id, request } => {
                match read_receipt(&mut tx, &request).await? {
                    Some(TenantProvisioningOutcome::Replayed) => InitialOwnerStatus::Accepted,
                    Some(_) => InitialOwnerStatus::Conflict,
                    None => {
                        let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = ? AND deleted_at IS NULL)")
                            .bind(user_id.as_slice()).fetch_one(&mut *tx).await.map_err(storage_error)?;
                        if !live {
                            InitialOwnerStatus::OwnerUnavailable
                        } else if identities_occupied(&mut tx, &request).await? {
                            InitialOwnerStatus::Conflict
                        } else {
                            InitialOwnerStatus::Pending
                        }
                    },
                }
            },
        };
        tx.commit().await.map_err(storage_error)?;
        tracing::debug!(?status, "initial owner enrollment observed");
        Ok(status)
    }
}

async fn read_enrollment(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<Enrollment, StorageError> {
    let row = sqlx::query(
        "SELECT state, user_id, tenant_request FROM initial_owner_enrollment WHERE singleton = 1",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(storage_error)?;
    let request: Option<sqlx::types::Json<serde_json::Value>> =
        row.try_get("tenant_request").map_err(storage_error)?;
    decode_enrollment(
        row.try_get("state").map_err(storage_error)?,
        row.try_get("user_id").map_err(storage_error)?,
        request.map(|value| value.0),
    )
}

async fn read_receipt(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    request: &TenantProvisioningRequest,
) -> Result<Option<TenantProvisioningOutcome>, StorageError> {
    let row = sqlx::query("SELECT request_version, request_digest, initial_workspace_id FROM tenant_provisioning_receipts WHERE org_id = ?")
        .bind(request.org().id()).fetch_optional(&mut **tx).await.map_err(storage_error)?;
    row.map(|row| {
        replay_outcome(
            row.try_get("request_version").map_err(storage_error)?,
            row.try_get("request_digest").map_err(storage_error)?,
            row.try_get("initial_workspace_id").map_err(storage_error)?,
            &request_digest(request)?,
        )
    })
    .transpose()
}

#[async_trait::async_trait]
impl TenantProvisioningStore for SqliteTenantProvisioningStore {
    #[tracing::instrument(skip_all)]
    async fn provision_tenant(
        &self,
        request: TenantProvisioningRequest,
    ) -> Result<TenantProvisioningOutcome, StorageError> {
        let tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        provision_in_transaction(tx, request, None).await
    }
}

async fn identities_occupied(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    request: &TenantProvisioningRequest,
) -> Result<bool, StorageError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM orgs WHERE id = ?1 OR (slug = ?2 AND deleted_at IS NULL))
            OR EXISTS (SELECT 1 FROM workspaces WHERE id = ?3)
            OR EXISTS (SELECT 1 FROM tenant_provisioning_receipts WHERE initial_workspace_id = ?3)",
    )
    .bind(request.org().id())
    .bind(request.org().slug())
    .bind(request.default_workspace().id())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage_error)
}

async fn provision_in_transaction(
    mut tx: sqlx::Transaction<'_, sqlx::Sqlite>,
    request: TenantProvisioningRequest,
    enrollment_owner: Option<[u8; 16]>,
) -> Result<TenantProvisioningOutcome, StorageError> {
    let org_values = request.org();
    let workspace_values = request.default_workspace();
    let digest = request_digest(&request)?;
    let created_at = chrono::Utc::now();
    let org = org_values.materialize(created_at);
    let workspace = workspace_values.materialize(org.id.clone(), created_at);
    if let Some(outcome) = read_receipt(&mut tx, &request).await? {
        return Ok(outcome);
    }
    if let Some(user_id) = enrollment_owner {
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = ? AND deleted_at IS NULL)",
        )
        .bind(user_id.as_slice())
        .fetch_one(&mut *tx)
        .await
        .map_err(storage_error)?;
        if !live {
            return Err(StorageError::NotFound {
                entity: "user",
                id: "initial owner".into(),
            });
        }
    }
    // A fresh org cannot have children: the tenant FKs already enforce it.
    if identities_occupied(&mut tx, &request).await? {
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
