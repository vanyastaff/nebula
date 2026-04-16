//! Postgres implementation of [`OrgRepo`].

use async_trait::async_trait;
use sqlx::{Pool, Postgres, types::Json};

use crate::{
    error::StorageError,
    pg::map_db_err,
    repos::OrgRepo,
    rows::{OrgMemberRow, OrgRow},
};

/// Postgres-backed organization repository.
#[derive(Clone)]
pub struct PgOrgRepo {
    pool: Pool<Postgres>,
}

impl PgOrgRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

// Column order must match every `SELECT ... FROM orgs` in this file.
type OrgTuple = (
    Vec<u8>,                               // id
    String,                                // slug
    String,                                // display_name
    chrono::DateTime<chrono::Utc>,         // created_at
    Vec<u8>,                               // created_by
    String,                                // plan
    Option<String>,                        // billing_email
    Json<serde_json::Value>,               // settings
    i64,                                   // version
    Option<chrono::DateTime<chrono::Utc>>, // deleted_at
);

fn tuple_to_row(t: OrgTuple) -> OrgRow {
    OrgRow {
        id: t.0,
        slug: t.1,
        display_name: t.2,
        created_at: t.3,
        created_by: t.4,
        plan: t.5,
        billing_email: t.6,
        settings: t.7.0,
        version: t.8,
        deleted_at: t.9,
    }
}

const SELECT_COLS: &str = "id, slug, display_name, created_at, created_by, plan, billing_email, settings, version, deleted_at";

#[async_trait]
impl OrgRepo for PgOrgRepo {
    async fn create(&self, org: &OrgRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO orgs \
             (id, slug, display_name, created_at, created_by, plan, billing_email, settings, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .bind(&org.display_name)
        .bind(org.created_at)
        .bind(&org.created_by)
        .bind(&org.plan)
        .bind(org.billing_email.as_deref())
        .bind(Json(&org.settings))
        .bind(org.version)
        .bind(org.deleted_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("org", e))?;
        Ok(())
    }

    async fn get(&self, id: &[u8]) -> Result<Option<OrgRow>, StorageError> {
        let sql = format!("SELECT {SELECT_COLS} FROM orgs WHERE id = $1 AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, OrgTuple>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("org", e))?;
        Ok(row.map(tuple_to_row))
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM orgs WHERE LOWER(slug) = LOWER($1) AND deleted_at IS NULL"
        );
        let row = sqlx::query_as::<_, OrgTuple>(&sql)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("org", e))?;
        Ok(row.map(tuple_to_row))
    }

    async fn update(&self, org: &OrgRow, expected_version: i64) -> Result<(), StorageError> {
        let rows = sqlx::query(
            "UPDATE orgs SET \
                 slug = $2, display_name = $3, plan = $4, billing_email = $5, \
                 settings = $6, version = version + 1 \
             WHERE id = $1 AND version = $7 AND deleted_at IS NULL",
        )
        .bind(&org.id)
        .bind(&org.slug)
        .bind(&org.display_name)
        .bind(&org.plan)
        .bind(org.billing_email.as_deref())
        .bind(Json(&org.settings))
        .bind(expected_version)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("org", e))?
        .rows_affected();

        if rows == 0 {
            // Distinguish missing vs version mismatch.
            let actual: Option<i64> = sqlx::query_scalar("SELECT version FROM orgs WHERE id = $1")
                .bind(&org.id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| map_db_err("org", e))?;
            return match actual {
                Some(v) => Err(StorageError::conflict(
                    "org",
                    hex::encode(&org.id),
                    expected_version,
                    v,
                )),
                None => Err(StorageError::not_found("org", hex::encode(&org.id))),
            };
        }
        Ok(())
    }

    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE orgs SET deleted_at = NOW(), version = version + 1 \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("org", e))?;
        Ok(())
    }

    async fn add_member(&self, member: &OrgMemberRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO org_members \
             (org_id, principal_kind, principal_id, role, invited_at, invited_by, accepted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&member.org_id)
        .bind(&member.principal_kind)
        .bind(&member.principal_id)
        .bind(&member.role)
        .bind(member.invited_at)
        .bind(member.invited_by.as_deref())
        .bind(member.accepted_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("org_member", e))?;
        Ok(())
    }

    async fn remove_member(
        &self,
        org_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<(), StorageError> {
        sqlx::query(
            "DELETE FROM org_members \
             WHERE org_id = $1 AND principal_kind = $2 AND principal_id = $3",
        )
        .bind(org_id)
        .bind(principal_kind)
        .bind(principal_id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("org_member", e))?;
        Ok(())
    }

    async fn get_member_role(
        &self,
        org_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Option<String>, StorageError> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM org_members \
             WHERE org_id = $1 AND principal_kind = $2 AND principal_id = $3",
        )
        .bind(org_id)
        .bind(principal_kind)
        .bind(principal_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_db_err("org_member", e))?;
        Ok(role)
    }

    async fn list_members(&self, org_id: &[u8]) -> Result<Vec<OrgMemberRow>, StorageError> {
        type T = (
            Vec<u8>,
            String,
            Vec<u8>,
            String,
            chrono::DateTime<chrono::Utc>,
            Option<Vec<u8>>,
            Option<chrono::DateTime<chrono::Utc>>,
        );
        let rows = sqlx::query_as::<_, T>(
            "SELECT org_id, principal_kind, principal_id, role, invited_at, invited_by, accepted_at \
             FROM org_members WHERE org_id = $1 ORDER BY invited_at",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_db_err("org_member", e))?;

        Ok(rows
            .into_iter()
            .map(|t| OrgMemberRow {
                org_id: t.0,
                principal_kind: t.1,
                principal_id: t.2,
                role: t.3,
                invited_at: t.4,
                invited_by: t.5,
                accepted_at: t.6,
            })
            .collect())
    }

    async fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Vec<OrgMemberRow>, StorageError> {
        type T = (
            Vec<u8>,
            String,
            Vec<u8>,
            String,
            chrono::DateTime<chrono::Utc>,
            Option<Vec<u8>>,
            Option<chrono::DateTime<chrono::Utc>>,
        );
        let rows = sqlx::query_as::<_, T>(
            "SELECT org_id, principal_kind, principal_id, role, invited_at, invited_by, accepted_at \
             FROM org_members WHERE principal_kind = $1 AND principal_id = $2 \
             ORDER BY invited_at",
        )
        .bind(principal_kind)
        .bind(principal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_db_err("org_member", e))?;

        Ok(rows
            .into_iter()
            .map(|t| OrgMemberRow {
                org_id: t.0,
                principal_kind: t.1,
                principal_id: t.2,
                role: t.3,
                invited_at: t.4,
                invited_by: t.5,
                accepted_at: t.6,
            })
            .collect())
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;
    use crate::test_support::{random_id, test_org};

    /// Connect to `DATABASE_URL`, or return `None` to skip the test.
    async fn pool() -> Option<Pool<Postgres>> {
        let url = std::env::var("DATABASE_URL").ok()?;
        Some(sqlx::PgPool::connect(&url).await.expect("connect"))
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let Some(pool) = pool().await else { return };
        let repo = PgOrgRepo::new(pool);
        let org = test_org(&format!("test-{}", hex::encode(&random_id()[..4])));
        repo.create(&org).await.expect("create");
        let loaded = repo.get(&org.id).await.expect("get").expect("some");
        assert_eq!(loaded.slug, org.slug);
    }

    #[tokio::test]
    async fn get_by_slug_case_insensitive() {
        let Some(pool) = pool().await else { return };
        let repo = PgOrgRepo::new(pool);
        let slug = format!("ACME-{}", hex::encode(&random_id()[..4]));
        let mut org = test_org(&slug);
        org.slug = slug.clone();
        repo.create(&org).await.expect("create");
        let loaded = repo
            .get_by_slug(&slug.to_lowercase())
            .await
            .expect("get")
            .expect("some");
        assert_eq!(loaded.id, org.id);
    }
}
