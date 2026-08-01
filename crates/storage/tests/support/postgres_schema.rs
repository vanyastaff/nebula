//! Private-schema isolation for `DATABASE_URL`-backed test backends.
//!
//! The InMemory and SQLite `:memory:` conformance backends hand every case a
//! fresh empty store, and the shared assertions depend on it: they use fixed
//! fixture ids (`wf_c`, `exe_cq`, `usr_a`, …) that several cases reuse. A
//! Postgres backend that connects straight to `DATABASE_URL` breaks that
//! contract — all cases land in `public`, the ids collide, and the second case
//! to touch one fails with a `Duplicate` error or a rejected schema catalog.
//!
//! Because the Postgres arm of those suites skip-cleans without `DATABASE_URL`,
//! the collisions were invisible: the "shared oracle" was green only because
//! its Postgres arm never ran. Giving each backend instance its own schema
//! restores the same-fresh-store contract the other two backends satisfy.
//!
//! The migration catalog observes and installs through `current_schema()`, so
//! a pool pinned to a private schema sees a genuinely fresh database.
//!
//! Included by `#[path]` from every `DATABASE_URL`-backed integration-test
//! binary that needs it. `nebula_storage::test_support` is `#[cfg(test)]`-only
//! and therefore invisible to integration tests, which compile as their own
//! crates.

use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::postgres::{PgPool, PgPoolOptions};

/// Maximum pooled connections per isolated test pool. Matches what the
/// conformance backends used before isolation, so concurrency behaviour
/// (lease contention, `SKIP LOCKED` claims) is unchanged.
const MAX_CONNECTIONS: u32 = 8;

/// Names the private schema for one test backend instance.
///
/// Only `[a-z0-9_]` by construction, so interpolating it into DDL cannot
/// inject SQL — the identifier is never operator- or fixture-supplied. The pid
/// separates concurrent test *processes* (nextest's default execution model)
/// and the counter separates instances within one process (`--test-threads N`).
#[must_use]
pub(crate) fn unique_schema_name(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Connect to `url` with every connection pinned to a freshly created private
/// schema named `<prefix>_<pid>_<n>`.
///
/// The schema is dropped before it is created: a pid repeats across runs
/// against a long-lived local database, and a stale schema would otherwise
/// leave the case running against the previous run's rows.
///
/// The caller still installs its own schema (`postgres::init_schema`) into the
/// returned pool.
///
/// # Errors
/// Returns the underlying [`sqlx::Error`] if the database is unreachable or
/// refuses the schema DDL.
pub(crate) async fn connect_with_private_schema(
    url: &str,
    prefix: &str,
) -> Result<PgPool, sqlx::Error> {
    let schema = unique_schema_name(prefix);

    let admin = PgPoolOptions::new().max_connections(1).connect(url).await?;
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&admin)
            .await?;
    }
    admin.close().await;

    let search_path = format!("SET search_path TO {schema}");
    PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .after_connect(move |connection, _meta| {
            let search_path = search_path.clone();
            Box::pin(async move {
                sqlx::query(sqlx::AssertSqlSafe(search_path))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
}

mod tests {
    use super::unique_schema_name;

    #[test]
    fn schema_names_are_injection_safe_and_distinct() {
        let first = unique_schema_name("nebula_case");
        let second = unique_schema_name("nebula_case");

        assert_ne!(
            first, second,
            "two instances in one process must not share a schema"
        );
        for name in [&first, &second] {
            assert!(
                name.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
                "schema identifier is interpolated into DDL unquoted: {name}"
            );
        }
    }
}
