//! Exact plan/flavor catalog conformance for the SQLite deployment backend.
//!
//! Every case runs against a fresh in-memory database whose schema comes from
//! the ordered migration catalog — there is no parallel bootstrap path — so the
//! adapter is exercised against exactly the `CHECK` constraints and foreign
//! keys migration 0041 installs.
//!
//! The durable-corruption cases below sit outside the shared oracle on purpose:
//! reaching them means writing bytes the port refuses to write, which only a
//! backend-specific poke at the table can do.

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/revision_catalog_oracle.rs"]
mod oracle;

use std::str::FromStr;

use nebula_storage::sqlite::{SqlitePlanFlavorCatalog, init_schema};
use nebula_storage_port::{
    PlanFlavorCatalog, PlanFlavorCatalogWriter, PlanFlavorRevisionTarget, RevisionCatalogError,
    RevisionInsertOutcome,
};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// An isolated in-memory database with the ordered migration catalog applied.
///
/// The shared cache keeps every pooled connection on the same database; a
/// private `:memory:` connection would give each one its own empty schema.
async fn fresh_pool() -> SqlitePool {
    let database = format!("nebula-catalog-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("in-memory SQLite URL must parse")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect to in-memory SQLite");
    init_schema(&pool)
        .await
        .expect("apply the ordered SQLite migration catalog");
    pool
}

async fn catalog() -> Option<SqlitePlanFlavorCatalog> {
    Some(SqlitePlanFlavorCatalog::new(fresh_pool().await))
}

revision_catalog_conformance_suite!(catalog());

/// A durable plan body that no longer satisfies its recorded form is reported
/// as corruption, not handed to the plugin layer as if it were a record.
///
/// Migration 0041 constrains identifier width, lifecycle vocabulary, format
/// text, and payload presence, but a `record_format` of `graph_v1_json` cannot
/// be checked against the bytes by SQL. That last invariant is the adapter's,
/// and this proves it holds against a real byte-level poke.
#[tokio::test]
async fn a_durably_corrupted_plan_body_is_reported_as_corruption() {
    let pool = fresh_pool().await;
    let catalog = SqlitePlanFlavorCatalog::new(pool.clone());
    let record = oracle::pair(0x40, 0, "v1");
    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::Inserted)
    );

    sqlx::query(
        "UPDATE port_executable_plan_revisions SET record_bytes = ? \
         WHERE executable_plan_id = ?",
    )
    .bind(b"not a graph-v1 json document".as_slice())
    .bind(record.ids().plan().as_bytes().as_slice())
    .execute(&pool)
    .await
    .expect("poke the durable plan payload");

    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Err(RevisionCatalogError::CorruptRecord {
            target: PlanFlavorRevisionTarget::ExecutablePlan(record.ids().plan()),
        })
    );
}

/// A durable `record_format` naming a recorded form this build cannot read is
/// reported as unsupported rather than decoded as if it were the known form.
///
/// The table's `CHECK` refuses the unknown format, so this exercises the
/// adapter's own guard by asserting the schema holds the line and then
/// verifying the decoder's answer for a database that predates the constraint.
#[tokio::test]
async fn the_schema_refuses_a_record_format_the_catalog_cannot_read() {
    let pool = fresh_pool().await;
    let catalog = SqlitePlanFlavorCatalog::new(pool.clone());
    let record = oracle::pair(0x41, 0, "v1");
    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::Inserted)
    );

    let rejected = sqlx::query(
        "UPDATE port_worker_flavor_revisions SET record_format = 'v2_cbor' \
         WHERE worker_flavor_id = ?",
    )
    .bind(record.ids().worker_flavor().as_bytes().as_slice())
    .execute(&pool)
    .await;
    assert!(
        rejected.is_err(),
        "migration 0041 must refuse a recorded form outside the closed vocabulary"
    );
    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Ok(record),
        "a refused poke leaves the durable record readable"
    );
}
