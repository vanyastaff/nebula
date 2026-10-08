//! `orgs`: tenants; slug is unique among active rows.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::OrgRow;
use nebula_storage_port::store::OrgStore;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;

use super::{
    cas_disambiguate, encode_instant, encode_version, instant, json, json_text, optional,
    optional_instant, required, soft_delete_by_id, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// SQLite-backed `orgs` store.
#[derive(Clone, Debug)]
pub struct SqliteOrgStore {
    pool: SqlitePool,
}

impl SqliteOrgStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_org(row: &SqliteRow) -> Result<OrgRow, StorageError> {
    Ok(OrgRow {
        id: required(row, "id")?,
        slug: required(row, "slug")?,
        display_name: required(row, "display_name")?,
        created_at: instant(row, "created_at")?,
        created_by: required(row, "created_by")?,
        plan: required(row, "plan")?,
        billing_email: optional(row, "billing_email")?,
        settings: json(row, "settings")?,
        version: version(row)?,
        deleted_at: optional_instant(row, "deleted_at")?,
    })
}

/// Insert `org` on `executor` — shared with tenant provisioning.
pub(super) async fn insert_org<'c, E>(executor: E, org: &OrgRow) -> Result<(), StorageError>
where
    E: sqlx::Executor<'c, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO orgs (id, slug, display_name, created_at, created_by, \
         plan, billing_email, settings, version, deleted_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&org.id)
    .bind(&org.slug)
    .bind(&org.display_name)
    .bind(encode_instant(org.created_at))
    .bind(&org.created_by)
    .bind(&org.plan)
    .bind(&org.billing_email)
    .bind(json_text(&org.settings))
    .bind(encode_version(org.version)?)
    .bind(org.deleted_at.map(encode_instant))
    .execute(executor)
    .await
    .map_err(|error| storage_error_for("org", error))?;
    Ok(())
}

#[async_trait::async_trait]
impl OrgStore for SqliteOrgStore {
    async fn create(&self, row: OrgRow) -> Result<(), StorageError> {
        insert_org(&self.pool, &row).await
    }

    async fn get(&self, id: &str) -> Result<Option<OrgRow>, StorageError> {
        sqlx::query("SELECT * FROM orgs WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_org)
            .transpose()
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        sqlx::query("SELECT * FROM orgs WHERE slug = ? AND deleted_at IS NULL")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_org)
            .transpose()
    }

    async fn update(&self, row: OrgRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE orgs SET slug = ?, display_name = ?, plan = ?, \
             billing_email = ?, settings = ?, version = ? \
             WHERE id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.plan)
        .bind(&row.billing_email)
        .bind(json_text(&row.settings))
        .bind(encode_version(row.version)?)
        .bind(&row.id)
        .bind(encode_version(expected_version)?)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("org", error))?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        cas_disambiguate(&self.pool, "orgs", "org", &row.id, expected_version).await
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        soft_delete_by_id(&self.pool, "orgs", "org", id).await
    }
}
