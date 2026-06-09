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

fn user_from_row(r: &sqlx::sqlite::SqliteRow) -> UserRow {
    UserRow {
        id: r.try_get("id").unwrap_or_default(),
        email: r.try_get("email").unwrap_or_default(),
        email_verified_at: r.try_get("email_verified_at").ok(),
        display_name: r.try_get("display_name").unwrap_or_default(),
        avatar_url: r.try_get("avatar_url").ok(),
        password_hash: r.try_get("password_hash").ok(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        last_login_at: r.try_get("last_login_at").ok(),
        locked_until: r.try_get("locked_until").ok(),
        failed_login_count: r
            .try_get::<i64, _>("failed_login_count")
            .unwrap_or_default() as i32,
        mfa_enabled: r.try_get::<i64, _>("mfa_enabled").unwrap_or_default() != 0,
        mfa_secret: r.try_get("mfa_secret").ok(),
        version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
        deleted_at: r.try_get("deleted_at").ok(),
    }
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
        Ok(row.as_ref().map(user_from_row))
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
        Ok(row.as_ref().map(user_from_row))
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
        id: r.try_get("id").unwrap_or_default(),
        slug: r.try_get("slug").unwrap_or_default(),
        display_name: r.try_get("display_name").unwrap_or_default(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        created_by: r.try_get("created_by").unwrap_or_default(),
        plan: r.try_get("plan").unwrap_or_default(),
        billing_email: r.try_get("billing_email").ok(),
        settings: text_to_json(&r.try_get::<String, _>("settings").unwrap_or_default())?,
        version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
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
        id: r.try_get("id").unwrap_or_default(),
        org_id: r.try_get("org_id").unwrap_or_default(),
        slug: r.try_get("slug").unwrap_or_default(),
        display_name: r.try_get("display_name").unwrap_or_default(),
        description: r.try_get("description").ok(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        created_by: r.try_get("created_by").unwrap_or_default(),
        is_default: r.try_get::<i64, _>("is_default").unwrap_or_default() != 0,
        settings: text_to_json(&r.try_get::<String, _>("settings").unwrap_or_default())?,
        version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
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
    let scope_kind_txt: String = r.try_get("scope_kind").unwrap_or_default();
    let principal_kind_txt: String = r.try_get("principal_kind").unwrap_or_default();
    Ok(MembershipRow {
        // Fail-closed: an unrecognized authz-domain value is a hard
        // deserialization error, never silently coerced to a default.
        scope_kind: ScopeKind::parse(&scope_kind_txt).map_err(|bad| {
            StorageError::Serialization(format!("unknown membership scope_kind {bad:?}"))
        })?,
        scope_id: r.try_get("scope_id").unwrap_or_default(),
        principal_kind: PrincipalKind::parse(&principal_kind_txt).map_err(|bad| {
            StorageError::Serialization(format!("unknown membership principal_kind {bad:?}"))
        })?,
        principal_id: r.try_get("principal_id").unwrap_or_default(),
        role: r.try_get("role").unwrap_or_default(),
        added_at: r.try_get("added_at").unwrap_or_default(),
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
        id: r.try_get("id").unwrap_or_default(),
        workspace_id: r.try_get("workspace_id").unwrap_or_default(),
        slug: r.try_get("slug").unwrap_or_default(),
        display_name: r.try_get("display_name").unwrap_or_default(),
        kind: r.try_get("kind").unwrap_or_default(),
        config: text_to_json(&r.try_get::<String, _>("config").unwrap_or_default())?,
        created_at: r.try_get("created_at").unwrap_or_default(),
        created_by: r.try_get("created_by").unwrap_or_default(),
        version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
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
        id: r.try_get("id").unwrap_or_default(),
        workspace_id: r.try_get("workspace_id").unwrap_or_default(),
        workflow_id: r.try_get("workflow_id").unwrap_or_default(),
        slug: r.try_get("slug").unwrap_or_default(),
        display_name: r.try_get("display_name").unwrap_or_default(),
        kind: r.try_get("kind").unwrap_or_default(),
        config: text_to_json(&r.try_get::<String, _>("config").unwrap_or_default())?,
        state: r.try_get("state").unwrap_or_default(),
        run_as: r.try_get("run_as").ok(),
        webhook_path: r.try_get("webhook_path").ok(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        created_by: r.try_get("created_by").unwrap_or_default(),
        version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
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

fn quota_from_row(r: &sqlx::sqlite::SqliteRow) -> QuotaRow {
    QuotaRow {
        org_id: r.try_get("org_id").unwrap_or_default(),
        plan: r.try_get("plan").unwrap_or_default(),
        concurrent_executions_limit: r
            .try_get::<i64, _>("concurrent_executions_limit")
            .unwrap_or_default() as i32,
        executions_per_month_limit: r
            .try_get::<Option<i64>, _>("executions_per_month_limit")
            .unwrap_or_default(),
        active_workflows_limit: r
            .try_get::<Option<i64>, _>("active_workflows_limit")
            .unwrap_or_default()
            .map(|v| v as i32),
        concurrent_executions: r
            .try_get::<i64, _>("concurrent_executions")
            .unwrap_or_default() as i32,
        executions_this_month: r
            .try_get::<i64, _>("executions_this_month")
            .unwrap_or_default(),
        month_reset_at: r.try_get("month_reset_at").unwrap_or_default(),
        updated_at: r.try_get("updated_at").unwrap_or_default(),
    }
}

#[async_trait::async_trait]
impl QuotaStore for SqliteQuotaStore {
    async fn get(&self, org_id: &str) -> Result<Option<QuotaRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_quotas WHERE org_id = ?")
            .bind(org_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        Ok(row.as_ref().map(quota_from_row))
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
            return Ok(v as i32);
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
        id: r.try_get("id").unwrap_or_default(),
        org_id: r.try_get("org_id").unwrap_or_default(),
        workspace_id: r.try_get("workspace_id").ok(),
        actor_kind: r.try_get("actor_kind").unwrap_or_default(),
        actor_id: r.try_get("actor_id").ok(),
        action: r.try_get("action").unwrap_or_default(),
        target_kind: r.try_get("target_kind").ok(),
        target_id: r.try_get("target_id").ok(),
        details: opt_text_to_json(
            r.try_get::<Option<String>, _>("details")
                .unwrap_or_default(),
        )?,
        ip_address: r.try_get("ip_address").ok(),
        user_agent: r.try_get("user_agent").ok(),
        emitted_at: r.try_get("emitted_at").unwrap_or_default(),
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
        id: r.try_get("id").unwrap_or_default(),
        workspace_id: r.try_get("workspace_id").unwrap_or_default(),
        execution_id: r.try_get("execution_id").ok(),
        kind: r.try_get("kind").unwrap_or_default(),
        content_type: r.try_get("content_type").ok(),
        size_bytes: r.try_get::<i64, _>("size_bytes").unwrap_or_default(),
        checksum: r.try_get("checksum").ok(),
        storage_mode: r.try_get("storage_mode").unwrap_or_default(),
        data: r.try_get("data").ok(),
        external_ref: r.try_get("external_ref").ok(),
        metadata: opt_text_to_json(
            r.try_get::<Option<String>, _>("metadata")
                .unwrap_or_default(),
        )?,
        created_at: r.try_get("created_at").unwrap_or_default(),
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
