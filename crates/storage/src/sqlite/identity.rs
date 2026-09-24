//! SQLite identity-zoo stores over the port-scoped schema.
//!
//! Each aggregate is a `port_*` table in the ordered SQLite migrations. Every tenant- or
//! parent-scoped query carries its scope predicate (`WHERE org_id = ?`,
//! `WHERE workspace_id = ? AND org_id = ?`, …) and active-row reads add
//! `AND deleted_at IS NULL`, so a cross-scope `get` yields `Ok(None)` and
//! a cross-scope `update` / `soft_delete` is `NotFound` — an id outside
//! the caller's scope is indistinguishable from one that does not exist
//! (no existence oracle, spec §6.1), exactly as the in-memory backend
//! behaves.
//!
//! First-writer-wins uniqueness (email / slug among *active* rows) is a
//! partial unique index `WHERE deleted_at IS NULL`, so a soft-deleted row
//! frees its key. Optimistic CAS is a single conditional `UPDATE … WHERE
//! version = ?` followed by a disambiguating read (zero rows ⇒ the row is
//! gone → `NotFound`, or the version moved → `Conflict`). JSON columns are
//! opaque TEXT round-tripped through `serde_json`; binary columns are
//! `BLOB`.

use std::sync::Arc;

use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TriggerRow, UserRow, WorkspaceRow,
};
use nebula_storage_port::dto::{
    OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembershipRole,
    PrincipalOrgMembership, TenantMembershipSnapshot, TenantProvisioningConflict,
    TenantProvisioningOutcome, TenantProvisioningRequest, WorkspaceMemberUpsert,
    WorkspaceMembership, WorkspaceMembershipRole,
};
use nebula_storage_port::store::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore,
    TenantProvisioningStore, TriggerStore, UserStore, WorkspaceStore,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::execution::conn_err;

/// Decode a NOT NULL column, returning `Err` when the column value is SQL NULL.
///
/// sqlx's SQLite backend maps NULL to the zero-value for scalar types (`""`
/// for `String`, `0` for `i64`, …) because the SQLite C API returns 0/empty
/// when `sqlite3_value_*` is called on a NULL cell. Calling `try_get::<T>`
/// therefore returns `Ok(default)` on NULL — the error never fires, so a plain
/// `.map_err(conn_err)?` silently accepts NULL as the default. We must decode
/// as `Option<T>` instead (sqlx correctly yields `None` for NULL regardless of
/// the inner type) and reject `None` explicitly.
fn required<'r, T>(row: &'r sqlx::sqlite::SqliteRow, col: &'static str) -> Result<T, StorageError>
where
    T: sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    row.try_get::<Option<T>, _>(col)
        .map_err(conn_err)?
        .ok_or_else(|| {
            StorageError::Connection(format!(
                "NOT NULL column '{col}' contained SQL NULL (schema/data inconsistency)"
            ))
        })
}

fn optional<'r, T>(
    row: &'r sqlx::sqlite::SqliteRow,
    col: &'static str,
) -> Result<Option<T>, StorageError>
where
    T: sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    row.try_get::<Option<T>, _>(col).map_err(conn_err)
}

fn json_to_text(v: &serde_json::Value) -> String {
    v.to_string()
}

fn text_to_json(s: &str) -> Result<serde_json::Value, StorageError> {
    serde_json::from_str(s).map_err(|e| StorageError::Serialization(e.to_string()))
}

fn opt_text_to_json(s: Option<String>) -> Result<Option<serde_json::Value>, StorageError> {
    match s {
        // A NULL column round-trips as `None`; treat an empty string the
        // same as NULL so a row stored with no JSON payload reads back as
        // `None` rather than tripping a serde EOF.
        Some(raw) if !raw.is_empty() => Ok(Some(text_to_json(&raw)?)),
        _ => Ok(None),
    }
}

// ── Users ─────────────────────────────────────────────────────────────────

/// SQLite-backed `users` store. Email is unique among active rows
/// (case-insensitive, via `lower(email)` partial unique index).
#[derive(Clone, Debug)]
pub struct SqliteUserStore {
    pool: SqlitePool,
}

impl SqliteUserStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn user_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<Arc<UserRow>, StorageError> {
    Ok(Arc::new(UserRow {
        // NOT NULL columns — `required` decodes via Option<T> and rejects NULL;
        // a bare `try_get::<String>` silently yields "" for NULL in SQLite.
        id: required(r, "id")?,
        email: required(r, "email")?,
        display_name: required(r, "display_name")?,
        created_at: required(r, "created_at")?,
        failed_login_count: i32::try_from(required::<i64>(r, "failed_login_count")?).map_err(
            |e| StorageError::Serialization(format!("failed_login_count out of i32 range: {e}")),
        )?,
        mfa_enabled: required::<i64>(r, "mfa_enabled")? != 0,
        version: required::<i64>(r, "version")? as u64,
        // Nullable columns — .ok() / Option decode is correct.
        email_verified_at: r.try_get("email_verified_at").ok(),
        avatar_url: r.try_get("avatar_url").ok(),
        password_hash: r.try_get("password_hash").ok(),
        last_login_at: r.try_get("last_login_at").ok(),
        locked_until: r.try_get("locked_until").ok(),
        mfa_secret_envelope: r.try_get("mfa_secret").ok(),
        deleted_at: r.try_get("deleted_at").ok(),
    }))
}

#[async_trait::async_trait]
impl UserStore for SqliteUserStore {
    async fn create(&self, row: UserRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_users (id, email, email_verified_at, display_name, \
             avatar_url, password_hash, created_at, last_login_at, locked_until, \
             failed_login_count, mfa_enabled, mfa_secret, version, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.email)
        .bind(&row.email_verified_at)
        .bind(&row.display_name)
        .bind(&row.avatar_url)
        .bind(&row.password_hash)
        .bind(&row.created_at)
        .bind(&row.last_login_at)
        .bind(&row.locked_until)
        .bind(i64::from(row.failed_login_count))
        .bind(i64::from(row.mfa_enabled))
        .bind(&row.mfa_secret_envelope)
        .bind(row.version as i64)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "user",
                    detail: format!("user {} or its active email already exists", row.id),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(&self, id: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_users WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(user_from_row).transpose()
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        let row = sqlx::query(
            "SELECT * FROM port_users \
             WHERE lower(email) = lower(?) AND deleted_at IS NULL",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(user_from_row).transpose()
    }

    async fn update(&self, row: UserRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_users SET email = ?, email_verified_at = ?, \
             display_name = ?, avatar_url = ?, password_hash = ?, \
             last_login_at = ?, locked_until = ?, failed_login_count = ?, \
             mfa_enabled = ?, mfa_secret = ?, version = ? \
             WHERE id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.email)
        .bind(&row.email_verified_at)
        .bind(&row.display_name)
        .bind(&row.avatar_url)
        .bind(&row.password_hash)
        .bind(&row.last_login_at)
        .bind(&row.locked_until)
        .bind(i64::from(row.failed_login_count))
        .bind(i64::from(row.mfa_enabled))
        .bind(&row.mfa_secret_envelope)
        .bind(row.version as i64)
        .bind(&row.id)
        .bind(expected_version as i64)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        cas_disambiguate(&self.pool, "port_users", "user", &row.id, expected_version).await
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        soft_delete_by_id(&self.pool, "port_users", "user", id).await
    }
}

// ── Orgs ──────────────────────────────────────────────────────────────────

/// SQLite-backed `orgs` store. Slug is unique among active rows.
#[derive(Clone, Debug)]
pub struct SqliteOrgStore {
    pool: SqlitePool,
}

impl SqliteOrgStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn org_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<OrgRow, StorageError> {
    Ok(OrgRow {
        // NOT NULL columns — use `required` to reject SQL NULL (see its doc).
        id: required(r, "id")?,
        slug: required(r, "slug")?,
        display_name: required(r, "display_name")?,
        created_at: required(r, "created_at")?,
        created_by: required(r, "created_by")?,
        plan: required(r, "plan")?,
        settings: text_to_json(&required::<String>(r, "settings")?)?,
        version: required::<i64>(r, "version")? as u64,
        // Nullable columns.
        billing_email: optional(r, "billing_email")?,
        deleted_at: optional(r, "deleted_at")?,
    })
}

#[async_trait::async_trait]
impl OrgStore for SqliteOrgStore {
    async fn create(&self, row: OrgRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_orgs (id, slug, display_name, created_at, created_by, \
             plan, billing_email, settings, version, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(&row.plan)
        .bind(&row.billing_email)
        .bind(json_to_text(&row.settings))
        .bind(row.version as i64)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "org",
                    detail: format!("org {} or its active slug already exists", row.id),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(&self, id: &str) -> Result<Option<OrgRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_orgs WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(org_from_row).transpose()
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_orgs WHERE slug = ? AND deleted_at IS NULL")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(org_from_row).transpose()
    }

    async fn update(&self, row: OrgRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_orgs SET slug = ?, display_name = ?, plan = ?, \
             billing_email = ?, settings = ?, version = ? \
             WHERE id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.plan)
        .bind(&row.billing_email)
        .bind(json_to_text(&row.settings))
        .bind(row.version as i64)
        .bind(&row.id)
        .bind(expected_version as i64)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        cas_disambiguate(&self.pool, "port_orgs", "org", &row.id, expected_version).await
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        soft_delete_by_id(&self.pool, "port_orgs", "org", id).await
    }
}

// ── Workspaces ────────────────────────────────────────────────────────────

/// SQLite-backed `workspaces` store (scoped by parent org). Slug is
/// unique among active rows *per org*.
#[derive(Clone, Debug)]
pub struct SqliteWorkspaceStore {
    pool: SqlitePool,
}

impl SqliteWorkspaceStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn workspace_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<WorkspaceRow, StorageError> {
    Ok(WorkspaceRow {
        // NOT NULL columns.
        id: required(r, "id")?,
        org_id: required(r, "org_id")?,
        slug: required(r, "slug")?,
        display_name: required(r, "display_name")?,
        created_at: required(r, "created_at")?,
        created_by: required(r, "created_by")?,
        is_default: required::<i64>(r, "is_default")? != 0,
        settings: text_to_json(&required::<String>(r, "settings")?)?,
        version: required::<i64>(r, "version")? as u64,
        // Nullable columns.
        description: optional(r, "description")?,
        deleted_at: optional(r, "deleted_at")?,
    })
}

async fn reject_second_active_default(
    connection: &mut SqliteConnection,
    row: &WorkspaceRow,
) -> Result<(), StorageError> {
    if !row.is_default || row.deleted_at.is_some() {
        return Ok(());
    }
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT id FROM port_workspaces \
         WHERE org_id = ?1 AND is_default = 1 AND deleted_at IS NULL AND id <> ?2",
    )
    .bind(&row.org_id)
    .bind(&row.id)
    .fetch_optional(connection)
    .await
    .map_err(conn_err)?;
    if let Some(existing_id) = existing {
        return Err(StorageError::Duplicate {
            entity: "workspace",
            detail: format!(
                "organization {} already has active default workspace {existing_id}",
                row.org_id
            ),
        });
    }
    Ok(())
}

#[async_trait::async_trait]
impl WorkspaceStore for SqliteWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conn_err)?;
        reject_second_active_default(&mut tx, &row).await?;
        let res = sqlx::query(
            "INSERT INTO port_workspaces (id, org_id, slug, display_name, \
             description, created_at, created_by, is_default, settings, version, \
             deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.org_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.description)
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(i64::from(row.is_default))
        .bind(json_to_text(&row.settings))
        .bind(row.version as i64)
        .bind(&row.deleted_at)
        .execute(&mut *tx)
        .await;
        match res {
            Ok(_) => {
                tx.commit().await.map_err(conn_err)?;
                Ok(())
            },
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "workspace",
                    detail: format!(
                        "workspace {} or its active slug in org {} already exists",
                        row.id, row.org_id
                    ),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(&self, org_id: &str, id: &str) -> Result<Option<WorkspaceRow>, StorageError> {
        let row = sqlx::query(
            "SELECT * FROM port_workspaces \
             WHERE org_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(workspace_from_row).transpose()
    }

    async fn get_by_slug(
        &self,
        org_id: &str,
        slug: &str,
    ) -> Result<Option<WorkspaceRow>, StorageError> {
        let row = sqlx::query(
            "SELECT * FROM port_workspaces \
             WHERE org_id = ? AND slug = ? AND deleted_at IS NULL",
        )
        .bind(org_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(workspace_from_row).transpose()
    }

    async fn list_for_org(&self, org_id: &str) -> Result<Vec<WorkspaceRow>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM port_workspaces \
             WHERE org_id = ? AND deleted_at IS NULL ORDER BY id",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter().map(workspace_from_row).collect()
    }

    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conn_err)?;
        let res = sqlx::query(
            "UPDATE port_workspaces SET slug = ?, display_name = ?, \
             description = ?, is_default = ?, settings = ?, version = ? \
             WHERE org_id = ? AND id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.description)
        .bind(i64::from(row.is_default))
        .bind(json_to_text(&row.settings))
        .bind(row.version as i64)
        .bind(&row.org_id)
        .bind(&row.id)
        .bind(expected_version as i64)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            reject_second_active_default(&mut tx, &row).await?;
            tx.commit().await.map_err(conn_err)?;
            return Ok(());
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM port_workspaces WHERE org_id = ? AND id = ?",
        )
        .bind(&row.org_id)
        .bind(&row.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;
        match current {
            Some(actual) => Err(StorageError::Conflict {
                entity: "workspace",
                id: row.id,
                expected: expected_version,
                actual: actual as u64,
            }),
            None => Err(StorageError::not_found("workspace", row.id)),
        }
    }

    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conn_err)?;
        let res = sqlx::query(
            "UPDATE port_workspaces SET deleted_at = ? \
             WHERE org_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(now_rfc3339())
        .bind(org_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            tx.commit().await.map_err(conn_err)?;
            Ok(())
        } else {
            Err(StorageError::not_found("workspace", id))
        }
    }
}

/// SQLite atomic tenant-provisioning store.
#[derive(Clone, Debug)]
pub struct SqliteTenantProvisioningStore {
    pool: SqlitePool,
}

impl SqliteTenantProvisioningStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
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
            .map_err(conn_err)?;
        let org_rows = sqlx::query(
            "SELECT * FROM port_orgs WHERE id = ?1 OR (slug = ?2 AND deleted_at IS NULL)",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .fetch_all(&mut *tx)
        .await
        .map_err(conn_err)?;
        let workspace_rows = sqlx::query(
            "SELECT * FROM port_workspaces WHERE id = ?2 OR (org_id = ?1 AND (slug = ?3 OR is_default = 1) AND deleted_at IS NULL)",
        )
        .bind(&org.id)
        .bind(&workspace.id)
        .bind(&workspace.slug)
        .fetch_all(&mut *tx)
        .await
        .map_err(conn_err)?;
        let owner_row = sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = 'org' AND scope_id = ?1 AND principal_kind = ?2 AND principal_id = ?3",
        )
        .bind(&org.id)
        .bind(request.owner_principal_kind().as_str())
        .bind(request.owner_principal_id())
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;

        let exact_org =
            org_rows.len() == 1 && org_values.matches_persisted(&org_from_row(&org_rows[0])?);
        let exact_workspace = workspace_rows.len() == 1
            && workspace_values
                .matches_persisted(&org.id, &workspace_from_row(&workspace_rows[0])?);
        let exact_owner = owner_row
            .as_ref()
            .map(membership_from_row)
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

        sqlx::query(
            "INSERT INTO port_orgs (id, slug, display_name, created_at, created_by, plan, billing_email, settings, version, deleted_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .bind(&org.display_name)
        .bind(&org.created_at)
        .bind(&org.created_by)
        .bind(&org.plan)
        .bind(&org.billing_email)
        .bind(json_to_text(&org.settings))
        .bind(org.version as i64)
        .bind(&org.deleted_at)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        sqlx::query(
            "INSERT INTO port_workspaces (id, org_id, slug, display_name, description, created_at, created_by, is_default, settings, version, deleted_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind(&workspace.id)
        .bind(&workspace.org_id)
        .bind(&workspace.slug)
        .bind(&workspace.display_name)
        .bind(&workspace.description)
        .bind(&workspace.created_at)
        .bind(&workspace.created_by)
        .bind(i64::from(workspace.is_default))
        .bind(json_to_text(&workspace.settings))
        .bind(workspace.version as i64)
        .bind(&workspace.deleted_at)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        sqlx::query(
            "INSERT INTO port_memberships (scope_kind, scope_id, principal_kind, principal_id, role, added_at, added_by) VALUES ('org', ?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&org.id)
        .bind(request.owner_principal_kind().as_str())
        .bind(request.owner_principal_id())
        .bind(OrgMembershipRole::Owner.as_str())
        .bind(now_rfc3339())
        .bind(request.owner_added_by())
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        Ok(TenantProvisioningOutcome::Created)
    }
}

// ── Memberships ───────────────────────────────────────────────────────────

/// SQLite-backed `org_members` + `workspace_members` store.
#[derive(Clone, Debug)]
pub struct SqliteMembershipStore {
    pool: SqlitePool,
}

impl SqliteMembershipStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn membership_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<MembershipRow, StorageError> {
    // NOT NULL columns — `required` rejects SQL NULL (see its doc).
    let scope_kind_txt: String = required(r, "scope_kind")?;
    let principal_kind_txt: String = required(r, "principal_kind")?;
    Ok(MembershipRow {
        // Fail-closed: an unrecognized authz-domain value is a hard
        // deserialization error, never silently coerced to a default.
        scope_kind: ScopeKind::parse(&scope_kind_txt).map_err(|bad| {
            StorageError::Serialization(format!("unknown membership scope_kind {bad:?}"))
        })?,
        scope_id: required(r, "scope_id")?,
        principal_kind: PrincipalKind::parse(&principal_kind_txt).map_err(|bad| {
            StorageError::Serialization(format!("unknown membership principal_kind {bad:?}"))
        })?,
        principal_id: required(r, "principal_id")?,
        role: required(r, "role")?,
        added_at: required(r, "added_at")?,
        // Nullable column.
        added_by: optional(r, "added_by")?,
    })
}

#[async_trait::async_trait]
impl MembershipStore for SqliteMembershipStore {
    // Historical grants lack an organization column. Reject workspace ids
    // with any other parent, including deleted aliases, rather than reuse grants.
    #[tracing::instrument(skip_all)]
    async fn get_tenant_membership(
        &self,
        org_id: &str,
        workspace_id: Option<&str>,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<TenantMembershipSnapshot, StorageError> {
        let rows = sqlx::query(
            "SELECT m.* \
             FROM port_memberships m \
             WHERE m.principal_kind = ?1 \
             AND m.principal_id = ?2 \
             AND ((m.scope_kind = 'org' \
             AND m.scope_id = ?3) \
             OR (m.scope_kind = 'workspace' \
             AND m.scope_id = ?4 \
             AND EXISTS (SELECT 1 FROM port_orgs o WHERE o.id = ?3 AND o.deleted_at IS NULL) \
             AND EXISTS (SELECT 1 \
             FROM port_workspaces w \
             WHERE w.org_id = ?3 \
             AND w.id = ?4 \
             AND w.deleted_at IS NULL AND NOT EXISTS (SELECT 1 FROM port_workspaces other WHERE other.id = w.id AND other.org_id <> w.org_id))))",
        )
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .bind(org_id)
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        let mut snapshot = TenantMembershipSnapshot::default();
        for raw in &rows {
            let row = membership_from_row(raw)?;
            match row.scope_kind {
                ScopeKind::Org => snapshot.org_role = Some(parse_org_role(&row.role)?),
                ScopeKind::Workspace => {
                    snapshot.workspace_role =
                        Some(WorkspaceMembershipRole::parse(&row.role).map_err(|_| {
                            StorageError::Serialization("membership role is invalid".into())
                        })?);
                },
            }
        }
        Ok(snapshot)
    }

    #[tracing::instrument(skip_all)]
    async fn list_orgs_for_principal(
        &self,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Vec<PrincipalOrgMembership>, StorageError> {
        let rows = sqlx::query(
            "SELECT * \
             FROM port_memberships \
             WHERE scope_kind = 'org' \
             AND principal_kind = ?1 \
             AND principal_id = ?2 \
             ORDER BY scope_id",
        )
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter()
            .map(|raw| {
                let row = membership_from_row(raw)?;
                Ok(PrincipalOrgMembership {
                    org_id: row.scope_id,
                    role: parse_org_role(&row.role)?,
                })
            })
            .collect()
    }

    #[tracing::instrument(skip_all)]
    async fn list_workspace_members(
        &self,
        org_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, StorageError> {
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        let workspace = sqlx::query(
            "SELECT id FROM port_workspaces WHERE org_id = ?1 AND id = ?2 AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM port_orgs o WHERE o.id = ?1 AND o.deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM port_workspaces other WHERE other.id = ?2 AND other.org_id <> ?1)",
        )
        .bind(org_id)
        .bind(workspace_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;
        if workspace.is_none() {
            return Err(StorageError::not_found("workspace", workspace_id));
        }
        let rows = sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = 'workspace' AND scope_id = ?1 ORDER BY principal_kind, principal_id",
        )
        .bind(workspace_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        rows.iter().map(workspace_membership_from_row).collect()
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_org_member_guarded(
        &self,
        request: OrgMemberUpsert,
    ) -> Result<OrgMemberUpsertOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conn_err)?;
        let org = sqlx::query("SELECT id FROM port_orgs WHERE id = ?1 AND deleted_at IS NULL")
            .bind(&request.org_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(conn_err)?;
        if org.is_none() {
            return Err(StorageError::not_found("org", request.org_id));
        }
        // BEGIN IMMEDIATE serializes the invariant read with every writer.
        let rows = sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = 'org' AND scope_id = ?1",
        )
        .bind(&request.org_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(conn_err)?;
        let mut privileged_other = false;
        for raw in &rows {
            let row = membership_from_row(raw)?;
            let role = parse_org_role(&row.role)?;
            privileged_other |= role.is_privileged()
                && (row.principal_kind != request.principal_kind
                    || row.principal_id != request.principal_id);
        }
        if !request.role.is_privileged() && !privileged_other {
            return Ok(OrgMemberUpsertOutcome::WouldLockOut);
        }
        sqlx::query(
            "INSERT INTO port_memberships (scope_kind, scope_id, principal_kind, principal_id, role, added_at, added_by) \
             VALUES ('org', ?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (scope_kind, scope_id, principal_kind, principal_id) \
             DO UPDATE \
             SET role = excluded.role, added_at = excluded.added_at, added_by = excluded.added_by",
        )
            .bind(&request.org_id)
            .bind(request.principal_kind.as_str())
            .bind(&request.principal_id)
            .bind(request.role.as_str())
            .bind(now_rfc3339())
            .bind(&request.added_by)
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        Ok(OrgMemberUpsertOutcome::Applied)
    }

    #[tracing::instrument(skip_all)]
    async fn remove_org_member_guarded(
        &self,
        org_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<OrgMemberRemoveOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conn_err)?;
        let org = sqlx::query("SELECT id FROM port_orgs WHERE id = ?1 AND deleted_at IS NULL")
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(conn_err)?;
        if org.is_none() {
            return Err(StorageError::not_found("org", org_id));
        }
        let rows = sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = 'org' AND scope_id = ?1",
        )
        .bind(org_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(conn_err)?;
        let mut found = false;
        let mut privileged_other = false;
        for raw in &rows {
            let row = membership_from_row(raw)?;
            let role = parse_org_role(&row.role)?;
            let target = row.principal_kind == principal_kind && row.principal_id == principal_id;
            found |= target;
            privileged_other |= role.is_privileged() && !target;
        }
        if !found {
            return Ok(OrgMemberRemoveOutcome::NotFound);
        }
        if !privileged_other {
            return Ok(OrgMemberRemoveOutcome::WouldLockOut);
        }
        let ambiguous_workspace = sqlx::query(
            "SELECT 1 FROM port_workspaces own JOIN port_workspaces other ON own.id = other.id AND own.org_id <> other.org_id WHERE own.org_id = ?1 LIMIT 1",
        )
        .bind(org_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;
        if ambiguous_workspace.is_some() {
            return Err(StorageError::Serialization(
                "workspace identity is ambiguous".into(),
            ));
        }
        sqlx::query("DELETE FROM port_memberships WHERE scope_kind = 'org' AND scope_id = ?1 AND principal_kind = ?2 AND principal_id = ?3")
            .bind(org_id).bind(principal_kind.as_str()).bind(principal_id).execute(&mut *tx).await.map_err(conn_err)?;
        sqlx::query(
            "DELETE FROM port_memberships WHERE scope_kind = 'workspace' AND principal_kind = ?1 AND principal_id = ?2 AND scope_id IN (SELECT id FROM port_workspaces WHERE org_id = ?3)",
        )
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .bind(org_id)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        Ok(OrgMemberRemoveOutcome::Removed)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_workspace_member(
        &self,
        request: WorkspaceMemberUpsert,
    ) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conn_err)?;
        let workspace = sqlx::query(
            "SELECT id FROM port_workspaces WHERE org_id = ?1 AND id = ?2 AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM port_orgs o WHERE o.id = ?1 AND o.deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM port_workspaces other WHERE other.id = ?2 AND other.org_id <> ?1)",
        )
        .bind(&request.org_id)
        .bind(&request.workspace_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;
        if workspace.is_none() {
            return Err(StorageError::not_found("workspace", request.workspace_id));
        }
        let org_membership = sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = 'org' AND scope_id = ?1 AND principal_kind = ?2 AND principal_id = ?3",
        )
        .bind(&request.org_id)
        .bind(request.principal_kind.as_str())
        .bind(&request.principal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;
        let Some(org_membership) = org_membership else {
            return Err(StorageError::not_found(
                "org membership",
                request.principal_id,
            ));
        };
        parse_org_role(&membership_from_row(&org_membership)?.role)?;
        sqlx::query(
            "INSERT INTO port_memberships (scope_kind, scope_id, principal_kind, principal_id, role, added_at, added_by) \
             VALUES ('workspace', ?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (scope_kind, scope_id, principal_kind, principal_id) \
             DO UPDATE \
             SET role = excluded.role, added_at = excluded.added_at, added_by = excluded.added_by",
        )
            .bind(&request.workspace_id)
            .bind(request.principal_kind.as_str())
            .bind(&request.principal_id)
            .bind(request.role.as_str())
            .bind(now_rfc3339())
            .bind(&request.added_by)
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        Ok(())
    }

    async fn get(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Option<MembershipRow>, StorageError> {
        let row = sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = ? AND scope_id = ? \
             AND principal_kind = ? AND principal_id = ?",
        )
        .bind(scope_kind.as_str())
        .bind(scope_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(membership_from_row).transpose()
    }

    async fn list_for_scope(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
    ) -> Result<Vec<MembershipRow>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM port_memberships \
             WHERE scope_kind = ? AND scope_id = ? ORDER BY principal_id",
        )
        .bind(scope_kind.as_str())
        .bind(scope_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter().map(membership_from_row).collect()
    }

    #[tracing::instrument(skip_all)]
    async fn remove_workspace_member(
        &self,
        org_id: &str,
        workspace_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<bool, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conn_err)?;
        let workspace = sqlx::query(
            "SELECT id FROM port_workspaces WHERE org_id = ?1 AND id = ?2 AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM port_orgs o WHERE o.id = ?1 AND o.deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM port_workspaces other WHERE other.id = ?2 AND other.org_id <> ?1)",
        )
        .bind(org_id)
        .bind(workspace_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(conn_err)?;
        if workspace.is_none() {
            return Ok(false);
        }
        let result = sqlx::query(
            "DELETE \
             FROM port_memberships \
             WHERE scope_kind = 'workspace' \
             AND scope_id = ?1 \
             AND principal_kind = ?2 \
             AND principal_id = ?3",
        )
        .bind(workspace_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;
        tx.commit().await.map_err(conn_err)?;
        Ok(result.rows_affected() != 0)
    }
}

fn parse_org_role(value: &str) -> Result<OrgMembershipRole, StorageError> {
    OrgMembershipRole::parse(value)
        .map_err(|_| StorageError::Serialization("membership role is invalid".into()))
}

fn workspace_membership_from_row(
    raw: &sqlx::sqlite::SqliteRow,
) -> Result<WorkspaceMembership, StorageError> {
    let row = membership_from_row(raw)?;
    Ok(WorkspaceMembership {
        principal_kind: row.principal_kind,
        principal_id: row.principal_id,
        role: WorkspaceMembershipRole::parse(&row.role)
            .map_err(|_| StorageError::Serialization("membership role is invalid".into()))?,
    })
}

// ── Resources (workspace-scoped) ──────────────────────────────────────────

/// SQLite-backed `resources` store. Slug is unique among active rows per
/// workspace scope.
#[derive(Clone, Debug)]
pub struct SqliteResourceStore {
    pool: SqlitePool,
}

impl SqliteResourceStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn resource_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<ResourceRow, StorageError> {
    Ok(ResourceRow {
        // NOT NULL columns — use `required` to reject SQL NULL (see its doc).
        id: required(r, "id")?,
        workspace_id: required(r, "workspace_id")?,
        slug: required(r, "slug")?,
        display_name: required(r, "display_name")?,
        kind: required(r, "kind")?,
        config: text_to_json(&required::<String>(r, "config")?)?,
        credential_bindings: serde_json::from_str(&required::<String>(r, "credential_bindings")?)
            .map_err(|error| StorageError::Serialization(error.to_string()))?,
        topology: optional_json(r, "topology")?,
        resilience_override: optional_json(r, "resilience_override")?,
        created_at: required(r, "created_at")?,
        created_by: required(r, "created_by")?,
        version: required::<i64>(r, "version")? as u64,
        // Nullable column.
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

/// Reads a nullable JSON text column; SQL NULL is `None`.
fn optional_json(
    r: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<serde_json::Value>, StorageError> {
    r.try_get::<Option<String>, _>(column)
        .map_err(conn_err)?
        .as_deref()
        .map(text_to_json)
        .transpose()
}

#[async_trait::async_trait]
impl ResourceStore for SqliteResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_resources (id, workspace_id, org_id, slug, \
             display_name, kind, config, credential_bindings, topology, \
             resilience_override, created_at, created_by, version, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_to_text(&row.config))
        .bind(
            serde_json::to_string(&row.credential_bindings)
                .map_err(|error| StorageError::Serialization(error.to_string()))?,
        )
        .bind(row.topology.as_ref().map(json_to_text))
        .bind(row.resilience_override.as_ref().map(json_to_text))
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(row.version as i64)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "resource",
                    detail: format!("resource {} or its active slug already exists", row.id),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ResourceRow>, StorageError> {
        let row = sqlx::query(
            "SELECT * FROM port_resources \
             WHERE workspace_id = ? AND org_id = ? AND id = ? \
             AND deleted_at IS NULL",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(resource_from_row).transpose()
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM port_resources \
             WHERE workspace_id = ? AND org_id = ? AND deleted_at IS NULL \
             ORDER BY id",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter().map(resource_from_row).collect()
    }

    async fn update(
        &self,
        scope: &Scope,
        row: ResourceRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_resources SET slug = ?, display_name = ?, kind = ?, \
             config = ?, credential_bindings = ?, topology = ?, resilience_override = ?, \
             version = ? \
             WHERE workspace_id = ? AND org_id = ? AND id = ? \
             AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_to_text(&row.config))
        .bind(
            serde_json::to_string(&row.credential_bindings)
                .map_err(|error| StorageError::Serialization(error.to_string()))?,
        )
        .bind(row.topology.as_ref().map(json_to_text))
        .bind(row.resilience_override.as_ref().map(json_to_text))
        .bind(row.version as i64)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.id)
        .bind(expected_version as i64)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM port_resources \
             WHERE workspace_id = ? AND org_id = ? AND id = ?",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        match current {
            Some(actual) => Err(StorageError::Conflict {
                entity: "resource",
                id: row.id,
                expected: expected_version,
                actual: actual as u64,
            }),
            None => Err(StorageError::not_found("resource", row.id)),
        }
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        soft_delete_scoped(&self.pool, "port_resources", "resource", scope, id).await
    }
}

// ── Triggers (workspace-scoped) ───────────────────────────────────────────

/// SQLite-backed `triggers` store.
#[derive(Clone, Debug)]
pub struct SqliteTriggerStore {
    pool: SqlitePool,
}

impl SqliteTriggerStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn trigger_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<TriggerRow, StorageError> {
    Ok(TriggerRow {
        // NOT NULL columns — use `required` to reject SQL NULL (see its doc).
        id: required(r, "id")?,
        workspace_id: required(r, "workspace_id")?,
        workflow_id: required(r, "workflow_id")?,
        slug: required(r, "slug")?,
        display_name: required(r, "display_name")?,
        kind: required(r, "kind")?,
        config: text_to_json(&required::<String>(r, "config")?)?,
        state: required(r, "state")?,
        created_at: required(r, "created_at")?,
        created_by: required(r, "created_by")?,
        version: required::<i64>(r, "version")? as u64,
        // Nullable columns.
        run_as: r.try_get("run_as").ok(),
        webhook_path: r.try_get("webhook_path").ok(),
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl TriggerStore for SqliteTriggerStore {
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_triggers (id, workspace_id, org_id, workflow_id, \
             slug, display_name, kind, config, state, run_as, webhook_path, \
             created_at, created_by, version, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_to_text(&row.config))
        .bind(&row.state)
        .bind(&row.run_as)
        .bind(&row.webhook_path)
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(row.version as i64)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "trigger",
                    detail: format!("trigger {} already exists", row.id),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<TriggerRow>, StorageError> {
        let row = sqlx::query(
            "SELECT * FROM port_triggers \
             WHERE workspace_id = ? AND org_id = ? AND id = ? \
             AND deleted_at IS NULL",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(trigger_from_row).transpose()
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<TriggerRow>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM port_triggers \
             WHERE workspace_id = ? AND org_id = ? AND deleted_at IS NULL \
             ORDER BY id",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter().map(trigger_from_row).collect()
    }

    async fn update(
        &self,
        scope: &Scope,
        row: TriggerRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_triggers SET workflow_id = ?, slug = ?, \
             display_name = ?, kind = ?, config = ?, state = ?, run_as = ?, \
             webhook_path = ?, version = ? WHERE workspace_id = ? AND org_id = ? \
             AND id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_to_text(&row.config))
        .bind(&row.state)
        .bind(&row.run_as)
        .bind(&row.webhook_path)
        .bind(row.version as i64)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.id)
        .bind(expected_version as i64)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM port_triggers \
             WHERE workspace_id = ? AND org_id = ? AND id = ?",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        match current {
            Some(actual) => Err(StorageError::Conflict {
                entity: "trigger",
                id: row.id,
                expected: expected_version,
                actual: actual as u64,
            }),
            None => Err(StorageError::not_found("trigger", row.id)),
        }
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        soft_delete_scoped(&self.pool, "port_triggers", "trigger", scope, id).await
    }
}

// ── Quotas (org-scoped, CAS counters) ─────────────────────────────────────

/// SQLite-backed `org_quotas` + `org_quota_usage` store.
#[derive(Clone, Debug)]
pub struct SqliteQuotaStore {
    pool: SqlitePool,
}

impl SqliteQuotaStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn quota_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<QuotaRow, StorageError> {
    Ok(QuotaRow {
        // NOT NULL columns — use `required` to reject SQL NULL (see its doc).
        org_id: required(r, "org_id")?,
        plan: required(r, "plan")?,
        concurrent_executions_limit: i32::try_from(required::<i64>(
            r,
            "concurrent_executions_limit",
        )?)
        .map_err(|e| {
            StorageError::Serialization(format!(
                "concurrent_executions_limit out of i32 range: {e}"
            ))
        })?,
        concurrent_executions: i32::try_from(required::<i64>(r, "concurrent_executions")?)
            .map_err(|e| {
                StorageError::Serialization(format!("concurrent_executions out of i32 range: {e}"))
            })?,
        executions_this_month: required::<i64>(r, "executions_this_month")?,
        month_reset_at: required(r, "month_reset_at")?,
        updated_at: required(r, "updated_at")?,
        // Nullable columns — Option<T> decode is correct; a NULL column becomes None.
        executions_per_month_limit: r
            .try_get::<Option<i64>, _>("executions_per_month_limit")
            .map_err(conn_err)?,
        active_workflows_limit: r
            .try_get::<Option<i64>, _>("active_workflows_limit")
            .map_err(conn_err)?
            .map(|v| {
                i32::try_from(v).map_err(|e| {
                    StorageError::Serialization(format!(
                        "active_workflows_limit out of i32 range: {e}"
                    ))
                })
            })
            .transpose()?,
    })
}

#[async_trait::async_trait]
impl QuotaStore for SqliteQuotaStore {
    async fn get(&self, org_id: &str) -> Result<Option<QuotaRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_quotas WHERE org_id = ?")
            .bind(org_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(quota_from_row).transpose()
    }

    async fn upsert(&self, row: QuotaRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_quotas (org_id, plan, concurrent_executions_limit, \
             executions_per_month_limit, active_workflows_limit, \
             concurrent_executions, executions_this_month, month_reset_at, \
             updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (org_id) DO UPDATE SET plan = excluded.plan, \
             concurrent_executions_limit = excluded.concurrent_executions_limit, \
             executions_per_month_limit = excluded.executions_per_month_limit, \
             active_workflows_limit = excluded.active_workflows_limit, \
             concurrent_executions = excluded.concurrent_executions, \
             executions_this_month = excluded.executions_this_month, \
             month_reset_at = excluded.month_reset_at, \
             updated_at = excluded.updated_at",
        )
        .bind(&row.org_id)
        .bind(&row.plan)
        .bind(i64::from(row.concurrent_executions_limit))
        .bind(row.executions_per_month_limit)
        .bind(row.active_workflows_limit.map(i64::from))
        .bind(i64::from(row.concurrent_executions))
        .bind(row.executions_this_month)
        .bind(&row.month_reset_at)
        .bind(&row.updated_at)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn adjust_concurrent(&self, org_id: &str, delta: i32) -> Result<i32, StorageError> {
        // Conditional decrement guards the floor: `concurrent_executions +
        // delta >= 0` is enforced in the WHERE so a would-be-negative
        // adjustment affects zero rows and is rejected.
        let res = sqlx::query(
            "UPDATE port_quotas \
             SET concurrent_executions = concurrent_executions + ? \
             WHERE org_id = ? AND concurrent_executions + ? >= 0",
        )
        .bind(i64::from(delta))
        .bind(org_id)
        .bind(i64::from(delta))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            let v = sqlx::query_scalar::<_, i64>(
                "SELECT concurrent_executions FROM port_quotas WHERE org_id = ?",
            )
            .bind(org_id)
            .fetch_one(&self.pool)
            .await
            .map_err(conn_err)?;
            return i32::try_from(v).map_err(|e| {
                StorageError::Serialization(format!("concurrent_executions out of i32 range: {e}"))
            });
        }
        // Disambiguate: no such org ⇒ NotFound; otherwise the guard
        // rejected a below-zero adjustment ⇒ Conflict.
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT concurrent_executions FROM port_quotas WHERE org_id = ?",
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        match current {
            Some(actual) => Err(StorageError::Conflict {
                entity: "quota",
                id: org_id.to_string(),
                expected: 0,
                actual: actual as u64,
            }),
            None => Err(StorageError::not_found("quota", org_id)),
        }
    }
}

// ── Audit log (append-only) ───────────────────────────────────────────────

/// SQLite-backed `audit_log` store. Append-only; reads are newest-first.
#[derive(Clone, Debug)]
pub struct SqliteAuditStore {
    pool: SqlitePool,
}

impl SqliteAuditStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn audit_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<AuditLogRow, StorageError> {
    Ok(AuditLogRow {
        // NOT NULL columns — use `required` to reject SQL NULL (see its doc).
        id: required(r, "id")?,
        org_id: required(r, "org_id")?,
        actor_kind: required(r, "actor_kind")?,
        action: required(r, "action")?,
        emitted_at: required(r, "emitted_at")?,
        // Nullable columns.
        workspace_id: r.try_get("workspace_id").ok(),
        actor_id: r.try_get("actor_id").ok(),
        target_kind: r.try_get("target_kind").ok(),
        target_id: r.try_get("target_id").ok(),
        details: opt_text_to_json(
            r.try_get::<Option<String>, _>("details")
                .map_err(conn_err)?,
        )?,
        ip_address: r.try_get("ip_address").ok(),
        user_agent: r.try_get("user_agent").ok(),
    })
}

#[async_trait::async_trait]
impl AuditStore for SqliteAuditStore {
    async fn append(&self, row: AuditLogRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_audit_log (id, org_id, workspace_id, actor_kind, \
             actor_id, action, target_kind, target_id, details, ip_address, \
             user_agent, emitted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.org_id)
        .bind(&row.workspace_id)
        .bind(&row.actor_kind)
        .bind(&row.actor_id)
        .bind(&row.action)
        .bind(&row.target_kind)
        .bind(&row.target_id)
        .bind(row.details.as_ref().map(json_to_text))
        .bind(&row.ip_address)
        .bind(&row.user_agent)
        .bind(&row.emitted_at)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn list_for_org(
        &self,
        org_id: &str,
        limit: u32,
    ) -> Result<Vec<AuditLogRow>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM port_audit_log WHERE org_id = ? \
             ORDER BY emitted_at DESC, id DESC LIMIT ?",
        )
        .bind(org_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter().map(audit_from_row).collect()
    }
}

// ── Blobs (workspace-scoped) ──────────────────────────────────────────────

/// SQLite-backed `blobs` store.
#[derive(Clone, Debug)]
pub struct SqliteBlobStore {
    pool: SqlitePool,
}

impl SqliteBlobStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn blob_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<BlobRow, StorageError> {
    Ok(BlobRow {
        // NOT NULL columns — use `required` to reject SQL NULL (see its doc).
        id: required(r, "id")?,
        workspace_id: required(r, "workspace_id")?,
        kind: required(r, "kind")?,
        size_bytes: required::<i64>(r, "size_bytes")?,
        storage_mode: required(r, "storage_mode")?,
        created_at: required(r, "created_at")?,
        // Nullable columns.
        execution_id: r.try_get("execution_id").ok(),
        content_type: r.try_get("content_type").ok(),
        checksum: r.try_get("checksum").ok(),
        data: r.try_get("data").ok(),
        external_ref: r.try_get("external_ref").ok(),
        metadata: opt_text_to_json(
            r.try_get::<Option<String>, _>("metadata")
                .map_err(conn_err)?,
        )?,
        expires_at: r.try_get("expires_at").ok(),
    })
}

#[async_trait::async_trait]
impl BlobStore for SqliteBlobStore {
    async fn put(&self, row: BlobRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_blobs (id, workspace_id, execution_id, kind, \
             content_type, size_bytes, checksum, storage_mode, data, \
             external_ref, metadata, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (workspace_id, id) DO UPDATE SET \
             execution_id = excluded.execution_id, kind = excluded.kind, \
             content_type = excluded.content_type, \
             size_bytes = excluded.size_bytes, checksum = excluded.checksum, \
             storage_mode = excluded.storage_mode, data = excluded.data, \
             external_ref = excluded.external_ref, metadata = excluded.metadata, \
             created_at = excluded.created_at, expires_at = excluded.expires_at",
        )
        .bind(&row.id)
        .bind(&row.workspace_id)
        .bind(&row.execution_id)
        .bind(&row.kind)
        .bind(&row.content_type)
        .bind(row.size_bytes)
        .bind(&row.checksum)
        .bind(&row.storage_mode)
        .bind(&row.data)
        .bind(&row.external_ref)
        .bind(row.metadata.as_ref().map(json_to_text))
        .bind(&row.created_at)
        .bind(&row.expires_at)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn get(&self, workspace_id: &str, id: &str) -> Result<Option<BlobRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_blobs WHERE workspace_id = ? AND id = ?")
            .bind(workspace_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(blob_from_row).transpose()
    }

    async fn delete(&self, workspace_id: &str, id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM port_blobs WHERE workspace_id = ? AND id = ?")
            .bind(workspace_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(conn_err)?;
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        let res = sqlx::query(
            "DELETE FROM port_blobs \
             WHERE expires_at IS NOT NULL AND expires_at <= ?",
        )
        .bind(now_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(res.rows_affected())
    }
}

// ── shared ────────────────────────────────────────────────────────────────

/// Current time as an RFC 3339 string — the soft-delete / eviction stamp
/// format the port DTOs use (consistent with the in-memory backend).
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Disambiguate a zero-row CAS `UPDATE` on a single-PK `id` table whose
/// rows soft-delete via `deleted_at`: the row is gone (or soft-deleted) ⇒
/// `NotFound`; the version moved ⇒ `Conflict { actual }`.
async fn cas_disambiguate(
    pool: &SqlitePool,
    table: &str,
    entity: &'static str,
    id: &str,
    expected_version: u64,
) -> Result<(), StorageError> {
    // `table` is a fixed internal literal (never user input), so the
    // format here cannot be an injection vector.
    let sql = format!("SELECT version FROM {table} WHERE id = ?");
    let current = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(conn_err)?;
    match current {
        Some(actual) => Err(StorageError::Conflict {
            entity,
            id: id.to_string(),
            expected: expected_version,
            actual: actual as u64,
        }),
        None => Err(StorageError::not_found(entity, id)),
    }
}

/// Soft-delete a single-PK `id` row (active rows only); zero rows ⇒
/// `NotFound`.
async fn soft_delete_by_id(
    pool: &SqlitePool,
    table: &str,
    entity: &'static str,
    id: &str,
) -> Result<(), StorageError> {
    let sql = format!("UPDATE {table} SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL");
    let res = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(now_rfc3339())
        .bind(id)
        .execute(pool)
        .await
        .map_err(conn_err)?;
    if res.rows_affected() > 0 {
        Ok(())
    } else {
        Err(StorageError::not_found(entity, id))
    }
}

/// Soft-delete a workspace-scoped `id` row (active rows only); zero rows ⇒
/// `NotFound`.
async fn soft_delete_scoped(
    pool: &SqlitePool,
    table: &str,
    entity: &'static str,
    scope: &Scope,
    id: &str,
) -> Result<(), StorageError> {
    let sql = format!(
        "UPDATE {table} SET deleted_at = ? \
         WHERE workspace_id = ? AND org_id = ? AND id = ? AND deleted_at IS NULL"
    );
    let res = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(now_rfc3339())
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .execute(pool)
        .await
        .map_err(conn_err)?;
    if res.rows_affected() > 0 {
        Ok(())
    } else {
        Err(StorageError::not_found(entity, id))
    }
}

#[cfg(test)]
#[path = "identity_decoder_null_guard_tests.rs"]
mod decoder_null_guard_tests;
