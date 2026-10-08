//! `orgs`: tenants; slug is unique among active rows.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::OrgRow;
use nebula_storage_port::store::OrgStore;
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use sqlx::types::Json;

use super::{
    cas_disambiguate, encode_version, json, optional, required, soft_delete_by_id, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// Postgres-backed `orgs` store.
#[derive(Clone, Debug)]
pub struct PgOrgStore {
    pool: PgPool,
}

impl PgOrgStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_org(row: &PgRow) -> Result<OrgRow, StorageError> {
    Ok(OrgRow {
        id: required(row, "id")?,
        slug: required(row, "slug")?,
        display_name: required(row, "display_name")?,
        created_at: required(row, "created_at")?,
        created_by: required(row, "created_by")?,
        plan: required(row, "plan")?,
        billing_email: optional(row, "billing_email")?,
        settings: json(row, "settings")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?,
    })
}

/// Insert `org` on `executor` — shared with tenant provisioning. A unique
/// violation is `Duplicate { entity: "org", .. }`.
pub(super) async fn insert_org<'c, E>(executor: E, org: &OrgRow) -> Result<(), StorageError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO orgs (id, slug, display_name, created_at, created_by, \
         plan, billing_email, settings, version, deleted_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&org.id)
    .bind(&org.slug)
    .bind(&org.display_name)
    .bind(org.created_at)
    .bind(&org.created_by)
    .bind(&org.plan)
    .bind(&org.billing_email)
    .bind(Json(&org.settings))
    .bind(encode_version(org.version)?)
    .bind(org.deleted_at)
    .execute(executor)
    .await
    .map_err(|error| storage_error_for("org", error))?;
    Ok(())
}

#[async_trait::async_trait]
impl OrgStore for PgOrgStore {
    async fn create(&self, row: OrgRow) -> Result<(), StorageError> {
        insert_org(&self.pool, &row).await
    }

    async fn get(&self, id: &str) -> Result<Option<OrgRow>, StorageError> {
        sqlx::query("SELECT * FROM orgs WHERE id = $1 AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_org)
            .transpose()
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        sqlx::query("SELECT * FROM orgs WHERE slug = $1 AND deleted_at IS NULL")
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
            "UPDATE orgs SET slug = $1, display_name = $2, plan = $3, \
             billing_email = $4, settings = $5, version = $6 \
             WHERE id = $7 AND deleted_at IS NULL AND version = $8",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.plan)
        .bind(&row.billing_email)
        .bind(Json(&row.settings))
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
