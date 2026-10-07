//! Resource status conformance for the SQLite adapter.

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/resource_status_oracle.rs"]
mod oracle;

#[path = "support/execution_parents.rs"]
#[expect(
    dead_code,
    reason = "status snapshots need their tenant, not a workflow"
)]
mod execution_parents;

use std::time::Duration;

use nebula_storage::sqlite::{
    SqliteResourceStatusStore, SqliteResourceStore, SqliteTenantProvisioningStore, init_schema,
};
use nebula_storage_port::dto::StatusWorkerId;
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

struct SqliteTimeControl {
    pool: SqlitePool,
}

#[async_trait::async_trait]
impl oracle::ResourceStatusTimeControl for SqliteTimeControl {
    async fn pass(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    async fn expire_long_ago(&self, worker: &StatusWorkerId) {
        sqlx::query("UPDATE resource_status_heartbeats SET expires_at = CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER) - 7200000000 WHERE worker_id = ?")
            .bind(worker.as_str())
            .execute(&self.pool)
            .await
            .expect("heartbeat backdate succeeds");
    }
}

async fn store() -> Option<(SqliteResourceStatusStore, SqliteTimeControl)> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite pool connects");
    init_schema(&pool).await.expect("schema initializes");
    // A snapshot belongs to its stored resource.
    execution_parents::provision_scope(
        &SqliteTenantProvisioningStore::new(pool.clone()),
        &oracle::published_scope(),
    )
    .await;
    oracle::seed_published_resources(&SqliteResourceStore::new(pool.clone())).await;
    Some((
        SqliteResourceStatusStore::new(pool.clone()),
        SqliteTimeControl { pool },
    ))
}

resource_status_conformance_suite!(store());
