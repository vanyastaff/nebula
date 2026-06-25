//! SQLite identity-zoo stores over the port-scoped schema.
//!
//! Each aggregate is a `port_*` table in `schema.sql`. Every tenant- or
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

use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TriggerRow, UserRow, WorkspaceRow,
};
use nebula_storage_port::store::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore, TriggerStore,
    UserStore, WorkspaceStore,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqlitePool};

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

fn user_from_row(r: &sqlx::sqlite::SqliteRow) -> Result<UserRow, StorageError> {
    Ok(UserRow {
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
        mfa_secret: r.try_get("mfa_secret").ok(),
        deleted_at: r.try_get("deleted_at").ok(),
    })
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
        .bind(&row.mfa_secret)
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

    async fn get(&self, id: &str) -> Result<Option<UserRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_users WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(user_from_row).transpose()
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<UserRow>, StorageError> {
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
        .bind(&row.mfa_secret)
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
        billing_email: r.try_get("billing_email").ok(),
        deleted_at: r.try_get("deleted_at").ok(),
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
        description: r.try_get("description").ok(),
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl WorkspaceStore for SqliteWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
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
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => Ok(()),
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
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM port_workspaces WHERE org_id = ? AND id = ?",
        )
        .bind(&row.org_id)
        .bind(&row.id)
        .fetch_optional(&self.pool)
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
        let res = sqlx::query(
            "UPDATE port_workspaces SET deleted_at = ? \
             WHERE org_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(now_rfc3339())
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            Ok(())
        } else {
            Err(StorageError::not_found("workspace", id))
        }
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
        added_by: r.try_get("added_by").ok(),
    })
}

#[async_trait::async_trait]
impl MembershipStore for SqliteMembershipStore {
    async fn upsert(&self, row: MembershipRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_memberships (scope_kind, scope_id, principal_kind, \
             principal_id, role, added_at, added_by) VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (scope_kind, scope_id, principal_kind, principal_id) \
             DO UPDATE SET role = excluded.role, added_at = excluded.added_at, \
             added_by = excluded.added_by",
        )
        .bind(row.scope_kind.as_str())
        .bind(&row.scope_id)
        .bind(row.principal_kind.as_str())
        .bind(&row.principal_id)
        .bind(&row.role)
        .bind(&row.added_at)
        .bind(&row.added_by)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
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

    async fn remove(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "DELETE FROM port_memberships WHERE scope_kind = ? AND scope_id = ? \
             AND principal_kind = ? AND principal_id = ?",
        )
        .bind(scope_kind.as_str())
        .bind(scope_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }
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
        created_at: required(r, "created_at")?,
        created_by: required(r, "created_by")?,
        version: required::<i64>(r, "version")? as u64,
        // Nullable column.
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl ResourceStore for SqliteResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_resources (id, workspace_id, org_id, slug, \
             display_name, kind, config, created_at, created_by, version, \
             deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_to_text(&row.config))
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
             config = ?, version = ? WHERE workspace_id = ? AND org_id = ? \
             AND id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_to_text(&row.config))
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
        let res = sqlx::query(
            "UPDATE port_resources SET deleted_at = ? \
             WHERE workspace_id = ? AND org_id = ? AND id = ? \
             AND deleted_at IS NULL",
        )
        .bind(now_rfc3339())
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            Ok(())
        } else {
            Err(StorageError::not_found("resource", id))
        }
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
        let res = sqlx::query(
            "UPDATE port_triggers SET deleted_at = ? \
             WHERE workspace_id = ? AND org_id = ? AND id = ? \
             AND deleted_at IS NULL",
        )
        .bind(now_rfc3339())
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            Ok(())
        } else {
            Err(StorageError::not_found("trigger", id))
        }
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

#[cfg(test)]
mod decoder_null_guard_tests {
    // FIX 1 regression: a NULL in a NOT-NULL-modeled column must surface as
    // `Err(StorageError::…)`, not silently default to an empty string / zero.
    // Each test crafts an in-memory SQLite row with a deliberate NULL in one
    // required column and asserts the decoder returns Err.

    use nebula_storage_port::StorageError;
    use sqlx::SqlitePool;

    use super::{
        audit_from_row, blob_from_row, org_from_row, quota_from_row, resource_from_row,
        trigger_from_row, user_from_row, workspace_from_row,
    };

    // Open a temporary in-memory SQLite pool with the given DDL applied, then
    // fetch the sole row and call `decoder` on it. Returns the decoder's Result.
    //
    // Both `ddl` and `insert` are `'static` string literals sourced from test
    // constants — the `sqlx::query` API requires `'static`.
    async fn with_null_row<T>(
        ddl: &'static str,
        insert: &'static str,
        decoder: impl Fn(&sqlx::sqlite::SqliteRow) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(ddl).execute(&pool).await.unwrap();
        sqlx::query(insert).execute(&pool).await.unwrap();
        let row = sqlx::query("SELECT * FROM t")
            .fetch_one(&pool)
            .await
            .unwrap();
        decoder(&row)
    }

    #[tokio::test]
    async fn user_from_row_null_required_id_is_err() {
        // The `id` column is NOT NULL in the schema; a NULL decode must be Err.
        let result = with_null_row(
            "CREATE TABLE t (id TEXT, email TEXT NOT NULL, display_name TEXT NOT NULL, \
             created_at TEXT NOT NULL, failed_login_count INTEGER NOT NULL DEFAULT 0, \
             mfa_enabled INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 0, \
             email_verified_at TEXT, avatar_url TEXT, password_hash TEXT, \
             last_login_at TEXT, locked_until TEXT, mfa_secret BLOB, deleted_at TEXT)",
            // Insert a row where `id` is explicitly NULL.
            "INSERT INTO t VALUES (NULL, 'a@b.com', 'Alice', '2024-01-01T00:00:00Z', \
             0, 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            user_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "user_from_row must return Err when the required `id` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn org_from_row_null_required_created_at_is_err() {
        let result = with_null_row(
            "CREATE TABLE t (id TEXT NOT NULL, slug TEXT NOT NULL, display_name TEXT NOT NULL, \
             created_at TEXT, created_by TEXT NOT NULL, plan TEXT NOT NULL, \
             settings TEXT NOT NULL DEFAULT '{}', version INTEGER NOT NULL DEFAULT 0, \
             billing_email TEXT, deleted_at TEXT)",
            // `created_at` is NOT NULL in the schema but NULL here.
            "INSERT INTO t VALUES ('org-1', 'my-org', 'My Org', NULL, 'user-1', 'free', \
             '{}', 0, NULL, NULL)",
            org_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "org_from_row must return Err when the required `created_at` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn workspace_from_row_null_required_slug_is_err() {
        let result = with_null_row(
            "CREATE TABLE t (id TEXT NOT NULL, org_id TEXT NOT NULL, slug TEXT, \
             display_name TEXT NOT NULL, created_at TEXT NOT NULL, \
             created_by TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0, \
             settings TEXT NOT NULL DEFAULT '{}', version INTEGER NOT NULL DEFAULT 0, \
             description TEXT, deleted_at TEXT)",
            // `slug` is NOT NULL in the schema but NULL here.
            "INSERT INTO t VALUES ('ws-1', 'org-1', NULL, 'My WS', '2024-01-01T00:00:00Z', \
             'user-1', 0, '{}', 0, NULL, NULL)",
            workspace_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "workspace_from_row must return Err when the required `slug` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn quota_from_row_null_required_plan_is_err() {
        let result = with_null_row(
            "CREATE TABLE t (org_id TEXT NOT NULL, plan TEXT, \
             concurrent_executions_limit INTEGER NOT NULL DEFAULT 0, \
             concurrent_executions INTEGER NOT NULL DEFAULT 0, \
             executions_this_month INTEGER NOT NULL DEFAULT 0, \
             month_reset_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
             executions_per_month_limit INTEGER, active_workflows_limit INTEGER)",
            // `plan` is NOT NULL in the schema but NULL here.
            "INSERT INTO t VALUES ('org-1', NULL, 10, 0, 0, '2024-01-01T00:00:00Z', \
             '2024-01-01T00:00:00Z', NULL, NULL)",
            quota_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "quota_from_row must return Err when the required `plan` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn audit_from_row_null_required_action_is_err() {
        let result = with_null_row(
            "CREATE TABLE t (id TEXT NOT NULL, org_id TEXT NOT NULL, \
             actor_kind TEXT NOT NULL, action TEXT, emitted_at TEXT NOT NULL, \
             workspace_id TEXT, actor_id TEXT, target_kind TEXT, target_id TEXT, \
             details TEXT, ip_address TEXT, user_agent TEXT)",
            // `action` is NOT NULL in the schema but NULL here.
            "INSERT INTO t VALUES ('evt-1', 'org-1', 'user', NULL, '2024-01-01T00:00:00Z', \
             NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            audit_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "audit_from_row must return Err when the required `action` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn resource_from_row_null_required_kind_is_err() {
        let result = with_null_row(
            "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, slug TEXT NOT NULL, \
             display_name TEXT NOT NULL, kind TEXT, config TEXT NOT NULL DEFAULT '{}', \
             created_at TEXT NOT NULL, created_by TEXT NOT NULL, \
             version INTEGER NOT NULL DEFAULT 0, deleted_at TEXT)",
            // `kind` is NOT NULL in the schema but NULL here.
            "INSERT INTO t VALUES ('res-1', 'ws-1', 'my-res', 'My Resource', NULL, '{}', \
             '2024-01-01T00:00:00Z', 'user-1', 0, NULL)",
            resource_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "resource_from_row must return Err when the required `kind` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn trigger_from_row_null_required_workflow_id_is_err() {
        let result = with_null_row(
            "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, \
             workflow_id TEXT, slug TEXT NOT NULL, display_name TEXT NOT NULL, \
             kind TEXT NOT NULL, config TEXT NOT NULL DEFAULT '{}', \
             state TEXT NOT NULL, created_at TEXT NOT NULL, created_by TEXT NOT NULL, \
             version INTEGER NOT NULL DEFAULT 0, run_as TEXT, webhook_path TEXT, deleted_at TEXT)",
            // `workflow_id` is NOT NULL in the schema but NULL here.
            "INSERT INTO t VALUES ('trg-1', 'ws-1', NULL, 'my-trg', 'My Trigger', \
             'webhook', '{}', 'active', '2024-01-01T00:00:00Z', 'user-1', 0, NULL, NULL, NULL)",
            trigger_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "trigger_from_row must return Err when the required `workflow_id` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn blob_from_row_null_required_storage_mode_is_err() {
        let result = with_null_row(
            "CREATE TABLE t (id TEXT NOT NULL, workspace_id TEXT NOT NULL, \
             kind TEXT NOT NULL, size_bytes INTEGER NOT NULL DEFAULT 0, \
             storage_mode TEXT, created_at TEXT NOT NULL, \
             execution_id TEXT, content_type TEXT, checksum BLOB, \
             data BLOB, external_ref TEXT, metadata TEXT, expires_at TEXT)",
            // `storage_mode` is NOT NULL in the schema but NULL here.
            "INSERT INTO t VALUES ('blob-1', 'ws-1', 'output', 42, NULL, \
             '2024-01-01T00:00:00Z', NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            blob_from_row,
        )
        .await;
        assert!(
            result.is_err(),
            "blob_from_row must return Err when the required `storage_mode` column is NULL, got Ok"
        );
    }

    #[tokio::test]
    async fn quota_from_row_i64_outside_i32_range_is_err() {
        // concurrent_executions_limit is stored as i64 in SQLite but mapped to
        // i32 in QuotaRow. A value beyond i32::MAX must produce Err, not a
        // silently wrapped integer.
        let overflow = (i32::MAX as i64) + 1; // 2_147_483_648 — one past i32::MAX
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE t (org_id TEXT NOT NULL, plan TEXT NOT NULL, \
             concurrent_executions_limit INTEGER NOT NULL, \
             concurrent_executions INTEGER NOT NULL DEFAULT 0, \
             executions_this_month INTEGER NOT NULL DEFAULT 0, \
             month_reset_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
             executions_per_month_limit INTEGER, active_workflows_limit INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // Bind the overflow value as a parameter so the query string stays static.
        sqlx::query(
            "INSERT INTO t VALUES ('org-1', 'pro', ?, 0, 0, \
             '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', NULL, NULL)",
        )
        .bind(overflow)
        .execute(&pool)
        .await
        .unwrap();
        let row = sqlx::query("SELECT * FROM t")
            .fetch_one(&pool)
            .await
            .unwrap();
        let result = quota_from_row(&row);
        assert!(
            result.is_err(),
            "quota_from_row must return Err when concurrent_executions_limit exceeds i32::MAX, got Ok"
        );
    }

    #[tokio::test]
    async fn user_from_row_failed_login_count_outside_i32_range_is_err() {
        // failed_login_count is stored as i64 in SQLite but mapped to i32 in
        // UserRow. A value beyond i32::MAX must produce Err, not wrap.
        let overflow = (i32::MAX as i64) + 1;
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE t (id TEXT NOT NULL, email TEXT NOT NULL, \
             display_name TEXT NOT NULL, created_at TEXT NOT NULL, \
             failed_login_count INTEGER NOT NULL, \
             mfa_enabled INTEGER NOT NULL DEFAULT 0, \
             version INTEGER NOT NULL DEFAULT 0, \
             email_verified_at TEXT, avatar_url TEXT, password_hash TEXT, \
             last_login_at TEXT, locked_until TEXT, mfa_secret BLOB, deleted_at TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO t VALUES ('u-1', 'a@b.com', 'Alice', '2024-01-01T00:00:00Z', \
             ?, 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
        )
        .bind(overflow)
        .execute(&pool)
        .await
        .unwrap();
        let row = sqlx::query("SELECT * FROM t")
            .fetch_one(&pool)
            .await
            .unwrap();
        let result = user_from_row(&row);
        assert!(
            result.is_err(),
            "user_from_row must return Err when failed_login_count exceeds i32::MAX, got Ok"
        );
    }
}
