//! Same-microsecond tiebreak of the history keyset on the SQL backends.
//!
//! Rows created one after another almost never share a creation microsecond,
//! so the conformance suite cannot reach the `id` half of the keyset. Here
//! every row shares one `created_at_us`, and the ids differ only where byte
//! order and a locale collation disagree (`A` < `Z` < `_` < `a` by bytes), so
//! paging one row at a time proves the backend orders and resumes by bytes —
//! the order the reference adapter uses.

#![cfg(any(feature = "sqlite", feature = "postgres"))]

use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{ExecutionHistoryPageSize, ExecutionHistoryQuery, Scope};

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

const IDS: [&str; 5] = ["exe_A", "exe_Z", "exe__", "exe_a", "exe_b"];
const CREATED_AT_US: i64 = 1_759_665_600_123_456;

fn scope() -> Scope {
    Scope::new("ws_keyset", "org_keyset")
}

/// Page one row at a time and return the ids in the order served.
async fn page_one_by_one(store: &dyn ExecutionStore) -> Vec<String> {
    let one = ExecutionHistoryPageSize::new(1).expect("page size");
    let mut query = ExecutionHistoryQuery::new().with_page_size(one);
    let mut served = Vec::new();
    loop {
        let page = store.list_history(&scope(), &query).await.expect("page");
        served.extend(page.items.into_iter().map(|item| item.id));
        let Some(cursor) = page.next_cursor else {
            break;
        };
        query = ExecutionHistoryQuery::new()
            .with_page_size(one)
            .with_cursor(cursor);
    }
    served
}

fn byte_descending() -> Vec<String> {
    let mut ids: Vec<String> = IDS.iter().map(|id| (*id).to_owned()).collect();
    ids.sort_unstable_by(|a, b| b.as_bytes().cmp(a.as_bytes()));
    ids
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_ties_page_in_byte_order() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("sqlite");
    nebula_storage::sqlite::init_schema(&pool)
        .await
        .expect("schema");
    for id in IDS {
        sqlx::query(
            "INSERT INTO port_executions (id, workspace_id, org_id, workflow_id, status, state, \
             version, fencing_generation, created_at, updated_at, created_at_us) \
             VALUES (?, ?, ?, 'wf', 'created', '{}', 0, 0, \
                     '2025-10-05T12:00:00.123456+00:00', '2025-10-05T12:00:00.123456+00:00', ?)",
        )
        .bind(id)
        .bind(&scope().workspace_id)
        .bind(&scope().org_id)
        .bind(CREATED_AT_US)
        .execute(&pool)
        .await
        .expect("seed");
    }
    let store = nebula_storage::sqlite::SqliteExecutionStore::new(pool);
    assert_eq!(page_one_by_one(&store).await, byte_descending());
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_ties_page_in_byte_order() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
            std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
            "NEBULA_REQUIRE_POSTGRES is set but DATABASE_URL is not"
        );
        eprintln!("WARN [execution_history_keyset] DATABASE_URL unset; Postgres case skipped");
        return;
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "nebula_history_keyset")
        .await
        .expect("postgres");
    nebula_storage::postgres::init_schema(&pool)
        .await
        .expect("schema");
    for id in IDS {
        sqlx::query(
            "INSERT INTO port_executions (id, workspace_id, org_id, workflow_id, status, state, \
             version, fencing_generation, created_at, updated_at, created_at_us) \
             VALUES ($1, $2, $3, 'wf', 'created', '{}', 0, 0, \
                     '2025-10-05T12:00:00.123456Z', '2025-10-05T12:00:00.123456Z', $4)",
        )
        .bind(id)
        .bind(&scope().workspace_id)
        .bind(&scope().org_id)
        .bind(CREATED_AT_US)
        .execute(&pool)
        .await
        .expect("seed");
    }
    let store = nebula_storage::postgres::PgExecutionStore::new(pool);
    assert_eq!(page_one_by_one(&store).await, byte_descending());
}
