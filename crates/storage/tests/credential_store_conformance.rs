//! Shared conformance suite for [`CredentialStore`] implementations.
//!
//! Every test in `run_conformance` runs against each durable backend
//! (`SqliteCredentialStore`, and Postgres when `DATABASE_URL` is set) to
//! guarantee behavioural parity. The SQLite store is backed by a named
//! in-memory database
//! (`sqlite:file:<unique>?mode=memory&cache=shared`) with `max_connections = 2`
//! so the pool exercises real SQL-layer concurrency without touching disk.
//!
//! Migration 0030 is applied inline before the store is constructed, matching
//! the pattern used by `tests/refresh_claim_sqlite_integration.rs`.

#![cfg(any(feature = "sqlite", feature = "postgres"))]

mod common;

use common::make_credential;
use nebula_credential::{CredentialStore, PutMode, StoreError};

#[cfg(feature = "postgres")]
use nebula_storage::credential::PgCredentialStore;
#[cfg(feature = "sqlite")]
use {
    nebula_storage::credential::SqliteCredentialStore,
    sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    std::str::FromStr,
};

// ── SQLite pool helper ────────────────────────────────────────────────────────

/// Build a migrated SQLite pool backed by a unique named in-memory database.
///
/// `mode=memory&cache=shared` is mandatory: plain `sqlite::memory:` gives each
/// connection a separate, invisible database. The random name keeps concurrent
/// test runs in the same process isolated from each other.
#[cfg(feature = "sqlite")]
async fn sqlite_pool() -> sqlx::SqlitePool {
    let db_name = format!("nebula-cred-conformance-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{db_name}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("static URL template is always valid")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("in-memory SQLite always connects");

    sqlx::query(include_str!(
        "../migrations/sqlite/0030_credentials_store.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration 0030 applied to fresh DB");

    pool
}

// ── shared conformance body ───────────────────────────────────────────────────

/// Run the full conformance suite against any [`CredentialStore`] impl.
///
/// All assertions must hold identically for every backend
/// (`SqliteCredentialStore`, `PgCredentialStore`). Adding a new case here
/// automatically covers all of them.
async fn run_conformance<S: CredentialStore>(store: S) {
    // ── 1. CRUD roundtrip ────────────────────────────────────────────────────
    let cred = make_credential("c1", b"hello");
    let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
    assert_eq!(stored.version, 1, "CreateOnly sets version = 1");
    assert_eq!(stored.data, b"hello", "data roundtrip");

    let fetched = store.get("c1").await.unwrap();
    assert_eq!(fetched.id, "c1");
    assert_eq!(fetched.data, b"hello");
    assert_eq!(fetched.version, 1);

    // ── 2. exists ────────────────────────────────────────────────────────────
    assert!(store.exists("c1").await.unwrap());
    assert!(!store.exists("no-such").await.unwrap());

    // ── 3. delete ────────────────────────────────────────────────────────────
    store.delete("c1").await.unwrap();
    assert!(!store.exists("c1").await.unwrap());

    // ── 4. delete → NotFound ─────────────────────────────────────────────────
    let err = store.delete("no-such").await.unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound { .. }),
        "delete missing → NotFound: {err}"
    );

    // ── 5. CreateOnly duplicate → AlreadyExists ──────────────────────────────
    let cred = make_credential("dup", b"v1");
    store.put(cred.clone(), PutMode::CreateOnly).await.unwrap();
    let err = store.put(cred, PutMode::CreateOnly).await.unwrap_err();
    assert!(
        matches!(err, StoreError::AlreadyExists { .. }),
        "CreateOnly dup → AlreadyExists: {err}"
    );

    // ── 6. Overwrite version increment ───────────────────────────────────────
    let first = store
        .put(make_credential("ow", b"v1"), PutMode::Overwrite)
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    let second = store
        .put(make_credential("ow", b"v2"), PutMode::Overwrite)
        .await
        .unwrap();
    assert_eq!(second.version, 2, "Overwrite increments version");
    assert_eq!(second.data, b"v2");

    // ── 7. CAS success ───────────────────────────────────────────────────────
    let seed = store
        .put(make_credential("cas", b"v1"), PutMode::CreateOnly)
        .await
        .unwrap();
    assert_eq!(seed.version, 1);
    let mut next = seed.clone();
    next.data = b"v2".to_vec();
    let updated = store
        .put(
            next,
            PutMode::CompareAndSwap {
                expected_version: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
    assert_eq!(updated.data, b"v2");

    // ── 8. CAS stale → VersionConflict ───────────────────────────────────────
    let mut stale = seed.clone();
    stale.data = b"v3".to_vec();
    let err = store
        .put(
            stale,
            PutMode::CompareAndSwap {
                expected_version: 1,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::VersionConflict { .. }),
        "stale CAS → VersionConflict: {err}"
    );

    // ── 9. CAS on missing → NotFound ─────────────────────────────────────────
    let err = store
        .put(
            make_credential("absent", b"x"),
            PutMode::CompareAndSwap {
                expected_version: 0,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound { .. }),
        "CAS on missing → NotFound: {err}"
    );

    // ── 10. list filter by state_kind ────────────────────────────────────────
    let mut bearer = make_credential("lk-bearer", b"");
    bearer.state_kind = "bearer".into();
    store.put(bearer, PutMode::CreateOnly).await.unwrap();

    let mut apikey = make_credential("lk-apikey", b"");
    apikey.state_kind = "api_key".into();
    store.put(apikey, PutMode::CreateOnly).await.unwrap();

    let mut bearers = store.list(Some("bearer")).await.unwrap();
    bearers.retain(|id| id == "lk-bearer"); // isolate from earlier creds
    assert_eq!(bearers, vec!["lk-bearer"], "list filters by state_kind");

    let mut apikeys = store.list(Some("api_key")).await.unwrap();
    apikeys.retain(|id| id == "lk-apikey");
    assert_eq!(apikeys, vec!["lk-apikey"]);

    let empty = store.list(Some("nonexistent")).await.unwrap();
    assert!(empty.is_empty(), "list with no matches returns empty");

    // ── 11. byte-identity round-trip of data (non-UTF8) ──────────────────────
    // Ensures the `data` column is stored and retrieved as a byte-exact BLOB,
    // not coerced through any text encoding.
    let non_utf8: Vec<u8> = vec![0xFF, 0xFE, 0x00, 0x01, 0x80, 0x7F];
    let mut bin_cred = make_credential("binary", &non_utf8);
    bin_cred.data = non_utf8.clone();
    let stored_bin = store.put(bin_cred, PutMode::CreateOnly).await.unwrap();
    assert_eq!(
        stored_bin.data, non_utf8,
        "non-UTF8 data survives round-trip"
    );
    let fetched_bin = store.get("binary").await.unwrap();
    assert_eq!(
        fetched_bin.data, non_utf8,
        "non-UTF8 data fetched back identically"
    );

    // ── 12. updated_at stable across a put → get round-trip ──────────────────
    // The `StoredCredential` returned by `put` must carry the same
    // store-assigned `updated_at` a subsequent `get` reports. Backends that
    // store at coarser resolution (SQLite persists millis) satisfy this because
    // `put` returns the persisted/read-back row, not the pre-store value — so
    // the resolution of the column is irrelevant to this equality.
    let ts_cred = make_credential("ts-check", b"ts");
    let after_put = store.put(ts_cred, PutMode::CreateOnly).await.unwrap();
    let after_get = store.get("ts-check").await.unwrap();
    assert_eq!(
        after_put.updated_at, after_get.updated_at,
        "updated_at is stable across put → get round-trip (millis normalisation)"
    );

    // ── 13. name uniqueness per owner ────────────────────────────────────────
    // Two credentials with the same owner_id + name must not coexist. Every
    // durable backend enforces this at the store level (SQLite via the partial
    // unique index, Postgres via its mirror), so the second insert MUST be
    // rejected — asserted strictly below for every backend.
    let owner_id = "owner-xyz";
    let mut named_a = make_credential("named-a", b"data-a");
    named_a.name = Some("My Credential".into());
    named_a.metadata.insert(
        "owner_id".into(),
        serde_json::Value::String(owner_id.into()),
    );
    store.put(named_a, PutMode::CreateOnly).await.unwrap();

    let mut named_b = make_credential("named-b", b"data-b");
    named_b.name = Some("My Credential".into());
    named_b.metadata.insert(
        "owner_id".into(),
        serde_json::Value::String(owner_id.into()),
    );
    // The duplicate (owner_id, name) must be rejected by the unique index. The
    // exact variant differs per driver (SQLite/Postgres surface the unique
    // violation as AlreadyExists or Backend depending on which constraint the
    // discriminator matches), so accept either — what matters is that it fails.
    let dup = store.put(named_b, PutMode::CreateOnly).await;
    assert!(
        matches!(
            dup,
            Err(StoreError::AlreadyExists { .. } | StoreError::Backend(_))
        ),
        "duplicate (owner_id, name) must be rejected by the unique index: {dup:?}"
    );
    let fetched_named = store.get("named-a").await.unwrap();
    assert_eq!(
        fetched_named.name.as_deref(),
        Some("My Credential"),
        "name field persisted and fetched back"
    );
}

// ── test entry points ─────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn conformance_sqlite() {
    let pool = sqlite_pool().await;
    run_conformance(SqliteCredentialStore::new(pool)).await;
}

// ── Postgres backend (DATABASE_URL-gated; skips clean when unset) ──────────────

/// Connect a Postgres pool from `DATABASE_URL` and apply migration 0030.
///
/// Returns `None` (skip) when `DATABASE_URL` is unset; panics when it is set
/// but unusable, so a misconfigured CI surfaces loudly rather than silently
/// skipping — mirrors `tests/pg_idempotency.rs`.
#[cfg(feature = "postgres")]
async fn pg_pool() -> Option<sqlx::PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => return None,
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect Postgres (DATABASE_URL)");
    sqlx::query(include_str!(
        "../migrations/postgres/0030_credentials_store.sql"
    ))
    .execute(&pool)
    .await
    .expect("migration 0030 applied to the Postgres database");
    Some(pool)
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn conformance_postgres() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Postgres conformance skipped — DATABASE_URL unset");
        return;
    };
    run_conformance(PgCredentialStore::new(pool)).await;
}

// ── SQLite-specific: name uniqueness is enforced at the index level ───────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_name_uniqueness_enforced_by_index() {
    let pool = sqlite_pool().await;
    let store = SqliteCredentialStore::new(pool);
    let owner_id = "owner-unique-test";

    let mut a = make_credential("unique-a", b"a");
    a.name = Some("Shared Name".into());
    a.metadata.insert(
        "owner_id".into(),
        serde_json::Value::String(owner_id.into()),
    );
    store.put(a, PutMode::CreateOnly).await.unwrap();

    let mut b = make_credential("unique-b", b"b");
    b.name = Some("Shared Name".into());
    b.metadata.insert(
        "owner_id".into(),
        serde_json::Value::String(owner_id.into()),
    );
    let err = store.put(b, PutMode::CreateOnly).await.unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::AlreadyExists { .. } | StoreError::Backend(_)
        ),
        "duplicate (owner_id, name) must fail at the unique index: {err}"
    );
}

// ── SQLite-specific: get on missing → NotFound ────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_get_not_found() {
    let pool = sqlite_pool().await;
    let store = SqliteCredentialStore::new(pool);
    let err = store.get("does-not-exist").await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound { .. }));
}

// ── SQLite-specific: connect() is idempotent and preserves existing rows ──────
// Regression guard for the durability bug — migration 0030 begins with
// `DROP TABLE credentials`, so a `connect()` that re-ran it on every call would
// wipe the store on each restart. A first `connect()` provisions the table and
// writes a row; a second `connect()` to the same database must NOT drop it. The
// first handle is held open so the shared-cache in-memory DB survives, standing
// in for a persistent file across a process restart.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_connect_is_idempotent_and_preserves_existing_rows() {
    let name = format!("nebula-cred-restart-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{name}?mode=memory&cache=shared");

    let first = SqliteCredentialStore::connect(&url).await.unwrap();
    first
        .put(make_credential("survivor", b"secret"), PutMode::CreateOnly)
        .await
        .unwrap();

    let second = SqliteCredentialStore::connect(&url).await.unwrap();
    let got = second
        .get("survivor")
        .await
        .expect("credential survives a reconnect — connect() must not re-run the DROP");
    assert_eq!(got.data, b"secret");

    drop(first);
    drop(second);
}
