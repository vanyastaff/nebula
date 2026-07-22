//! Shared conformance suite for [`CredentialPersistence`] implementations.
//!
//! Every test in `run_conformance` runs against each durable backend
//! (`SqliteCredentialPersistence`, and Postgres when `DATABASE_URL` is set) to
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
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
    CredentialWriteMode, StoredCredential,
};

#[cfg(feature = "postgres")]
use nebula_storage::credential::PgCredentialPersistence;
#[cfg(feature = "sqlite")]
use {
    nebula_storage::credential::SqliteCredentialPersistence,
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

struct OwnerView<'a, S: ?Sized> {
    store: &'a S,
    owner: CredentialOwner,
}

impl<'a, S> OwnerView<'a, S>
where
    S: CredentialPersistence + ?Sized,
{
    fn new(store: &'a S, owner: &str) -> Self {
        Self {
            store,
            owner: CredentialOwner::from_canonical(owner),
        }
    }

    fn selector(&self, id: &str) -> CredentialSelector {
        CredentialSelector::new(self.owner.clone(), id)
    }

    async fn put(
        &self,
        credential: StoredCredential,
        mode: CredentialWriteMode,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let selector = self.selector(&credential.id);
        self.store.put(&selector, credential, mode).await
    }

    async fn get(&self, id: &str) -> Result<StoredCredential, CredentialPersistenceError> {
        self.store.get(&self.selector(id)).await
    }

    async fn delete(&self, id: &str) -> Result<(), CredentialPersistenceError> {
        self.store.delete(&self.selector(id)).await
    }

    async fn list(
        &self,
        state_kind: Option<&str>,
    ) -> Result<Vec<String>, CredentialPersistenceError> {
        self.store.list(&self.owner, state_kind).await
    }

    async fn exists(&self, id: &str) -> Result<bool, CredentialPersistenceError> {
        self.store.exists(&self.selector(id)).await
    }
}

/// Run the full conformance suite against any [`CredentialPersistence`] impl.
///
/// All assertions must hold identically for every backend
/// (`SqliteCredentialPersistence`, `PgCredentialPersistence`). Adding a new case here
/// automatically covers all of them.
async fn run_conformance<S: CredentialPersistence>(store: S) {
    let scoped = OwnerView::new(&store, "owner-default");
    // ── 1. CRUD roundtrip ────────────────────────────────────────────────────
    let cred = make_credential("c1", b"hello");
    let stored = scoped
        .put(cred, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();
    assert_eq!(stored.version, 1, "CreateOnly sets version = 1");
    assert_eq!(stored.data, b"hello", "data roundtrip");

    let fetched = scoped.get("c1").await.unwrap();
    assert_eq!(fetched.id, "c1");
    assert_eq!(fetched.data, b"hello");
    assert_eq!(fetched.version, 1);

    // ── 2. exists ────────────────────────────────────────────────────────────
    assert!(scoped.exists("c1").await.unwrap());
    assert!(!scoped.exists("no-such").await.unwrap());

    // ── 3. delete ────────────────────────────────────────────────────────────
    scoped.delete("c1").await.unwrap();
    assert!(!scoped.exists("c1").await.unwrap());

    // ── 4. delete → NotFound ─────────────────────────────────────────────────
    let err = scoped.delete("no-such").await.unwrap_err();
    assert!(
        matches!(err, CredentialPersistenceError::NotFound { .. }),
        "delete missing → NotFound: {err}"
    );

    // ── 5. CreateOnly duplicate → AlreadyExists ──────────────────────────────
    let cred = make_credential("dup", b"v1");
    scoped
        .put(cred.clone(), CredentialWriteMode::CreateOnly)
        .await
        .unwrap();
    let err = scoped
        .put(cred, CredentialWriteMode::CreateOnly)
        .await
        .unwrap_err();
    assert!(
        matches!(err, CredentialPersistenceError::AlreadyExists { .. }),
        "CreateOnly dup → AlreadyExists: {err}"
    );

    // ── 6. Overwrite version increment ───────────────────────────────────────
    let first = scoped
        .put(make_credential("ow", b"v1"), CredentialWriteMode::Overwrite)
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    let second = scoped
        .put(make_credential("ow", b"v2"), CredentialWriteMode::Overwrite)
        .await
        .unwrap();
    assert_eq!(second.version, 2, "Overwrite increments version");
    assert_eq!(second.data, b"v2");

    // ── 7. CAS success ───────────────────────────────────────────────────────
    let seed = scoped
        .put(
            make_credential("cas", b"v1"),
            CredentialWriteMode::CreateOnly,
        )
        .await
        .unwrap();
    assert_eq!(seed.version, 1);
    let mut next = seed.clone();
    next.data = b"v2".to_vec().into();
    let updated = scoped
        .put(
            next,
            CredentialWriteMode::CompareAndSwap {
                expected_version: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
    assert_eq!(updated.data, b"v2");

    // ── 8. CAS stale → VersionConflict ───────────────────────────────────────
    let mut stale = seed.clone();
    stale.data = b"v3".to_vec().into();
    let err = scoped
        .put(
            stale,
            CredentialWriteMode::CompareAndSwap {
                expected_version: 1,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, CredentialPersistenceError::VersionConflict { .. }),
        "stale CAS → VersionConflict: {err}"
    );

    // ── 9. CAS on missing → NotFound ─────────────────────────────────────────
    let err = scoped
        .put(
            make_credential("absent", b"x"),
            CredentialWriteMode::CompareAndSwap {
                expected_version: 0,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, CredentialPersistenceError::NotFound { .. }),
        "CAS on missing → NotFound: {err}"
    );

    // ── 10. list filter by state_kind ────────────────────────────────────────
    let mut bearer = make_credential("lk-bearer", b"");
    bearer.state_kind = "bearer".into();
    scoped
        .put(bearer, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();

    let mut apikey = make_credential("lk-apikey", b"");
    apikey.state_kind = "api_key".into();
    scoped
        .put(apikey, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();

    let mut bearers = scoped.list(Some("bearer")).await.unwrap();
    bearers.retain(|id| id == "lk-bearer"); // isolate from earlier creds
    assert_eq!(bearers, vec!["lk-bearer"], "list filters by state_kind");

    let mut apikeys = scoped.list(Some("api_key")).await.unwrap();
    apikeys.retain(|id| id == "lk-apikey");
    assert_eq!(apikeys, vec!["lk-apikey"]);

    let empty = scoped.list(Some("nonexistent")).await.unwrap();
    assert!(empty.is_empty(), "list with no matches returns empty");

    // ── 11. byte-identity round-trip of data (non-UTF8) ──────────────────────
    // Ensures the `data` column is stored and retrieved as a byte-exact BLOB,
    // not coerced through any text encoding.
    let non_utf8: Vec<u8> = vec![0xFF, 0xFE, 0x00, 0x01, 0x80, 0x7F];
    let mut bin_cred = make_credential("binary", &non_utf8);
    bin_cred.data = non_utf8.clone().into();
    let stored_bin = scoped
        .put(bin_cred, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();
    assert_eq!(
        stored_bin.data, non_utf8,
        "non-UTF8 data survives round-trip"
    );
    let fetched_bin = scoped.get("binary").await.unwrap();
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
    let after_put = scoped
        .put(ts_cred, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();
    let after_get = scoped.get("ts-check").await.unwrap();
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
    let named_scope = OwnerView::new(&store, owner_id);
    let mut named_a = make_credential("named-a", b"data-a");
    named_a.name = Some("My Credential".into());
    named_a.metadata.insert(
        "owner_id".into(),
        serde_json::Value::String(owner_id.into()),
    );
    named_scope
        .put(named_a, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();

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
    let dup = named_scope
        .put(named_b, CredentialWriteMode::CreateOnly)
        .await;
    assert!(
        matches!(
            dup,
            Err(CredentialPersistenceError::AlreadyExists { .. }
                | CredentialPersistenceError::Backend(_))
        ),
        "duplicate (owner_id, name) must be rejected by the unique index: {dup:?}"
    );
    let fetched_named = named_scope.get("named-a").await.unwrap();
    assert_eq!(
        fetched_named.name.as_deref(),
        Some("My Credential"),
        "name field persisted and fetched back"
    );

    // ── 14. the same display name is valid in a different owner partition ──
    let foreign = OwnerView::new(&store, "owner-foreign");
    let mut foreign_named = make_credential("named-foreign", b"foreign-data");
    foreign_named.name = Some("My Credential".into());
    let foreign_named = foreign
        .put(foreign_named, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();
    assert_eq!(foreign_named.name.as_deref(), Some("My Credential"));
    assert_eq!(foreign_named.data, b"foreign-data");
    assert_eq!(
        named_scope.get("named-a").await.unwrap().data,
        b"data-a",
        "same-name write in another owner must not replace the original row"
    );

    // ── 15. wrong-owner operations are indistinguishable from missing ───────
    assert!(!foreign.exists("named-a").await.unwrap());
    assert!(matches!(
        foreign.get("named-a").await,
        Err(CredentialPersistenceError::NotFound { .. })
    ));
    assert!(matches!(
        foreign.delete("named-a").await,
        Err(CredentialPersistenceError::NotFound { .. })
    ));
    assert!(
        !foreign
            .list(None)
            .await
            .unwrap()
            .contains(&"named-a".to_owned())
    );
    assert_eq!(
        named_scope.get("named-a").await.unwrap().data,
        b"data-a",
        "a wrong-owner delete must leave the victim row intact"
    );

    let victim = named_scope
        .put(
            make_credential("owner-isolation", b"victim"),
            CredentialWriteMode::CreateOnly,
        )
        .await
        .unwrap();
    assert_eq!(victim.version, 1);

    for mode in [
        CredentialWriteMode::CreateOnly,
        CredentialWriteMode::Overwrite,
        CredentialWriteMode::CompareAndSwap {
            expected_version: 1,
        },
    ] {
        let err = foreign
            .put(make_credential("owner-isolation", b"attacker"), mode)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CredentialPersistenceError::NotFound { .. }),
            "wrong-owner {mode:?} must look missing: {err:?}"
        );

        let survivor = named_scope.get("owner-isolation").await.unwrap();
        assert_eq!(survivor.data, b"victim");
        assert_eq!(
            survivor.version, 1,
            "wrong-owner {mode:?} must not mutate the victim version"
        );
    }

    let err = foreign.delete("owner-isolation").await.unwrap_err();
    assert!(
        matches!(err, CredentialPersistenceError::NotFound { .. }),
        "wrong-owner delete must look missing: {err:?}"
    );
    let survivor = named_scope.get("owner-isolation").await.unwrap();
    assert_eq!(survivor.data, b"victim");
    assert_eq!(survivor.version, 1);

    // ── 16. selector and row ids must agree, with no partial write ───────────
    let mismatch_selector = scoped.selector("selector-id");
    let err = store
        .put(
            &mismatch_selector,
            make_credential("row-id", b"must-not-persist"),
            CredentialWriteMode::CreateOnly,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        CredentialPersistenceError::InvalidRequest("selector credential id does not match row id")
    ));
    assert!(!scoped.exists("selector-id").await.unwrap());
    assert!(!scoped.exists("row-id").await.unwrap());

    // ── 17. concurrent CAS has exactly one winner ───────────────────────────
    let race_seed = scoped
        .put(
            make_credential("cas-race", b"seed"),
            CredentialWriteMode::CreateOnly,
        )
        .await
        .unwrap();
    assert_eq!(race_seed.version, 1);

    let mut left = race_seed.clone();
    left.data = b"left".to_vec().into();
    let mut right = race_seed;
    right.data = b"right".to_vec().into();
    let mode = CredentialWriteMode::CompareAndSwap {
        expected_version: 1,
    };
    let (left_result, right_result) = tokio::join!(scoped.put(left, mode), scoped.put(right, mode));

    let mut success_count = 0;
    let mut conflict_count = 0;
    let mut winner_data = None;
    let outcomes: [_; 2] = (left_result, right_result).into();
    for outcome in outcomes {
        match outcome {
            Ok(stored) => {
                success_count += 1;
                assert_eq!(stored.version, 2);
                assert!(stored.data == b"left" || stored.data == b"right");
                winner_data = Some(stored.data.as_ref().to_vec());
            },
            Err(CredentialPersistenceError::VersionConflict {
                expected, actual, ..
            }) => {
                conflict_count += 1;
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            },
            Err(other) => panic!("concurrent CAS returned an unexpected error: {other:?}"),
        }
    }
    assert_eq!(success_count, 1, "concurrent CAS must have one winner");
    assert_eq!(conflict_count, 1, "concurrent CAS must have one conflict");

    let winner_data = winner_data.expect("one successful CAS records winner data");
    let final_row = scoped.get("cas-race").await.unwrap();
    assert_eq!(final_row.version, 2);
    assert_eq!(final_row.data.as_ref(), winner_data.as_slice());

    // Caller metadata cannot redirect a write: the adapter derives and stamps
    // owner identity exclusively from the selector.
    let mut spoofed = make_credential("owner-stamp", b"x");
    spoofed.metadata.insert(
        "owner_id".to_owned(),
        serde_json::Value::String("attacker-chosen".to_owned()),
    );
    let persisted = scoped
        .put(spoofed, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();
    assert_eq!(
        persisted
            .metadata
            .get("owner_id")
            .and_then(|value| value.as_str()),
        Some("owner-default")
    );
}

// ── test entry points ─────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn conformance_sqlite() {
    let pool = sqlite_pool().await;
    run_conformance(SqliteCredentialPersistence::new(pool)).await;
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
    run_conformance(PgCredentialPersistence::new(pool)).await;
}

// ── SQLite-specific: name uniqueness is enforced at the index level ───────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_name_uniqueness_enforced_by_index() {
    let pool = sqlite_pool().await;
    let store = SqliteCredentialPersistence::new(pool);
    let owner_id = "owner-unique-test";
    let scoped = OwnerView::new(&store, owner_id);

    let mut a = make_credential("unique-a", b"a");
    a.name = Some("Shared Name".into());
    a.metadata.insert(
        "owner_id".into(),
        serde_json::Value::String(owner_id.into()),
    );
    scoped
        .put(a, CredentialWriteMode::CreateOnly)
        .await
        .unwrap();

    let mut b = make_credential("unique-b", b"b");
    b.name = Some("Shared Name".into());
    b.metadata.insert(
        "owner_id".into(),
        serde_json::Value::String(owner_id.into()),
    );
    let err = scoped
        .put(b, CredentialWriteMode::CreateOnly)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CredentialPersistenceError::AlreadyExists { .. }
                | CredentialPersistenceError::Backend(_)
        ),
        "duplicate (owner_id, name) must fail at the unique index: {err}"
    );
}

// ── SQLite-specific: get on missing → NotFound ────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_get_not_found() {
    let pool = sqlite_pool().await;
    let store = SqliteCredentialPersistence::new(pool);
    let scoped = OwnerView::new(&store, "owner-missing-test");
    let err = scoped.get("does-not-exist").await.unwrap_err();
    assert!(matches!(err, CredentialPersistenceError::NotFound { .. }));
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

    let first = SqliteCredentialPersistence::connect(&url).await.unwrap();
    let first_scoped = OwnerView::new(&first, "owner-restart-test");
    first_scoped
        .put(
            make_credential("survivor", b"secret"),
            CredentialWriteMode::CreateOnly,
        )
        .await
        .unwrap();

    let second = SqliteCredentialPersistence::connect(&url).await.unwrap();
    let second_scoped = OwnerView::new(&second, "owner-restart-test");
    let got = second_scoped
        .get("survivor")
        .await
        .expect("credential survives a reconnect — connect() must not re-run the DROP");
    assert_eq!(got.data, b"secret");

    drop(first);
    drop(second);
}
