//! PostgreSQL acceptance coverage for the structural credential lifecycle.
//!
//! Every test owns an isolated schema. An absent `DATABASE_URL` skips cleanly;
//! a configured but unusable database remains a hard failure.

#![cfg(feature = "postgres")]

use std::{
    error::Error,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use nebula_core::CredentialId;
use nebula_storage::credential::PgCredentialPersistence;
use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCreate, CredentialOwner, CredentialPersistence,
    CredentialPersistenceError, CredentialRecordState, CredentialReplacement, CredentialSelector,
    CredentialTombstone, CredentialVersion, SecretBytes, StoredCredential,
};
use serde_json::{Map, Value};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::sync::Barrier;

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

struct IsolatedDatabase {
    admin: PgPool,
    pool: PgPool,
    options: PgConnectOptions,
    schema: String,
}

impl IsolatedDatabase {
    async fn connect() -> Option<Self> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => {
                assert!(
                    std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                    "NEBULA_REQUIRE_POSTGRES=1 but DATABASE_URL is absent"
                );
                return None;
            },
            Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
        };

        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect to DATABASE_URL");
        let schema = unique_schema_name();
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create isolated credential lifecycle schema");

        let options = PgConnectOptions::from_str(&url)
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .expect("connect to isolated credential lifecycle schema");

        Some(Self {
            admin,
            pool,
            options,
            schema,
        })
    }

    async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("drop isolated credential lifecycle schema");
        self.admin.close().await;
    }
}

fn unique_schema_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("nebula_credential_lifecycle_{}_{nanos}", std::process::id())
}

fn owner(value: &str) -> CredentialOwner {
    CredentialOwner::from_canonical(value)
}

fn selector(owner: &CredentialOwner, credential_id: CredentialId) -> CredentialSelector {
    CredentialSelector::new(owner.clone(), credential_id)
}

fn metadata(name: Option<&str>) -> Map<String, Value> {
    let mut metadata =
        Map::from_iter([("opaque".to_owned(), serde_json::json!({"preserved": true}))]);
    if let Some(name) = name {
        metadata.insert(
            "display".to_owned(),
            serde_json::json!({"display_name": name}),
        );
    }
    metadata
}

fn create(name: Option<&str>, secret: &[u8]) -> CredentialCreate {
    CredentialCreate::new(
        "provider.api-token".to_owned(),
        SecretBytes::new(secret.to_vec()),
        "active".to_owned(),
        7,
        name.map(str::to_owned),
        None,
        false,
        metadata(name),
    )
}

fn replacement(
    expected_version: CredentialVersion,
    name: Option<&str>,
    secret: &[u8],
) -> CredentialReplacement {
    CredentialReplacement::new(
        expected_version,
        SecretBytes::new(secret.to_vec()),
        "active".to_owned(),
        8,
        name.map(str::to_owned),
        None,
        true,
        metadata(name),
        nebula_storage_port::CredentialMaterialTransition::advance(),
    )
}

fn version(value: i64) -> CredentialVersion {
    CredentialVersion::try_from(value).expect("fixture version is valid")
}

#[tokio::test]
async fn postgres_lifecycle_enforces_precedence_cas_and_terminal_visibility() -> TestResult<()> {
    let Some(database) = IsolatedDatabase::connect().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return Ok(());
    };
    let store = PgCredentialPersistence::connect_with(database.options.clone()).await?;
    let owner_a = owner("tenant-a");
    let owner_b = owner("tenant-b");

    let named_id = CredentialId::new();
    let named_selector = selector(&owner_a, named_id);
    let created = store
        .create(&named_selector, create(Some("Production"), b"secret-v1"))
        .await?;
    assert_eq!(created.credential_id(), named_id);
    assert_eq!(created.version(), version(1));
    assert_eq!(created.state(), CredentialRecordState::Live);

    let mismatched_id = CredentialId::new();
    let mismatched_selector = selector(&owner_a, mismatched_id);
    let mismatched_create = CredentialCreate::new(
        "provider.api-token".to_owned(),
        SecretBytes::new(b"must-not-write".to_vec()),
        "active".to_owned(),
        7,
        Some("Physical".to_owned()),
        None,
        false,
        metadata(Some("Projected")),
    );
    assert_eq!(
        store
            .create(&mismatched_selector, mismatched_create)
            .await
            .expect_err("physical and metadata names must be one authority"),
        CredentialPersistenceError::CorruptRecord
    );
    assert!(!store.exists(&mismatched_selector).await?);

    let malformed_replacement = CredentialReplacement::new(
        version(1),
        SecretBytes::new(b"must-not-write".to_vec()),
        "active".to_owned(),
        8,
        Some("Production".to_owned()),
        None,
        true,
        Map::from_iter([("display".to_owned(), Value::Bool(true))]),
        nebula_storage_port::CredentialMaterialTransition::advance(),
    );
    assert_eq!(
        store
            .replace(&named_selector, malformed_replacement)
            .await
            .expect_err("malformed display metadata must fail before DML"),
        CredentialPersistenceError::CorruptRecord
    );
    assert_eq!(
        store.get(&named_selector).await?.version(),
        version(1),
        "invalid replacement must not advance the row"
    );

    let corrupt_owner = owner("tenant-corrupt");
    let corrupt_id = CredentialId::new();
    let corrupt_selector = selector(&corrupt_owner, corrupt_id);
    store
        .create(&corrupt_selector, create(None, b"opaque"))
        .await?;
    sqlx::query(
        "UPDATE credentials
         SET metadata = '{\"display\":\"not-an-object\"}'
         WHERE owner_id = $1 AND id = $2",
    )
    .bind(corrupt_owner.as_str())
    .bind(corrupt_id.to_string())
    .execute(&database.pool)
    .await?;
    assert_eq!(
        store
            .get(&corrupt_selector)
            .await
            .expect_err("persisted malformed metadata must fail closed"),
        CredentialPersistenceError::CorruptRecord
    );
    sqlx::query(
        "UPDATE credentials
         SET metadata = '{}'
         WHERE owner_id = $1 AND id = $2",
    )
    .bind(corrupt_owner.as_str())
    .bind(corrupt_id.to_string())
    .execute(&database.pool)
    .await?;

    let duplicate_id = store
        .create(&named_selector, create(Some("Production"), b"duplicate"))
        .await
        .expect_err("same-owner id collision must be classified");
    assert_eq!(
        duplicate_id,
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );

    let foreign_selector = selector(&owner_b, named_id);
    let foreign_collision = store
        .create(
            &foreign_selector,
            create(Some("Foreign Production"), b"foreign"),
        )
        .await
        .expect_err("a globally occupied foreign id must not disclose its owner");
    assert_eq!(foreign_collision, CredentialPersistenceError::NotFound);

    let duplicate_name_selector = selector(&owner_a, CredentialId::new());
    let duplicate_name = store
        .create(
            &duplicate_name_selector,
            create(Some("Production"), b"other-id"),
        )
        .await
        .expect_err("same-owner live names are unique");
    assert_eq!(
        duplicate_name,
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );

    let replaced = store
        .replace(
            &named_selector,
            replacement(version(1), Some("Production"), b"secret-v2"),
        )
        .await?;
    assert_eq!(replaced.version(), version(2));
    assert_eq!(replaced.state(), CredentialRecordState::Live);

    let stale = store
        .replace(
            &named_selector,
            replacement(version(1), Some("Production"), b"stale"),
        )
        .await
        .expect_err("stale replace must expose the actual structural version");
    assert_eq!(
        stale,
        CredentialPersistenceError::VersionConflict {
            expected: version(1),
            actual: version(2),
        }
    );

    let secondary_id = CredentialId::new();
    let secondary_selector = selector(&owner_a, secondary_id);
    store
        .create(&secondary_selector, create(Some("Secondary"), b"secondary"))
        .await?;
    let rename_collision = store
        .replace(
            &named_selector,
            replacement(version(2), Some("Secondary"), b"must-rollback"),
        )
        .await
        .expect_err("rename must preserve owner-local live-name uniqueness");
    assert_eq!(
        rename_collision,
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );
    assert_eq!(
        store.get(&named_selector).await?.version(),
        version(2),
        "failed rename must not advance the persistence version"
    );

    let tombstoned = store
        .tombstone(&named_selector, CredentialTombstone::new(version(2)))
        .await?;
    assert_eq!(tombstoned.version(), version(3));
    assert_eq!(tombstoned.state(), CredentialRecordState::Tombstoned);
    assert!(tombstoned.tombstoned_at().is_some());

    let physical = store.get(&named_selector).await?;
    match physical {
        StoredCredential::Tombstoned(row) => {
            assert_eq!(row.credential_id(), named_id);
            assert_eq!(row.version(), version(3));
        },
        StoredCredential::Live(_) => panic!("physical get must retain the terminal record"),
    }
    assert_eq!(
        store
            .get_head(&named_selector)
            .await
            .expect_err("management head reads are live-only"),
        CredentialPersistenceError::NotFound
    );
    assert!(!store.exists(&named_selector).await?);
    assert!(!store.list(&owner_a, None).await?.contains(&named_id));
    assert!(
        store
            .list_heads(&owner_a, None)
            .await?
            .iter()
            .all(|head| head.credential_id() != named_id)
    );

    let terminal_replace = store
        .replace(
            &named_selector,
            replacement(version(1), Some("Production"), b"must-not-return"),
        )
        .await
        .expect_err("terminal state has NotFound precedence over stale CAS");
    assert_eq!(terminal_replace, CredentialPersistenceError::NotFound);
    let terminal_tombstone = store
        .tombstone(&named_selector, CredentialTombstone::new(version(i64::MAX)))
        .await
        .expect_err("terminal state has NotFound precedence over exhaustion");
    assert_eq!(terminal_tombstone, CredentialPersistenceError::NotFound);
    let terminal_create = store
        .create(
            &named_selector,
            create(Some("Production"), b"must-not-resurrect"),
        )
        .await
        .expect_err("a tombstone permanently reserves its global id");
    assert_eq!(
        terminal_create,
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );

    let reused_name_id = CredentialId::new();
    let reused_name_selector = selector(&owner_a, reused_name_id);
    let reused_name = store
        .create(
            &reused_name_selector,
            create(Some("Production"), b"replacement-name-owner"),
        )
        .await?;
    assert_eq!(reused_name.credential_id(), reused_name_id);
    assert!(
        store.list(&owner_a, None).await?.contains(&reused_name_id),
        "tombstoning releases the owner-local display name"
    );

    let restart_live_id = CredentialId::new();
    let restart_live_selector = selector(&owner_a, restart_live_id);
    store
        .create(
            &restart_live_selector,
            create(Some("Restart Live"), b"restart-v1"),
        )
        .await?;
    let restart_live_commit = store
        .replace(
            &restart_live_selector,
            replacement(version(1), Some("Restart Live"), b"restart-v2"),
        )
        .await?;
    assert_eq!(restart_live_commit.version(), version(2));

    let mut interrupted = database.pool.begin().await?;
    sqlx::query(
        "UPDATE credentials
         SET data = $1, version = version + 1
         WHERE owner_id = $2 AND id = $3",
    )
    .bind(b"uncommitted".as_slice())
    .bind(owner_a.as_str())
    .bind(restart_live_id.to_string())
    .execute(&mut *interrupted)
    .await?;
    drop(interrupted);
    match store.get(&restart_live_selector).await? {
        StoredCredential::Live(row) => {
            assert_eq!(row.version(), version(2));
            assert_eq!(row.data().as_ref(), b"restart-v2");
        },
        StoredCredential::Tombstoned(_) => {
            panic!("an interrupted uncommitted transaction cannot tombstone the prior row")
        },
    }

    let exhausted_id = CredentialId::new();
    let exhausted_selector = selector(&owner_a, exhausted_id);
    store
        .create(&exhausted_selector, create(None, b"last-live"))
        .await?;
    sqlx::query(
        "UPDATE credentials
         SET version = 9223372036854775806
         WHERE owner_id = $1 AND id = $2",
    )
    .bind(owner_a.as_str())
    .bind(exhausted_id.to_string())
    .execute(&database.pool)
    .await?;

    let exhausted_replace = store
        .replace(
            &exhausted_selector,
            replacement(version(i64::MAX - 1), None, b"overflow"),
        )
        .await
        .expect_err("replace must preserve terminal headroom");
    assert_eq!(
        exhausted_replace,
        CredentialPersistenceError::VersionExhausted
    );
    match store.get(&exhausted_selector).await? {
        StoredCredential::Live(row) => {
            assert_eq!(row.version(), version(i64::MAX - 1));
            assert_eq!(row.data().as_ref(), b"last-live");
        },
        StoredCredential::Tombstoned(_) => panic!("failed replace must leave the row live"),
    }

    let terminal = store
        .tombstone(
            &exhausted_selector,
            CredentialTombstone::new(version(i64::MAX - 1)),
        )
        .await?;
    assert_eq!(terminal.version(), version(i64::MAX));
    assert_eq!(terminal.state(), CredentialRecordState::Tombstoned);

    drop(store);
    let restarted = PgCredentialPersistence::connect_with(database.options.clone()).await?;
    assert!(
        matches!(
            restarted.get(&named_selector).await?,
            StoredCredential::Tombstoned(_)
        ),
        "a fresh pool must observe the durable terminal record"
    );
    assert_eq!(
        restarted.get(&reused_name_selector).await?.version(),
        reused_name.version(),
        "a fresh pool must preserve the exact committed live version"
    );
    match restarted.get(&restart_live_selector).await? {
        StoredCredential::Live(row) => {
            assert_eq!(row.version(), restart_live_commit.version());
            assert_eq!(row.data().as_ref(), b"restart-v2");
        },
        StoredCredential::Tombstoned(_) => {
            panic!("a fresh pool must retain the acknowledged live replacement")
        },
    }
    drop(restarted);
    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn postgres_concurrent_mutations_have_one_linear_winner() -> TestResult<()> {
    let Some(database) = IsolatedDatabase::connect().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return Ok(());
    };
    let store = PgCredentialPersistence::connect_with(database.options.clone()).await?;
    let credential_owner = owner("tenant-concurrency");

    let replace_id = CredentialId::new();
    let replace_selector = selector(&credential_owner, replace_id);
    store
        .create(&replace_selector, create(None, b"replace-before"))
        .await?;
    let replace_barrier = Arc::new(Barrier::new(2));
    let replace_a = {
        let barrier = Arc::clone(&replace_barrier);
        let store = store.clone();
        let selector = replace_selector.clone();
        async move {
            barrier.wait().await;
            store
                .replace(&selector, replacement(version(1), None, b"replace-a"))
                .await
        }
    };
    let replace_b = {
        let barrier = Arc::clone(&replace_barrier);
        let store = store.clone();
        let selector = replace_selector.clone();
        async move {
            barrier.wait().await;
            store
                .replace(&selector, replacement(version(1), None, b"replace-b"))
                .await
        }
    };
    let replace_outcomes: [_; 2] = tokio::join!(replace_a, replace_b).into();
    assert_eq!(
        replace_outcomes
            .iter()
            .filter(|outcome| outcome.is_ok())
            .count(),
        1
    );
    assert_eq!(
        replace_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Err(CredentialPersistenceError::VersionConflict { expected, actual })
                        if *expected == version(1) && *actual == version(2)
                )
            })
            .count(),
        1
    );

    let tombstone_id = CredentialId::new();
    let tombstone_selector = selector(&credential_owner, tombstone_id);
    store
        .create(&tombstone_selector, create(None, b"tombstone-before"))
        .await?;
    let tombstone_barrier = Arc::new(Barrier::new(2));
    let tombstone_a = {
        let barrier = Arc::clone(&tombstone_barrier);
        let store = store.clone();
        let selector = tombstone_selector.clone();
        async move {
            barrier.wait().await;
            store
                .tombstone(&selector, CredentialTombstone::new(version(1)))
                .await
        }
    };
    let tombstone_b = {
        let barrier = Arc::clone(&tombstone_barrier);
        let store = store.clone();
        let selector = tombstone_selector.clone();
        async move {
            barrier.wait().await;
            store
                .tombstone(&selector, CredentialTombstone::new(version(1)))
                .await
        }
    };
    let tombstone_outcomes: [_; 2] = tokio::join!(tombstone_a, tombstone_b).into();
    assert_eq!(
        tombstone_outcomes
            .iter()
            .filter(|outcome| outcome.is_ok())
            .count(),
        1
    );
    assert_eq!(
        tombstone_outcomes
            .iter()
            .filter(|outcome| { matches!(outcome, Err(CredentialPersistenceError::NotFound)) })
            .count(),
        1
    );

    let name_barrier = Arc::new(Barrier::new(2));
    let name_a_id = CredentialId::new();
    let name_b_id = CredentialId::new();
    let name_a = {
        let barrier = Arc::clone(&name_barrier);
        let store = store.clone();
        let selector = selector(&credential_owner, name_a_id);
        async move {
            barrier.wait().await;
            store
                .create(&selector, create(Some("Concurrent Name"), b"name-a"))
                .await
        }
    };
    let name_b = {
        let barrier = Arc::clone(&name_barrier);
        let store = store.clone();
        let selector = selector(&credential_owner, name_b_id);
        async move {
            barrier.wait().await;
            store
                .create(&selector, create(Some("Concurrent Name"), b"name-b"))
                .await
        }
    };
    let name_outcomes: [_; 2] = tokio::join!(name_a, name_b).into();
    assert_eq!(
        name_outcomes
            .iter()
            .filter(|outcome| outcome.is_ok())
            .count(),
        1
    );
    assert_eq!(
        name_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Err(CredentialPersistenceError::AlreadyExists {
                        key: CredentialAlreadyExistsKey::Name,
                    })
                )
            })
            .count(),
        1
    );

    let dual_id = CredentialId::new();
    let dual_selector = selector(&credential_owner, dual_id);
    let dual_barrier = Arc::new(Barrier::new(2));
    let dual_a = {
        let barrier = Arc::clone(&dual_barrier);
        let store = store.clone();
        let selector = dual_selector.clone();
        async move {
            barrier.wait().await;
            store
                .create(&selector, create(Some("Dual Collision"), b"dual-a"))
                .await
        }
    };
    let dual_b = {
        let barrier = Arc::clone(&dual_barrier);
        let store = store.clone();
        let selector = dual_selector;
        async move {
            barrier.wait().await;
            store
                .create(&selector, create(Some("Dual Collision"), b"dual-b"))
                .await
        }
    };
    let dual_outcomes: [_; 2] = tokio::join!(dual_a, dual_b).into();
    assert_eq!(
        dual_outcomes
            .iter()
            .filter(|outcome| outcome.is_ok())
            .count(),
        1
    );
    assert_eq!(
        dual_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Err(CredentialPersistenceError::AlreadyExists {
                        key: CredentialAlreadyExistsKey::Id,
                    })
                )
            })
            .count(),
        1,
        "simultaneous id+name collision must classify as id"
    );

    let shared_id = CredentialId::new();
    let id_barrier = Arc::new(Barrier::new(2));
    let id_a = {
        let barrier = Arc::clone(&id_barrier);
        let store = store.clone();
        let selector = selector(&owner("tenant-concurrency-a"), shared_id);
        async move {
            barrier.wait().await;
            store.create(&selector, create(None, b"id-a")).await
        }
    };
    let id_b = {
        let barrier = Arc::clone(&id_barrier);
        let store = store.clone();
        let selector = selector(&owner("tenant-concurrency-b"), shared_id);
        async move {
            barrier.wait().await;
            store.create(&selector, create(None, b"id-b")).await
        }
    };
    let id_outcomes: [_; 2] = tokio::join!(id_a, id_b).into();
    assert_eq!(
        id_outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
        1
    );
    assert_eq!(
        id_outcomes
            .iter()
            .filter(|outcome| { matches!(outcome, Err(CredentialPersistenceError::NotFound)) })
            .count(),
        1
    );

    drop(store);
    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn postgres_does_not_retry_an_ambiguous_database_failure() -> TestResult<()> {
    let Some(database) = IsolatedDatabase::connect().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return Ok(());
    };
    let store = PgCredentialPersistence::connect_with(database.options.clone()).await?;
    let credential_owner = owner("tenant-retry");
    let credential_id = CredentialId::new();
    let credential_selector = selector(&credential_owner, credential_id);
    store
        .create(&credential_selector, create(None, b"before"))
        .await?;

    sqlx::query("CREATE SEQUENCE credential_update_attempts START WITH 1")
        .execute(&database.pool)
        .await?;
    sqlx::query(
        "CREATE FUNCTION fail_credential_update() RETURNS trigger AS $$
         BEGIN
             PERFORM nextval('credential_update_attempts');
             RAISE EXCEPTION 'injected serialization failure' USING ERRCODE = '40001';
         END;
         $$ LANGUAGE plpgsql",
    )
    .execute(&database.pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER fail_credential_update
         BEFORE UPDATE ON credentials
         FOR EACH ROW EXECUTE FUNCTION fail_credential_update()",
    )
    .execute(&database.pool)
    .await?;

    let error = store
        .replace(
            &credential_selector,
            replacement(version(1), None, b"after"),
        )
        .await
        .expect_err("the injected transaction failure must be reported");
    assert_eq!(error, CredentialPersistenceError::Unavailable);

    let attempts: i64 = sqlx::query_scalar("SELECT last_value FROM credential_update_attempts")
        .fetch_one(&database.pool)
        .await?;
    assert_eq!(
        attempts, 1,
        "the storage adapter must not retry an ambiguous mutation"
    );

    match store.get(&credential_selector).await? {
        StoredCredential::Live(row) => {
            assert_eq!(row.version(), version(1));
            assert_eq!(row.data().as_ref(), b"before");
        },
        StoredCredential::Tombstoned(_) => panic!("rolled-back replace must preserve live state"),
    }

    drop(store);
    database.cleanup().await;
    Ok(())
}
