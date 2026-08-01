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
    PlanFlavorCatalog, PlanFlavorCatalogAdmin, PlanFlavorCatalogWriter, PlanFlavorRevisionTarget,
    RevisionCatalogError, RevisionInsertOutcome,
};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// A private metrics registry for this backend under test.
///
/// Production wiring threads the process-shared registry so a scraper observes
/// the catalog's conflict and unknown-outcome counts; a conformance run only
/// needs the counters to be bindable, and a private registry keeps concurrent
/// cases from sharing series.
fn registry() -> nebula_metrics::MetricsRegistry {
    nebula_metrics::MetricsRegistry::new()
}

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
    Some(SqlitePlanFlavorCatalog::new(
        fresh_pool().await,
        &registry(),
    ))
}

/// Read back one catalog outcome series.
///
/// The registry hands out the same counter for the same name and label set, so
/// rebuilding the labels reads exactly the series the adapter incremented.
fn counted(metrics: &nebula_metrics::MetricsRegistry, operation: &str, outcome: &str) -> u64 {
    let labels = metrics.interner().label_set(&[
        ("backend", "sqlite"),
        ("operation", operation),
        ("outcome", outcome),
    ]);
    metrics
        .counter_labeled(
            nebula_metrics::NEBULA_STORAGE_REVISION_CATALOG_OPERATIONS_TOTAL,
            &labels,
        )
        .expect("the catalog counter registers")
        .get()
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
    let catalog = SqlitePlanFlavorCatalog::new(pool.clone(), &registry());
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
/// refused by the schema, so the catalog never has to decode one.
///
/// The adapter's own `UnsupportedRecordFormat` guard is covered by the
/// `decode_worker_flavor_row` unit tests in `crate::revision_catalog`, which
/// can hand the decoder a row this `CHECK` makes unreachable through SQL.
#[tokio::test]
async fn the_schema_refuses_a_record_format_the_catalog_cannot_read() {
    let pool = fresh_pool().await;
    let catalog = SqlitePlanFlavorCatalog::new(pool.clone(), &registry());
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

/// Catalog outcomes reach a scraper, not only a trace sampler.
///
/// `content_conflict` is the case that motivates the counter: an immutable
/// identity reused for different bytes is invisible in a success rate, and a
/// deployment backend whose only record of it is a sampled span gives an
/// operator nothing to alert on.
#[tokio::test]
async fn catalog_outcomes_are_counted_per_operation_and_outcome() {
    let metrics = registry();
    let catalog = SqlitePlanFlavorCatalog::new(fresh_pool().await, &metrics);
    let record = oracle::pair(0x42, 0, "v1");

    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::Inserted)
    );
    assert_eq!(counted(&metrics, "insert", "inserted"), 1);

    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::AlreadyPresent)
    );
    assert_eq!(counted(&metrics, "insert", "already_present"), 1);

    let conflicting = nebula_storage_port::PlanFlavorRevisionRecord::graph_v1_json(
        record.ids().plan(),
        nebula_storage_port::RevisionRecordBytes::try_from_vec(br#"{"plan":"rewritten"}"#.to_vec())
            .expect("conflict body is non-empty"),
        record.worker_flavor().clone(),
    );
    assert!(catalog.insert(&conflicting).await.is_err());
    assert_eq!(
        counted(&metrics, "insert", "content_conflict"),
        1,
        "an immutable identity reused for different bytes is alertable on its own"
    );

    assert!(catalog.load_exact(record.ids()).await.is_ok());
    assert_eq!(counted(&metrics, "load_exact", "loaded"), 1);

    let target = PlanFlavorRevisionTarget::ExecutablePlan(record.ids().plan());
    assert!(catalog.begin_drain(target).await.is_ok());
    assert_eq!(counted(&metrics, "begin_drain", "started"), 1);

    assert_eq!(catalog.delete_drained(target).await, Ok(()));
    assert_eq!(counted(&metrics, "delete_drained", "deleted"), 1);

    assert_eq!(
        counted(&metrics, "load_exact", "outcome_unknown"),
        0,
        "an outcome that did not happen must not be counted"
    );
}
