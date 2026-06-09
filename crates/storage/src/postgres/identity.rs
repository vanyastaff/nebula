//! Postgres identity-zoo stores over the port-scoped schema.
//!
//! Each aggregate is a `port_*` table in `schema.sql`. Every tenant- or
//! parent-scoped query carries its scope predicate (`WHERE org_id = $1`,
//! `WHERE workspace_id = $1 AND org_id = $2`, …) and active-row reads add
//! `AND deleted_at IS NULL`, so a cross-scope `get` yields `Ok(None)` and
//! a cross-scope `update` / `soft_delete` is `NotFound` — an id outside
//! the caller's scope is indistinguishable from one that does not exist
//! (no existence oracle, spec §6.1), exactly as the in-memory and SQLite
//! backends behave.
//!
//! First-writer-wins uniqueness (email / slug among *active* rows) is a
//! partial unique index `WHERE deleted_at IS NULL`, so a soft-deleted row
//! frees its key. Optimistic CAS is a single conditional `UPDATE … WHERE
//! version = $N` followed by a disambiguating read (gone ⇒ `NotFound`,
//! moved ⇒ `Conflict`). JSON columns are `JSONB` mapped through
//! [`sqlx::types::Json`]; binary columns are `BYTEA`.

use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TriggerRow, UserRow, WorkspaceRow,
};
use nebula_storage_port::store::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore, TriggerStore,
    UserStore, WorkspaceStore,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::types::Json;
use sqlx::{PgPool, Row};

use super::execution::conn_err;

// ── Users ─────────────────────────────────────────────────────────────────

/// Postgres-backed `users` store. Email is unique among active rows
/// (case-insensitive, via a `lower(email)` partial unique index).
#[derive(Clone, Debug)]
pub struct PgUserStore {
    pool: PgPool,
}

impl PgUserStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn user_from_row(r: &sqlx::postgres::PgRow) -> Result<UserRow, StorageError> {
    Ok(UserRow {
        id: r.try_get("id").map_err(conn_err)?,
        email: r.try_get("email").map_err(conn_err)?,
        email_verified_at: r.try_get("email_verified_at").ok(),
        display_name: r.try_get("display_name").map_err(conn_err)?,
        avatar_url: r.try_get("avatar_url").ok(),
        password_hash: r.try_get("password_hash").ok(),
        created_at: r.try_get("created_at").map_err(conn_err)?,
        last_login_at: r.try_get("last_login_at").ok(),
        locked_until: r.try_get("locked_until").ok(),
        failed_login_count: r
            .try_get::<i64, _>("failed_login_count")
            .map_err(conn_err)? as i32,
        mfa_enabled: r.try_get("mfa_enabled").map_err(conn_err)?,
        mfa_secret: r.try_get("mfa_secret").ok(),
        version: r.try_get::<i64, _>("version").map_err(conn_err)? as u64,
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl UserStore for PgUserStore {
    async fn create(&self, row: UserRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_users (id, email, email_verified_at, display_name, \
             avatar_url, password_hash, created_at, last_login_at, locked_until, \
             failed_login_count, mfa_enabled, mfa_secret, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
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
        .bind(row.mfa_enabled)
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
        let row = sqlx::query("SELECT * FROM port_users WHERE id = $1 AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(user_from_row).transpose()
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<UserRow>, StorageError> {
        let row = sqlx::query(
            "SELECT * FROM port_users \
             WHERE lower(email) = lower($1) AND deleted_at IS NULL",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(user_from_row).transpose()
    }

    async fn update(&self, row: UserRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_users SET email = $1, email_verified_at = $2, \
             display_name = $3, avatar_url = $4, password_hash = $5, \
             last_login_at = $6, locked_until = $7, failed_login_count = $8, \
             mfa_enabled = $9, mfa_secret = $10, version = $11 \
             WHERE id = $12 AND deleted_at IS NULL AND version = $13",
        )
        .bind(&row.email)
        .bind(&row.email_verified_at)
        .bind(&row.display_name)
        .bind(&row.avatar_url)
        .bind(&row.password_hash)
        .bind(&row.last_login_at)
        .bind(&row.locked_until)
        .bind(i64::from(row.failed_login_count))
        .bind(row.mfa_enabled)
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

/// Postgres-backed `orgs` store. Slug is unique among active rows.
#[derive(Clone, Debug)]
pub struct PgOrgStore {
    pool: PgPool,
}

impl PgOrgStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn org_from_row(r: &sqlx::postgres::PgRow) -> Result<OrgRow, StorageError> {
    Ok(OrgRow {
        id: r.try_get("id").map_err(conn_err)?,
        slug: r.try_get("slug").map_err(conn_err)?,
        display_name: r.try_get("display_name").map_err(conn_err)?,
        created_at: r.try_get("created_at").map_err(conn_err)?,
        created_by: r.try_get("created_by").map_err(conn_err)?,
        plan: r.try_get("plan").map_err(conn_err)?,
        billing_email: r.try_get("billing_email").ok(),
        settings: r
            .try_get::<Json<serde_json::Value>, _>("settings")
            .map(|j| j.0)
            .map_err(conn_err)?,
        version: r.try_get::<i64, _>("version").map_err(conn_err)? as u64,
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl OrgStore for PgOrgStore {
    async fn create(&self, row: OrgRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_orgs (id, slug, display_name, created_at, created_by, \
             plan, billing_email, settings, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&row.id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(&row.plan)
        .bind(&row.billing_email)
        .bind(Json(&row.settings))
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
        let row = sqlx::query("SELECT * FROM port_orgs WHERE id = $1 AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(org_from_row).transpose()
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_orgs WHERE slug = $1 AND deleted_at IS NULL")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(org_from_row).transpose()
    }

    async fn update(&self, row: OrgRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_orgs SET slug = $1, display_name = $2, plan = $3, \
             billing_email = $4, settings = $5, version = $6 \
             WHERE id = $7 AND deleted_at IS NULL AND version = $8",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.plan)
        .bind(&row.billing_email)
        .bind(Json(&row.settings))
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

/// Postgres-backed `workspaces` store (scoped by parent org). Slug is
/// unique among active rows *per org*.
#[derive(Clone, Debug)]
pub struct PgWorkspaceStore {
    pool: PgPool,
}

impl PgWorkspaceStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn workspace_from_row(r: &sqlx::postgres::PgRow) -> Result<WorkspaceRow, StorageError> {
    Ok(WorkspaceRow {
        id: r.try_get("id").map_err(conn_err)?,
        org_id: r.try_get("org_id").map_err(conn_err)?,
        slug: r.try_get("slug").map_err(conn_err)?,
        display_name: r.try_get("display_name").map_err(conn_err)?,
        description: r.try_get("description").ok(),
        created_at: r.try_get("created_at").map_err(conn_err)?,
        created_by: r.try_get("created_by").map_err(conn_err)?,
        is_default: r.try_get("is_default").map_err(conn_err)?,
        settings: r
            .try_get::<Json<serde_json::Value>, _>("settings")
            .map(|j| j.0)
            .map_err(conn_err)?,
        version: r.try_get::<i64, _>("version").map_err(conn_err)? as u64,
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl WorkspaceStore for PgWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_workspaces (id, org_id, slug, display_name, \
             description, created_at, created_by, is_default, settings, version, \
             deleted_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&row.id)
        .bind(&row.org_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.description)
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(row.is_default)
        .bind(Json(&row.settings))
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
             WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
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
             WHERE org_id = $1 AND deleted_at IS NULL ORDER BY id",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter().map(workspace_from_row).collect()
    }

    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_workspaces SET slug = $1, display_name = $2, \
             description = $3, is_default = $4, settings = $5, version = $6 \
             WHERE org_id = $7 AND id = $8 AND deleted_at IS NULL \
             AND version = $9",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.description)
        .bind(row.is_default)
        .bind(Json(&row.settings))
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
            "SELECT version FROM port_workspaces WHERE org_id = $1 AND id = $2",
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
            "UPDATE port_workspaces SET deleted_at = $1 \
             WHERE org_id = $2 AND id = $3 AND deleted_at IS NULL",
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

/// Postgres-backed `org_members` + `workspace_members` store.
#[derive(Clone, Debug)]
pub struct PgMembershipStore {
    pool: PgPool,
}

impl PgMembershipStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn membership_from_row(r: &sqlx::postgres::PgRow) -> Result<MembershipRow, StorageError> {
    let scope_kind_txt: String = r.try_get("scope_kind").map_err(conn_err)?;
    let principal_kind_txt: String = r.try_get("principal_kind").map_err(conn_err)?;
    Ok(MembershipRow {
        // Fail-closed: an unrecognized authz-domain value is a hard
        // deserialization error, never silently coerced.
        scope_kind: ScopeKind::parse(&scope_kind_txt).map_err(|bad| {
            StorageError::Serialization(format!("unknown membership scope_kind {bad:?}"))
        })?,
        scope_id: r.try_get("scope_id").map_err(conn_err)?,
        principal_kind: PrincipalKind::parse(&principal_kind_txt).map_err(|bad| {
            StorageError::Serialization(format!("unknown membership principal_kind {bad:?}"))
        })?,
        principal_id: r.try_get("principal_id").map_err(conn_err)?,
        role: r.try_get("role").map_err(conn_err)?,
        added_at: r.try_get("added_at").map_err(conn_err)?,
        added_by: r.try_get("added_by").ok(),
    })
}

#[async_trait::async_trait]
impl MembershipStore for PgMembershipStore {
    async fn upsert(&self, row: MembershipRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_memberships (scope_kind, scope_id, principal_kind, \
             principal_id, role, added_at, added_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
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
            "SELECT * FROM port_memberships WHERE scope_kind = $1 AND scope_id = $2 \
             AND principal_kind = $3 AND principal_id = $4",
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
             WHERE scope_kind = $1 AND scope_id = $2 ORDER BY principal_id",
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
            "DELETE FROM port_memberships WHERE scope_kind = $1 AND scope_id = $2 \
             AND principal_kind = $3 AND principal_id = $4",
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

/// Postgres-backed `resources` store. Slug is unique among active rows
/// per workspace scope.
#[derive(Clone, Debug)]
pub struct PgResourceStore {
    pool: PgPool,
}

impl PgResourceStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn resource_from_row(r: &sqlx::postgres::PgRow) -> Result<ResourceRow, StorageError> {
    Ok(ResourceRow {
        id: r.try_get("id").map_err(conn_err)?,
        workspace_id: r.try_get("workspace_id").map_err(conn_err)?,
        slug: r.try_get("slug").map_err(conn_err)?,
        display_name: r.try_get("display_name").map_err(conn_err)?,
        kind: r.try_get("kind").map_err(conn_err)?,
        config: r
            .try_get::<Json<serde_json::Value>, _>("config")
            .map(|j| j.0)
            .map_err(conn_err)?,
        created_at: r.try_get("created_at").map_err(conn_err)?,
        created_by: r.try_get("created_by").map_err(conn_err)?,
        version: r.try_get::<i64, _>("version").map_err(conn_err)? as u64,
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl ResourceStore for PgResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_resources (id, workspace_id, org_id, slug, \
             display_name, kind, config, created_at, created_by, version, \
             deleted_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
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
             WHERE workspace_id = $1 AND org_id = $2 AND id = $3 \
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
             WHERE workspace_id = $1 AND org_id = $2 AND deleted_at IS NULL \
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
            "UPDATE port_resources SET slug = $1, display_name = $2, kind = $3, \
             config = $4, version = $5 WHERE workspace_id = $6 AND org_id = $7 \
             AND id = $8 AND deleted_at IS NULL AND version = $9",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
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
             WHERE workspace_id = $1 AND org_id = $2 AND id = $3",
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
            "UPDATE port_resources SET deleted_at = $1 \
             WHERE workspace_id = $2 AND org_id = $3 AND id = $4 \
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

/// Postgres-backed `triggers` store.
#[derive(Clone, Debug)]
pub struct PgTriggerStore {
    pool: PgPool,
}

impl PgTriggerStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn trigger_from_row(r: &sqlx::postgres::PgRow) -> Result<TriggerRow, StorageError> {
    Ok(TriggerRow {
        id: r.try_get("id").map_err(conn_err)?,
        workspace_id: r.try_get("workspace_id").map_err(conn_err)?,
        workflow_id: r.try_get("workflow_id").map_err(conn_err)?,
        slug: r.try_get("slug").map_err(conn_err)?,
        display_name: r.try_get("display_name").map_err(conn_err)?,
        kind: r.try_get("kind").map_err(conn_err)?,
        config: r
            .try_get::<Json<serde_json::Value>, _>("config")
            .map(|j| j.0)
            .map_err(conn_err)?,
        state: r.try_get("state").map_err(conn_err)?,
        run_as: r.try_get("run_as").ok(),
        webhook_path: r.try_get("webhook_path").ok(),
        created_at: r.try_get("created_at").map_err(conn_err)?,
        created_by: r.try_get("created_by").map_err(conn_err)?,
        version: r.try_get::<i64, _>("version").map_err(conn_err)? as u64,
        deleted_at: r.try_get("deleted_at").ok(),
    })
}

#[async_trait::async_trait]
impl TriggerStore for PgTriggerStore {
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_triggers (id, workspace_id, org_id, workflow_id, \
             slug, display_name, kind, config, state, run_as, webhook_path, \
             created_at, created_by, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
             $14, $15)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
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
             WHERE workspace_id = $1 AND org_id = $2 AND id = $3 \
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
             WHERE workspace_id = $1 AND org_id = $2 AND deleted_at IS NULL \
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
            "UPDATE port_triggers SET workflow_id = $1, slug = $2, \
             display_name = $3, kind = $4, config = $5, state = $6, \
             run_as = $7, webhook_path = $8, version = $9 \
             WHERE workspace_id = $10 AND org_id = $11 AND id = $12 \
             AND deleted_at IS NULL AND version = $13",
        )
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
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
             WHERE workspace_id = $1 AND org_id = $2 AND id = $3",
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
            "UPDATE port_triggers SET deleted_at = $1 \
             WHERE workspace_id = $2 AND org_id = $3 AND id = $4 \
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

/// Postgres-backed `org_quotas` + `org_quota_usage` store.
#[derive(Clone, Debug)]
pub struct PgQuotaStore {
    pool: PgPool,
}

impl PgQuotaStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn quota_from_row(r: &sqlx::postgres::PgRow) -> Result<QuotaRow, StorageError> {
    Ok(QuotaRow {
        org_id: r.try_get("org_id").map_err(conn_err)?,
        plan: r.try_get("plan").map_err(conn_err)?,
        concurrent_executions_limit: r
            .try_get::<i64, _>("concurrent_executions_limit")
            .map_err(conn_err)? as i32,
        executions_per_month_limit: r
            .try_get::<Option<i64>, _>("executions_per_month_limit")
            .map_err(conn_err)?,
        active_workflows_limit: r
            .try_get::<Option<i64>, _>("active_workflows_limit")
            .map_err(conn_err)?
            .map(|v| v as i32),
        concurrent_executions: r
            .try_get::<i64, _>("concurrent_executions")
            .map_err(conn_err)? as i32,
        executions_this_month: r
            .try_get::<i64, _>("executions_this_month")
            .map_err(conn_err)?,
        month_reset_at: r.try_get("month_reset_at").map_err(conn_err)?,
        updated_at: r.try_get("updated_at").map_err(conn_err)?,
    })
}

#[async_trait::async_trait]
impl QuotaStore for PgQuotaStore {
    async fn get(&self, org_id: &str) -> Result<Option<QuotaRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_quotas WHERE org_id = $1")
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
             updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
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
        // Conditional adjustment guards the floor in the WHERE so a
        // would-be-negative result affects zero rows and is rejected.
        let updated = sqlx::query_scalar::<_, i64>(
            "UPDATE port_quotas \
             SET concurrent_executions = concurrent_executions + $1 \
             WHERE org_id = $2 AND concurrent_executions + $1 >= 0 \
             RETURNING concurrent_executions",
        )
        .bind(i64::from(delta))
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        if let Some(v) = updated {
            return Ok(v as i32);
        }
        // Disambiguate: no such org ⇒ NotFound; otherwise the guard
        // rejected a below-zero adjustment ⇒ Conflict.
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT concurrent_executions FROM port_quotas WHERE org_id = $1",
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

/// Postgres-backed `audit_log` store. Append-only; reads are
/// newest-first.
#[derive(Clone, Debug)]
pub struct PgAuditStore {
    pool: PgPool,
}

impl PgAuditStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn audit_from_row(r: &sqlx::postgres::PgRow) -> Result<AuditLogRow, StorageError> {
    Ok(AuditLogRow {
        id: r.try_get("id").map_err(conn_err)?,
        org_id: r.try_get("org_id").map_err(conn_err)?,
        workspace_id: r.try_get("workspace_id").ok(),
        actor_kind: r.try_get("actor_kind").map_err(conn_err)?,
        actor_id: r.try_get("actor_id").ok(),
        action: r.try_get("action").map_err(conn_err)?,
        target_kind: r.try_get("target_kind").ok(),
        target_id: r.try_get("target_id").ok(),
        details: r
            .try_get::<Option<Json<serde_json::Value>>, _>("details")
            .map_err(conn_err)?
            .map(|j| j.0),
        ip_address: r.try_get("ip_address").ok(),
        user_agent: r.try_get("user_agent").ok(),
        emitted_at: r.try_get("emitted_at").map_err(conn_err)?,
    })
}

#[async_trait::async_trait]
impl AuditStore for PgAuditStore {
    async fn append(&self, row: AuditLogRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_audit_log (id, org_id, workspace_id, actor_kind, \
             actor_id, action, target_kind, target_id, details, ip_address, \
             user_agent, emitted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(&row.id)
        .bind(&row.org_id)
        .bind(&row.workspace_id)
        .bind(&row.actor_kind)
        .bind(&row.actor_id)
        .bind(&row.action)
        .bind(&row.target_kind)
        .bind(&row.target_id)
        .bind(row.details.as_ref().map(Json))
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
            "SELECT * FROM port_audit_log WHERE org_id = $1 \
             ORDER BY emitted_at DESC, id DESC LIMIT $2",
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

/// Postgres-backed `blobs` store.
#[derive(Clone, Debug)]
pub struct PgBlobStore {
    pool: PgPool,
}

impl PgBlobStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn blob_from_row(r: &sqlx::postgres::PgRow) -> Result<BlobRow, StorageError> {
    Ok(BlobRow {
        id: r.try_get("id").map_err(conn_err)?,
        workspace_id: r.try_get("workspace_id").map_err(conn_err)?,
        execution_id: r.try_get("execution_id").ok(),
        kind: r.try_get("kind").map_err(conn_err)?,
        content_type: r.try_get("content_type").ok(),
        size_bytes: r.try_get::<i64, _>("size_bytes").map_err(conn_err)?,
        checksum: r.try_get("checksum").ok(),
        storage_mode: r.try_get("storage_mode").map_err(conn_err)?,
        data: r.try_get("data").ok(),
        external_ref: r.try_get("external_ref").ok(),
        metadata: r
            .try_get::<Option<Json<serde_json::Value>>, _>("metadata")
            .map_err(conn_err)?
            .map(|j| j.0),
        created_at: r.try_get("created_at").map_err(conn_err)?,
        expires_at: r.try_get("expires_at").ok(),
    })
}

#[async_trait::async_trait]
impl BlobStore for PgBlobStore {
    async fn put(&self, row: BlobRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_blobs (id, workspace_id, execution_id, kind, \
             content_type, size_bytes, checksum, storage_mode, data, \
             external_ref, metadata, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
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
        .bind(row.metadata.as_ref().map(Json))
        .bind(&row.created_at)
        .bind(&row.expires_at)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn get(&self, workspace_id: &str, id: &str) -> Result<Option<BlobRow>, StorageError> {
        let row = sqlx::query("SELECT * FROM port_blobs WHERE workspace_id = $1 AND id = $2")
            .bind(workspace_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(conn_err)?;
        row.as_ref().map(blob_from_row).transpose()
    }

    async fn delete(&self, workspace_id: &str, id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM port_blobs WHERE workspace_id = $1 AND id = $2")
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
             WHERE expires_at IS NOT NULL AND expires_at <= $1",
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
/// format the port DTOs use (consistent with the other backends).
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Disambiguate a zero-row CAS `UPDATE` on a single-PK `id` table whose
/// rows soft-delete via `deleted_at`: the row is gone (or soft-deleted) ⇒
/// `NotFound`; the version moved ⇒ `Conflict { actual }`.
async fn cas_disambiguate(
    pool: &PgPool,
    table: &str,
    entity: &'static str,
    id: &str,
    expected_version: u64,
) -> Result<(), StorageError> {
    // `table` is a fixed internal literal (never user input), so the
    // format here cannot be an injection vector.
    let sql = format!("SELECT version FROM {table} WHERE id = $1");
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
    pool: &PgPool,
    table: &str,
    entity: &'static str,
    id: &str,
) -> Result<(), StorageError> {
    let sql = format!("UPDATE {table} SET deleted_at = $1 WHERE id = $2 AND deleted_at IS NULL");
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
