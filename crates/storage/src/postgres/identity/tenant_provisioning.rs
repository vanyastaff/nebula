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
use crate::auth::{
    InitialOwnerStatus,
    initial_owner::{Enrollment, decode_enrollment},
};
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

    /// Accept the identity owner's frozen enrollment while locking its live account.
    /// Matching receipts remain historical even after account or tenant removal.
    ///
    /// # Errors
    /// Corrupt enrollment or database failures return value-free storage errors.
    #[tracing::instrument(skip_all)]
    pub async fn accept_initial_owner(&self) -> Result<InitialOwnerStatus, StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
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

    /// Observe enrollment and its receipt in one repeatable, read-only snapshot.
    ///
    /// # Errors
    /// Returns a storage error if the snapshot cannot be read or decoded.
    #[tracing::instrument(skip_all)]
    pub async fn initial_owner_status(&self) -> Result<InitialOwnerStatus, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(storage_error)?;
        let status = match read_enrollment(&mut tx).await? {
            Enrollment::Available => InitialOwnerStatus::Available,
            Enrollment::Sealed => InitialOwnerStatus::Sealed,
            Enrollment::Pending { user_id, request } => {
                match read_receipt(&mut tx, &request).await? {
                    Some(TenantProvisioningOutcome::Replayed) => InitialOwnerStatus::Accepted,
                    Some(_) => InitialOwnerStatus::Conflict,
                    None => {
                        let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)")
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Enrollment, StorageError> {
    let row = sqlx::query(
        "SELECT state, user_id, tenant_request FROM initial_owner_enrollment WHERE singleton = 1",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(storage_error)?;
    decode_enrollment(
        row.try_get("state").map_err(storage_error)?,
        row.try_get("user_id").map_err(storage_error)?,
        row.try_get("tenant_request").map_err(storage_error)?,
    )
}

async fn read_receipt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &TenantProvisioningRequest,
) -> Result<Option<TenantProvisioningOutcome>, StorageError> {
    let row = sqlx::query("SELECT request_version, request_digest, initial_workspace_id FROM tenant_provisioning_receipts WHERE org_id = $1")
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

const EXISTING_STATE: TenantProvisioningOutcome =
    TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState);

async fn identities_occupied(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &TenantProvisioningRequest,
) -> Result<bool, StorageError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM orgs WHERE id = $1 OR (slug = $2 AND deleted_at IS NULL))
            OR EXISTS (SELECT 1 FROM workspaces WHERE id = $3)
            OR EXISTS (SELECT 1 FROM tenant_provisioning_receipts WHERE initial_workspace_id = $3)",
    )
    .bind(request.org().id())
    .bind(request.org().slug())
    .bind(request.default_workspace().id())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage_error)
}

#[async_trait::async_trait]
impl TenantProvisioningStore for PgTenantProvisioningStore {
    #[tracing::instrument(skip_all)]
    async fn provision_tenant(
        &self,
        request: TenantProvisioningRequest,
    ) -> Result<TenantProvisioningOutcome, StorageError> {
        let tx = self.pool.begin().await.map_err(storage_error)?;
        provision_in_transaction(tx, request, None).await
    }
}

async fn provision_in_transaction(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    request: TenantProvisioningRequest,
    enrollment_owner: Option<[u8; 16]>,
) -> Result<TenantProvisioningOutcome, StorageError> {
    let org_values = request.org();
    let workspace_values = request.default_workspace();
    let digest = request_digest(&request)?;
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
    if let Some(outcome) = read_receipt(&mut tx, &request).await? {
        return Ok(outcome);
    }
    if let Some(user_id) = enrollment_owner {
        let live: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR SHARE",
        )
        .bind(user_id.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        if live.is_none() {
            return Err(StorageError::NotFound {
                entity: "user",
                id: "initial owner".into(),
            });
        }
    }
    // A fresh org cannot have children: the tenant FKs already enforce it.
    // Ordinary writers need not acquire our advisory locks; unique indexes
    // arbitrate any collision that occurs after this snapshot.
    if identities_occupied(&mut tx, &request).await? {
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
