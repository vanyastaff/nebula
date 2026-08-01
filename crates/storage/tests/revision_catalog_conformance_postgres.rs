//! Exact plan/flavor catalog conformance for the PostgreSQL deployment backend.
//!
//! PostgreSQL is a deployment backend, so its absence is a job failure, never a
//! silent substitution by SQLite or the in-memory reference model. With
//! `NEBULA_REQUIRE_POSTGRES=1` and no `DATABASE_URL`, every case fails; without
//! that variable a developer without a database sees the cases report that the
//! backend was unreachable and assert nothing.
//!
//! Run locally via:
//!   DATABASE_URL=postgres://... cargo nextest run \
//!     -p nebula-storage --features postgres \
//!     --test revision_catalog_conformance_postgres

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/revision_catalog_oracle.rs"]
mod oracle;

use nebula_storage::postgres::{PgPlanFlavorCatalog, init_schema};
use nebula_storage_port::{
    PlanFlavorCatalog, PlanFlavorCatalogWriter, PlanFlavorRevisionTarget, RevisionCatalogError,
    RevisionInsertOutcome,
};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;

/// A private metrics registry for this backend under test.
///
/// Production wiring threads the process-shared registry so a scraper observes
/// the catalog's conflict and unknown-outcome counts; a conformance run only
/// needs the counters to be bindable, and a private registry keeps concurrent
/// cases from sharing series.
fn registry() -> nebula_metrics::MetricsRegistry {
    nebula_metrics::MetricsRegistry::new()
}

static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Connect to `DATABASE_URL` and apply the ordered migration catalog, or report
/// that PostgreSQL is unreachable.
///
/// The oracle folds a per-process namespace into every identity, so cases share
/// one database without meeting an earlier run's immutable revisions.
async fn pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1: \
                 PostgreSQL is a deployment backend and is never substituted"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    SCHEMA_READY
        .get_or_init(|| async {
            init_schema(&pool)
                .await
                .expect("apply the ordered PostgreSQL migration catalog");
        })
        .await;
    Some(pool)
}

async fn catalog() -> Option<PgPlanFlavorCatalog> {
    pool()
        .await
        .map(|pool| PgPlanFlavorCatalog::new(pool, &registry()))
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
    let Some(pool) = pool().await else {
        eprintln!("PostgreSQL unreachable in this environment");
        return;
    };
    let catalog = PgPlanFlavorCatalog::new(pool.clone(), &registry());
    let record = oracle::pair(0x40, 0, "v1");
    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::Inserted)
    );

    sqlx::query(
        "UPDATE port_executable_plan_revisions SET record_bytes = $1 \
         WHERE executable_plan_id = $2",
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
/// refused by the schema, so the catalog never has to decode one.
#[tokio::test]
async fn the_schema_refuses_a_record_format_the_catalog_cannot_read() {
    let Some(pool) = pool().await else {
        eprintln!("PostgreSQL unreachable in this environment");
        return;
    };
    let catalog = PgPlanFlavorCatalog::new(pool.clone(), &registry());
    let record = oracle::pair(0x41, 0, "v1");
    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::Inserted)
    );

    let rejected = sqlx::query(
        "UPDATE port_worker_flavor_revisions SET record_format = 'v2_cbor' \
         WHERE worker_flavor_id = $1",
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
